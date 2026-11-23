"""raw/<scenario>.ndjson -> fixtures/<scenario>.ndjson (sanitized for a public repo).

Usage: python3 sanitize.py [raw file ...]      (default: every file in $CAPTURE_DIR/raw)
Environment: CAPTURE_DIR (folder with raw/, fixtures/, work/; default: the parent of this folder),
             FIXTURES_DIR (where to write; default: $CAPTURE_DIR/fixtures),
             CLI_VERSION (text for the first line of each fixture; default: what `claude --version` prints),
             DOC_NAME (the document the first line points to),
             LEGACY_TIME_MARKS=1 (time markers as in the part 1 fixtures: see the comment in sanitize()).

Fixture format, one JSON object per line:
  {"_note": ..., "_argv": [...]}     first line: what the recording is, and the CLI arguments
  {"_t": 12.3}                        time marker: seconds since the process was started (written before a stdout line
                                      when 0.25 s or more passed since the last time given)
  {"_sent": {...}, "_t": 12.3}        a line the host wrote to the CLI's stdin
  {"_stderr": "..."}                  a line of the CLI's stderr
  {"_meta": {...}}                    a note of the driver (stdin closed, exit code, files found on disk)
  anything else                       a line of the CLI's stdout, as written (after sanitizing)

Sanitizing: user name and temp paths, every id (session, request, message, tool use, task, run, hook, uuid),
thinking signatures, account fields, and the lists that describe the machine (slash commands, skills, plugins,
agents, model catalog, usage report) are replaced or shortened. Key names and structure are kept.
"""
import datetime, getpass, glob, json, os, re, shutil, subprocess, sys

OUT = os.path.abspath(os.environ.get("CAPTURE_DIR") or os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
USER = getpass.getuser()
HOME = os.path.expanduser("~")
DOC_NAME = os.environ.get("DOC_NAME", "the document next to the fixtures")
# MCP servers the scenarios themselves configure. Every other server name is this machine's and is removed.
OWN_MCP = ("exits-at-once", "no-such-command", "needs-login", "ping-project",
           "ping-slow", "ping-flaky", "ping-config", "ping-same", "ping-user")


def cli_version():
    v = os.environ.get("CLI_VERSION")
    if not v:
        exe = os.environ.get("CLAUDE_BIN") or shutil.which("claude") or "claude"
        v = subprocess.run([exe, "--version"], capture_output=True, text=True).stdout.split(" ")[0].strip()
    return v


def work_paths():
    """Every spelling of the work folder: with and without /private, plain and as the CLI writes it in project
    folder names (each character that is not a letter or a digit becomes '-')."""
    w = os.path.join(OUT, "work")
    plain = {w, os.path.realpath(w)}
    for p in list(plain):
        plain.add(p[len("/private"):] if p.startswith("/private/") else "/private" + p)
    plain = sorted(plain, key=len, reverse=True)
    dashed = sorted({re.sub(r"[^A-Za-z0-9]", "-", p) for p in plain}, key=len, reverse=True)
    return plain, dashed


WORK_PLAIN, WORK_DASHED = work_paths()
UUID = r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"
UUID_RE = re.compile(UUID)
ZERO_UUID = "00000000-0000-4000-8000-000000000000"
KEEP_COMMANDS = ("clear", "compact", "context")

class Ids:
    def __init__(self):
        self.map = {}
        self.count = {}

    def get(self, value, prefix):
        if value not in self.map:
            self.count[prefix] = self.count.get(prefix, 0) + 1
            self.map[value] = "%s-%d" % (prefix, self.count[prefix])
        return self.map[value]


def prescan(v, ids, key=None, sent=False):
    if isinstance(v, dict):
        for k, x in v.items():
            prescan(x, ids, k, sent)
    elif isinstance(v, list):
        for x in v:
            prescan(x, ids, key, sent)
    elif isinstance(v, str) and v and v != ZERO_UUID:
        is_uuid = UUID_RE.fullmatch(v) is not None
        if key == "session_id" and is_uuid:
            ids.get(v, "sess")
        elif key == "new_conversation_id" and is_uuid:
            ids.get(v, "conv")
        elif key == "request_id" and is_uuid:
            ids.get(v, "req")
        elif key in ("command_uuid", "message_uuid") and is_uuid:
            ids.get(v, "user-msg")
        elif key == "uuid" and sent and is_uuid:
            ids.get(v, "user-msg")
        elif key in ("task_id", "agent_id", "agentId") and re.fullmatch(r"[a-z0-9]{8,20}", v):
            ids.get(v, "tid")
        elif key in ("run_id", "runId") and re.fullmatch(r"[a-z0-9]+-[a-z0-9]+", v):
            ids.get(v, "run")
        elif key == "hook_id" and is_uuid:
            ids.get(v, "hook")


def keep_commands(cmds):
    out = []
    for c in cmds:
        name = c.get("name") if isinstance(c, dict) else c
        if name in KEEP_COMMANDS:
            out.append(c)
    return out[:2]


def redact_usage(u):
    out = {}
    for k, v in u.items():
        if k == "rate_limits" and isinstance(v, dict):
            out[k] = {"five_hour": v.get("five_hour"), "_redacted": "other keys removed (account data)"}
        elif k == "behaviors":
            out[k] = {"_redacted": "removed (usage breakdown of this machine)"}
        elif k == "subscription_type":
            out[k] = "<plan>"
        else:
            out[k] = v
    return out


def cut_cost_text(s):
    i = s.find("What's contributing to your limits usage")
    if i >= 0:
        s = s[:i] + "[rest removed: usage breakdown of this machine]"
    return re.sub(r"\([A-Z][A-Za-z_]+/[A-Za-z_]+\)", "(Region/City)", s)


def transform(v, ids, sig, key=None):
    """Structural shortening; returns the new value."""
    if isinstance(v, str):
        if key == "signature" and v:
            return sig.get(v, "sig")
        if "contributing to your limits usage" in v or (key in ("text", "result") and "resets " in v and "% used" in v):
            return cut_cost_text(v)
        return v
    if isinstance(v, list):
        return [transform(x, ids, sig, key) for x in v]
    if not isinstance(v, dict):
        return v
    d = dict(v)
    if d.get("type") == "system" and d.get("subtype") == "init":
        d["slash_commands"] = keep_commands(d.get("slash_commands", []))
        d["skills"] = []
        d["plugins"] = []
        d["agents"] = [a for a in d.get("agents", []) if a == "general-purpose"]
        if "terminal_slash_commands" in d:
            d["terminal_slash_commands"] = d["terminal_slash_commands"][:1]
        d["mcp_servers"] = [s for s in d.get("mcp_servers", []) if s.get("name") in OWN_MCP]
        d["tools"] = [t for t in d.get("tools", []) if not t.startswith("mcp__") or t.split("__")[1] in OWN_MCP]
    if d.get("type") == "system" and d.get("subtype") == "commands_changed":
        d["commands"] = keep_commands(d.get("commands", []))
    if "commands" in d and "models" in d and "account" in d:  # initialize response
        d["commands"] = keep_commands(d["commands"])
        d["agents"] = [a for a in d.get("agents", []) if a.get("name") == "general-purpose"]
        d["account"] = {k: (x if k == "apiProvider" else "<%s>" % k) for k, x in d["account"].items()}
        d["available_output_styles"] = ["default"]
        d["pid"] = 12345
    if "models" in d and isinstance(d["models"], list) and d["models"] and isinstance(d["models"][0], dict) and "value" in d["models"][0]:
        d["models"] = [m for m in d["models"] if m.get("value") in ("default", "sonnet", "haiku")]
    if "rate_limits" in d and "behaviors" in d:
        d = redact_usage(d)
    if "gridRows" in d:
        d["gridRows"] = []
    if "sections" in d and isinstance(d["sections"], list):  # get_status
        for s in d["sections"]:
            if isinstance(s, dict):
                s["rows"] = [dict(r, value="<%s>" % r["label"].lower().replace(" ", "-")) if r.get("label") in ("Login method", "Organization", "Email") else r for r in s.get("rows", [])]
    if "remote_control_policy_lock_reason" in d and d["remote_control_policy_lock_reason"]:
        d["remote_control_policy_lock_reason"] = "<text>"
    if "rules" in d and isinstance(d["rules"], list) and "workspaceDirectories" in d:  # list_permission_rules state
        narrowing = [r for r in d["rules"] if r.get("source") == "toolsNarrowing"]
        d["rules"] = narrowing[:2] + [r for r in d["rules"] if r.get("source") != "toolsNarrowing"]
    if "pid" in d and isinstance(d["pid"], int):
        d["pid"] = 12345
    if isinstance(d.get("mcpServers"), list):  # mcp_status: a server's config can hold keys
        d["mcpServers"] = [s for s in d["mcpServers"] if s.get("name") in OWN_MCP]
    if d.get("type") == "base64" and isinstance(d.get("data"), str) and len(d["data"]) > 20000:  # a big image block
        d["data"] = "<base64 removed: %d characters>" % len(d["data"])
    return {k: transform(x, ids, sig, k) for k, x in d.items()}


def scrub(s, ids, name_map):
    u = re.escape(USER)
    for p in WORK_DASHED:
        s = s.replace(p + "-", "-tmp-work-").replace(p, "-tmp-work")
    for p in WORK_PLAIN:
        s = s.replace(p, "/tmp/work")
    s = re.sub(r"(?:/private)?/tmp/claude-\d+/", "/tmp/claude-tmp/", s)
    s = re.sub(r"(?:/private)?/var/folders/[\w+]+/[\w+]+/", "/var/folders/xx/yy/", s)
    s = s.replace(HOME, "/Users/me").replace(re.sub(r"[^A-Za-z0-9]", "-", HOME) + "-", "-Users-me-")
    s = re.sub(u, "me", s, flags=re.I)
    s = re.sub(r"/tmp/cc-socks/\d+\.sock", "/tmp/cc-socks/12345.sock", s)
    s = re.sub(r"[\w.+-]+@[\w-]+\.[A-Za-z][\w.-]*", lambda m: m.group(0) if m.group(0) == "demo@example.invalid" else "user-email", s)
    s = re.sub(r"cf-ray: [0-9a-f]+-[A-Z]{3}", "cf-ray: 0000000000000000-XXX", s)  # the last part names a place
    s = re.sub(r"(@ +\d+ me) +\d{1,2} [A-Z][a-z]{2} \d{2}:\d{2}", r"\1 1 Jan 00:00", s)  # local time of a file listing
    for value in sorted(ids.map, key=len, reverse=True):
        if not UUID_RE.fullmatch(value):
            s = s.replace(value, ids.map[value])
    s = re.sub(r"\bmsg_[A-Za-z0-9]{10,}", lambda m: ids.get(m.group(0), "msg"), s)
    s = re.sub(r"\b(?:srv)?toolu_[A-Za-z0-9]{10,}", lambda m: ids.get(m.group(0), "toolu"), s)
    s = re.sub(r"\breq_[A-Za-z0-9]{10,}", lambda m: ids.get(m.group(0), "apireq"), s)
    s = UUID_RE.sub(lambda m: m.group(0) if m.group(0) == ZERO_UUID else ids.get(m.group(0), "uuid"), s)
    return s


def rechunk_stream_deltas(recs, ids):
    """A path or a name can be split over several streamed deltas. Join the deltas of each content block, scrub the
    whole string, and (only if it changed) cut it back into the same number of deltas."""
    open_blocks = {}

    def close(key):
        parts = open_blocks.pop(key, [])
        if not parts:
            return
        full = "".join(d[f] for d, f in parts)
        new = scrub(full, ids, None)
        if new == full:
            return
        n = len(parts)
        size = len(new) // n
        for i, (d, f) in enumerate(parts):
            d[f] = new[i * size:(i + 1) * size] if i < n - 1 else new[(n - 1) * size:]

    for r in recs:
        m = r["msg"]
        if r["dir"] != "recv" or not isinstance(m, dict) or m.get("type") != "stream_event":
            continue
        ev = m.get("event", {})
        key = (m.get("session_id"), m.get("parent_tool_use_id"), ev.get("index"))
        if ev.get("type") in ("content_block_start", "content_block_stop"):
            close(key)
        elif ev.get("type") == "content_block_delta":
            delta = ev.get("delta", {})
            for f in ("partial_json", "text", "thinking"):
                if isinstance(delta.get(f), str):
                    open_blocks.setdefault(key, []).append((delta, f))
    for key in list(open_blocks):
        close(key)


def dump(o):
    return json.dumps(o, ensure_ascii=False, separators=(",", ":"))


def sanitize(path):
    name = os.path.basename(path)[:-len(".ndjson")]
    recs = [json.loads(l) for l in open(path)]
    ids, sig = Ids(), Ids()
    for r in recs:
        if r["dir"] == "meta" and isinstance(r["msg"], dict) and "argv" in r["msg"]:
            a = r["msg"]["argv"]
            for flag in ("--session-id", "--resume"):
                if flag in a:
                    ids.get(a[a.index(flag) + 1], "sess")
    for r in recs:
        prescan(r["msg"], ids, sent=(r["dir"] == "send"))
    rechunk_stream_deltas(recs, ids)
    out, last_t, last_mark = [], None, None
    for r in recs:
        d, m, t = r["dir"], r["msg"], r["t"]
        if d == "meta" and isinstance(m, dict) and "argv" in m:
            line = {"_note": "claude %s, recorded %s. Lines without a leading _ key are CLI stdout lines. "
                             "Ids, paths and account fields are placeholders; slash_commands, skills, plugins, agents, commands, "
                             "models and usage reports are shortened. See %s." % (VERSION, RECORDED.get(path, TODAY), DOC_NAME),
                    "_argv": ["claude"] + m["argv"][1:], "_cwd": m["cwd"]}
        elif d == "meta":
            line = {"_meta": m, "_t": t}
        elif d == "send":
            line = {"_sent": transform(m, ids, sig), "_t": t}
        elif d == "stderr":
            line = {"_stderr": m, "_t": t}
        else:
            # A time marker before a stdout line. Part 1's rule compared with the previous RECORD, so a long run of
            # stream events got no marker; the rule now compares with the previous MARKER.
            since = last_t if LEGACY_TIME_MARKS else last_mark
            if since is None or t - since >= 0.25:
                out.append(dump({"_t": t}))
                last_mark = t
            line = transform(m, ids, sig)
        if d != "recv":
            last_mark = t  # these lines carry their own _t
        last_t = t
        out.append(scrub(dump(line), ids, None))
    dest = os.path.join(os.environ.get("FIXTURES_DIR") or os.path.join(OUT, "fixtures"), name + ".ndjson")
    with open(dest, "w") as f:
        f.write("\n".join(out) + "\n")
    for l in out:
        json.loads(l)  # every line stays valid JSON
    return dest, len(out)


VERSION = None
LEGACY_TIME_MARKS = bool(os.environ.get("LEGACY_TIME_MARKS"))  # set it to rebuild part 1's fixtures byte for byte
TODAY = datetime.date.today().isoformat()
RECORDED = {}

if __name__ == "__main__":
    VERSION = cli_version()
    paths = sys.argv[1:] or sorted(glob.glob(os.path.join(OUT, "raw", "*.ndjson")))
    RECORDED = {p: datetime.date.fromtimestamp(os.path.getmtime(p)).isoformat() for p in paths}
    for p in paths:
        dest, n = sanitize(p)
        print("%-50s %5d lines" % (os.path.basename(dest), n))
