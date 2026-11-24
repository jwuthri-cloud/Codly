"""Builds the part-6 document from the sanitized r8-* fixtures, so every JSON line quoted there is copied from a fixture.

python3 make_notes_part6.py [output file]      (default: $CAPTURE_DIR/NOTES-part6.md)

Numbers and words that the text states about a fixture are checked against it (`check`): the build fails when a
recording no longer says what the text says, and when a note that lists lines was written late ("Notes written late").
"""
import json
import os
import sys

from notes_lib import OUT, block, check, flags, fx, is_cli, j, kind, meta, results, sub, tl

P = []
w = P.append

FIX = os.environ.get("FIXTURES_DIR") or os.path.join(OUT, "fixtures")
ALL = sorted(f[:-len(".ndjson")] for f in os.listdir(FIX) if f.startswith("r8-"))
for n in ALL:
    check("claude 2.1.311, recorded 2026-11-23." in fx(n)[0]["_note"], "the version and the day of recording of " + n)


def named(prefix):
    return [n for n in ALL if n.startswith(prefix)]


def note(name, key):
    """The one driver note of a fixture that has this key: the value of the key."""
    found = meta(name, key)
    check(len(found) == 1, "one note %r in %s" % (key, name))
    return found[0][key]


def notes(name, key, value=None):
    """Every driver note with this key (and this value of it, when given), whole."""
    return [m for m in meta(name, key) if value is None or m[key] == value]


def whole(name, key):
    """The one driver note of a fixture that has this key, whole."""
    found = meta(name, key)
    check(len(found) == 1, "one note %r in %s" % (key, name))
    return found[0]


def one(name, key, value):
    found = notes(name, key, value)
    check(len(found) == 1, "one note %s=%r in %s" % (key, value, name))
    return found[0]


def is_sent(subtype=None, kind_=None):
    return lambda l: "_sent" in l and (kind_ is None or l["_sent"].get("type") == kind_) and (
        subtype is None or l["_sent"].get("request", {}).get("subtype") == subtype)


def from_note(key):
    return lambda l: "_meta" in l and key in l["_meta"]


def pairs(name, subtype):
    """[(request, answer)] of the host's control requests of that subtype, in order."""
    lines, out = fx(name), []
    for i, l in enumerate(lines):
        if is_sent(subtype)(l):
            rid = l["_sent"]["request_id"]
            ans = next((m for m in lines[i + 1:] if m.get("type") == "control_response" and m["response"].get("request_id") == rid), None)
            out.append((l, ans))
    return out


def qpairs(name, subtype, **kw):
    out = []
    for req, ans in pairs(name, subtype):
        out += [j(req["_sent"], **kw), j(ans, **kw)]
    return out


def both(names):
    return "`%s`" % "`, `".join(names)


def task_or_list(l):
    return sub(l) in ("task_started", "task_updated", "task_notification", "background_tasks_changed")


def has_tool_use(l):
    return l.get("type") == "assistant" and any(b["type"] == "tool_use" for b in l["message"]["content"])


def has_text(l):
    return l.get("type") == "assistant" and any(b["type"] == "text" for b in l["message"]["content"])


def text_of(l):
    return "".join(b.get("text", "") for b in l["message"]["content"] if b["type"] == "text")


def ends(l):
    return kind(l) in ("result", "sent") or sub(l) == "init" or (kind(l) == "meta" and any(k in l["_meta"] for k in ("closing_stdin", "exit_code", "process_exited", "after_the_exit")))


def turn_costs(names):
    """What the recordings spent: the cost of each turn's own model requests (`model_usage_since_last_result`, costUSD),
    summed over every `result` line. A resumed or forked session's `total_cost_usd` carries the earlier cost."""
    total = 0.0
    for n in names:
        for r in results(n):
            total += sum(m.get("costUSD", 0.0) for m in (r.get("model_usage_since_last_result") or {}).values())
    return total


def running_totals(names):
    """The sum, per fixture, of each session's last `total_cost_usd` (what cost.py prints)."""
    total = 0.0
    for n in names:
        last = {}
        for r in results(n):
            last[r.get("session_id")] = r.get("total_cost_usd") or 0.0
        total += sum(last.values())
    return total


SPENT, RUNNING = turn_costs(ALL), running_totals(ALL)
check(SPENT < 2.0 and RUNNING < 2.0, "the session's cost under 2 USD")

# ================================================================= head
N1 = {v: named("r8-untrusted-mcp-%s-" % v) for v in ("enable-all-user", "enable-all-user-local", "enabled-named-user", "enabled-named-user-local",
                                                     "plugin-user", "plugin-user-local", "plugin-repo-user", "plugin-repo-user-local",
                                                     "plugin-repo-local-user", "plugin-repo-local-user-local", "plugin-repo-project-user",
                                                     "plugin-repo-project-user-local")}
# a prefix also matches a longer variant ("plugin-user-" in "plugin-user-local-1"): keep the fixtures whose rest is a label
N1 = {v: [n for n in ns if n[len("r8-untrusted-mcp-%s-" % v):].isdigit()] for v, ns in N1.items()}
ALL1 = sorted(n for ns in N1.values() for n in ns)
PROJECT_STARTS = sum(note(n, "server_starts")["project"] for n in ALL1)
check(PROJECT_STARTS == 0 and all(not any(s["name"] == "ping-project" for s in note(n, "init_mcp_servers")) for n in ALL1), "the repo's server never loaded")
PLUGIN_ON = sorted(n for n in ALL1 if note(n, "server_starts")["plugin"] > 0)
REPO_ON = [n for n in PLUGIN_ON if "plugin-repo" in n]
check(REPO_ON == sorted(N1["plugin-repo-user-local"] + N1["plugin-repo-local-user-local"]), "the repo's local file switches the plugin on, and only it")
w("""# Claude Code CLI stream-json: recorded protocol shapes, part 6

**QUESTION 1 FIRST. UNDER `--setting-sources user,local`, WITHOUT `--strict-mcp-config`, A FILE OF THE REPO
(`.claude/settings.local.json` WITH `"enabledPlugins": {"<plugin>": true}`) SWITCHES ON A PLUGIN THAT THE USER HAD
INSTALLED AND TURNED OFF, AND THE PLUGIN'S MCP SERVER STARTS (%d of %d recordings of that form, %s). UNDER `user` ALONE
IT DOES NOT. THE REPO'S OWN `.mcp.json` SERVER STARTED IN NONE OF THE %d RECORDINGS OF SECTION 1, ALSO WHEN THE USER
SETTINGS FILE SAYS `enableAllProjectMcpServers` OR NAMES THE SERVER IN `enabledMcpjsonServers`.** So the item of package
`claude-part5-truth` (drop `--strict-mcp-config` in a folder you have not trusted when "Use my Claude setup" is on)
holds for `--setting-sources user` and must be taken back for `user,local`: there the repo can turn on code (a
plugin's server; a plugin can also bring hooks, not recorded) that the user had turned off.

- CLI: `claude` 2.1.311 (the same version as parts 1 to 5; checked with `claude --version` before the runs).
- Recorded: 2026-11-23, macOS, `--model haiku` (`claude-haiku-5-5`), subscription login (`"apiKeySource":"none"`).
- Method, base command, fixture line format and sanitizing: as in part 1 ("Base command", "Files", "Sanitizing") with
  the additions of parts 2 to 5. Fixtures of this part are `fixtures/claude-2.1.311/r8-*.ndjson`; the scripts are
  `drivers/claude-stream-json/r8_*.py` (README there says how to record again).
- Cost of the %d recordings at list price: %.4f USD, the sum of each turn's own model requests
  (`model_usage_since_last_result`). `cost.py` prints %.4f: it sums each session's last `total_cost_usd` per
  recording, and a resumed or forked session carries its earlier cost (sections 2 and 3). The runs in a throw-away HOME
  have no login and cost nothing.
- Everything below was observed in these recordings. Each answer says how many runs it rests on (n) and says no more
  than the recordings show. Statements marked "bundle" were read in the CLI's program file (`bgrep.py`), not recorded.
- The sections are numbered as the 12 questions this part was recorded for; the last two tables have them in the same
  order.

Differences from the part 5 method:
- `r5_lib.Procs` now matches the whole command text (`Procs("sleep 41")` no longer matches `sleep 410` or
  `sleep 41.5`), and every script uses it that way. Part 6 scripts give each scenario's `sleep` a duration no other
  script uses.
- Plugins: section 1 installs a plugin of the scenario (`ping-plugin`, from a local marketplace folder) with the CLI's
  own `claude plugin` commands in a throw-away HOME. The sanitizer keeps that plugin (and its server
  `plugin:ping-plugin:ping-plugged`) in `init`, and removes every other plugin.
- New sanitizing: file names the CLI makes from a process id, a hash or the clock (`.claude/sessions/<pid>.<hash>.key`,
  `.claude/sessions/<pid>.json`, `.claude.json.backup.<time>`, `mcp-logs-<server>/<time>.jsonl`) are replaced. The six
  `r7-mcp-toggle-saved-*` fixtures of part 5 were sanitized again with that rule (`sanitize.py --refresh`): only their
  `files_changed` notes changed; nothing was recorded again.
- Sections 2 and 3 count, in the session's own transcript file under `~/.claude/projects/`, its lines and the lines
  that name each secret word, and the session files of its folder (`r8_lib.transcript`: counts only, the transcript
  itself is never copied).
- In quoted lines `"…":"…"` stands for keys left out; `# t=` lines are seconds since the process started. A quoted
  line of a subagent keeps `parent_tool_use_id` and `agent_id`.
""" % (len(REPO_ON), len(N1["plugin-repo-user-local"]) + len(N1["plugin-repo-local-user-local"]), both(REPO_ON), len(ALL1),
       len(ALL), SPENT, RUNNING))

# ================================================================= 1
w("## 1. Your own setup in a folder you have not trusted: the user settings that load project servers, and plugins\n")
w("Question: in a throw-away HOME whose USER settings file says to load the project's servers, with no `--strict-mcp-config` and "
  "`--setting-sources user` or `user,local`: is the repo's `.mcp.json` server started? A plugin installed for the user that brings an "
  "MCP server: is it loaded, and can a file of the repo switch such a plugin on?\n")
w("Fixtures: `r8-untrusted-mcp-<form>-<sources>-<label>`. The work folder's `.mcp.json` defines `ping-project` (one line in `started.txt` per "
  "start). Every run has a THROW-AWAY HOME (no login: its one message ends at once with `Not logged in`; the recordings show what was LOADED "
  "and STARTED). Flags of `plugin-repo-local-user-local-1`: %s. The plugin `ping-plugin` comes from a local marketplace folder "
  "(`claude plugin marketplace add`, then `claude plugin install --scope user`, both with the throw-away HOME); its `.mcp.json` brings "
  "`ping-plugged` (one line in `started-plugin.txt` per start). For the `plugin-repo-*` forms the script then ran "
  "`claude plugin disable --scope user` and wrote `{\"enabledPlugins\":{\"ping-plugin@codly-test-market\":true}}` into the work "
  "folder's `.claude/settings.json` and `.claude/settings.local.json` (`plugin-repo`), only the local file (`plugin-repo-local`) or only "
  "`settings.json` (`plugin-repo-project`).\n" % flags(N1["plugin-repo-local-user-local"][0]))
n = N1["plugin-repo-local-user-local"][0]
w("The set-up note and the `init` line of `%s`:\n" % n)
w(block([j(note(n, "setup"), maxlist=9)] + tl(n, lambda l: sub(l) == "init")))
n = N1["plugin-repo-local-user"][0]
w("The `init` line of `%s` (the same files, `--setting-sources user`):\n" % n)
w(block(tl(n, lambda l: sub(l) == "init")))
FORM_TEXT = {"enable-all": "user settings: `\"enableAllProjectMcpServers\": true`", "enabled-named": "user settings: `\"enabledMcpjsonServers\": [\"ping-project\"]`",
             "plugin": "the plugin installed and on for the user", "plugin-repo": "the plugin off for the user; both repo files turn it on",
             "plugin-repo-local": "the plugin off for the user; the repo's `settings.local.json` turns it on",
             "plugin-repo-project": "the plugin off for the user; the repo's `settings.json` turns it on"}
w("| form | `--setting-sources` | what the HOME and the repo have | recordings | `ping-project` started | `ping-plugged` started | scenario servers in `init` |")
w("|---|---|---|---|---|---|---|")
for v, ns in N1.items():
    form, src = (v[:-len("-user-local")], "user,local") if v.endswith("-user-local") else (v[:-len("-user")], "user")
    check(len(ns) >= 2, "two recordings of " + v)
    check(all(note(m, "setup")["setting_sources"] == src for m in ns), "the sources of " + v)
    starts = [note(m, "server_starts") for m in ns]
    servers = sorted({s["name"] + " (" + s["status"] + ")" for m in ns for s in note(m, "init_mcp_servers")})
    w("| `%s` | `%s` | %s | %d | %s | %s | %s |" % (form, src, FORM_TEXT[form], len(ns), " / ".join(str(s["project"]) for s in starts),
                                                 " / ".join(str(s["plugin"]) for s in starts), ", ".join("`%s`" % x for x in servers) or "none"))
    if form in ("plugin-repo", "plugin-repo-local", "plugin-repo-project"):
        check(all(note(m, "setup")["user_settings_file"] == {"enabledPlugins": {"ping-plugin@codly-test-market": False}} for m in ns), "the plugin off for the user in " + v)
for m in PLUGIN_ON:
    check(note(m, "init_mcp_servers") == [{"name": "plugin:ping-plugin:ping-plugged", "status": "connected", "source": "plugin"}]
          and note(m, "init_mcp_tools") == ["mcp__plugin_ping-plugin_ping-plugged__ping"], "the plugin's server in the init of " + m)
check(sorted(PLUGIN_ON) == sorted(N1["plugin-user"] + N1["plugin-user-local"] + REPO_ON), "where the plugin's server started")
w("""
What the adapter can rely on:
- The repo's `.mcp.json` server was started in NONE of these %d recordings (no start, no `init` entry, no tool): not
  with `"enableAllProjectMcpServers": true` in the user settings file, not with the server named in
  `enabledMcpjsonServers` there, under `user` or `user,local` (n=2 each). With parts 4 and 5 (16 recordings) that is
  every form recorded so far.
- A plugin the user installed and left on is loaded under `user` and under `user,local` (n=2 each): its server starts,
  `init.mcp_servers` names it `plugin:ping-plugin:ping-plugged` with `"source":"plugin"`, its tool is
  `mcp__plugin_ping-plugin_ping-plugged__ping`, and `init.plugins` lists the plugin (its `path` is the marketplace
  folder it came from).
- A plugin the user turned OFF (`enabledPlugins` false in the user settings file) is switched ON by the repo's
  `.claude/settings.local.json` under `user,local` (n=4: the local file alone, and both files). It stays off under
  `user` (n=6) and with only the repo's `.claude/settings.json` under `user,local` (n=2): the project file is not
  read without `project` among the sources.
- Not recorded: a plugin the user never installed (a repo file can also name a marketplace: `extraKnownMarketplaces`);
  the plugin's hooks and skills (only its MCP server was looked at); the same with the real login.
""" % len(ALL1))

# ================================================================= 2
w("## 2. Rewind a conversation: `--resume <id> --resume-session-at <uuid>`\n")
w("Question: does the next turn know only what came before that message? What do `init` and the stream say, which session id follows, is "
  "the old transcript kept, can the dropped part be resumed again? Where does a host get the uuid?\n")
L2 = sorted({n[len("r8-rewind-"):-2] for n in named("r8-rewind-")})
check(len(L2) == 2, "two labels of section 2")
a, b, c, d = ["r8-rewind-%s-%s" % (L2[0], p) for p in "abcd"]
w("Fixtures: `r8-rewind-<label>-a` … `-d`, labels %s. Process a: a new session with `--replay-user-messages`, three turns, each tells one "
  "secret word (`lantern`, `pebble`, `violet`) and asks for `noted`; each user message carries a uuid of the host. Process b: "
  "`--resume <a's session> --resume-session-at <uuid of the LAST assistant line of turn 1>`, one turn: \"List every secret word I asked you "
  "to remember\". Process c: a plain `--resume` of the same session, the same question. Process d: `--resume-session-at <uuid of the last "
  "assistant line of turn 3>`, the same question. Flags of b: %s (the uuid is the placeholder of b's own fixture).\n" % (", ".join(L2), flags(b)))
w("Process a, turn 1: the host's line, the CLI's echo of it, the assistant line and the result (`uuid` and `user_message_uuid` kept):\n")


def with_ids(l):
    if l.get("type") == "assistant":
        return {"type": "assistant", "message": {"content": [{"type": "text", "text": text_of(l)}], "…": "…"}, "uuid": l["uuid"], "…": "…"}
    if l.get("type") == "user":
        return {"type": "user", "message": l["message"], "uuid": l.get("uuid"), "isReplay": l.get("isReplay"), "…": "…"}
    if l.get("type") == "result":
        return {"type": "result", "subtype": l["subtype"], "result": l["result"], "session_id": l["session_id"], "user_message_uuid": l.get("user_message_uuid"), "…": "…"}
    return {"_sent": l["_sent"]}


rows, done = [], False
for l in fx(a):
    if "_sent" in l and l["_sent"].get("type") == "user" or (is_cli(l) and l.get("type") in ("assistant", "user", "result")):
        if l.get("type") == "assistant" and not has_text(l):
            continue
        rows.append(j(with_ids(l)))
        if l.get("type") == "result":
            break
w(block(rows))
w("Process b: the host's line, the `init` line, the answer and the result (`session_id` kept):\n")
w(block([j({"_sent": l["_sent"]}) if "_sent" in l else j({k: l[k] for k in ("type", "subtype", "session_id", "model")} | {"…": "…"}) if sub(l) == "init"
         else j(with_ids(l)) for l in fx(b) if "_sent" in l or (is_cli(l) and (sub(l) == "init" or has_text(l) or l.get("type") == "result"))]))
w("Process d: the stderr line and the result (no `init` line came):\n")
w(block([j({"_stderr": l["_stderr"]}) for l in fx(d) if "_stderr" in l] + [j({k: l[k] for k in ("type", "subtype", "is_error", "num_turns", "errors", "session_id")}) for l in results(d)]))
w("| label | b: words named | c: words named | d: result | transcript lines after a / b / c / d | lines naming `pebble`, `violet` after a / after d |")
w("|---|---|---|---|---|---|")
for lab in L2:
    a, b, c, d = ["r8-rewind-%s-%s" % (lab, p) for p in "abcd"]
    tr = [note(x, "transcript") for x in (a, b, c, d)]
    for t in (one(a, "turn", 1), one(a, "turn", 2), one(a, "turn", 3)):
        check(t["replayed_user_lines"] == [{"uuid_is_the_hosts": True, "isReplay": True}] and t["result_user_message_uuid_is_the_hosts"], "the echo and the result's uuid in " + a)
    nb, nc, nd = note(b, "words_named"), note(c, "words_named"), note(d, "words_named")
    check(nb == ["lantern"] and nc == ["lantern"] and nd == [], "the words of " + lab)
    for x in (b, c):
        m = one(x, "asked", x[-1])
        check(m["init_session_id_is_the_first_session"] and m["result_session_id_is_the_first_session"], "the same session in " + x)
    rd = results(d)
    check(len(rd) == 1 and rd[0]["subtype"] == "error_during_execution" and rd[0]["errors"][0].startswith("No message found with message.uuid of: ")
          and not any(sub(l) == "init" for l in fx(d)) and note(d, "exit_code") == 1, "d's error in " + lab)
    check(all(t["transcript_found"] for t in tr) and tr[0]["lines"] < tr[1]["lines"] < tr[2]["lines"] == tr[3]["lines"], "the transcript grows in " + lab)
    check(tr[0]["lines_that_name"]["pebble"] == tr[3]["lines_that_name"]["pebble"] > 0 and tr[0]["lines_that_name"]["violet"] == tr[3]["lines_that_name"]["violet"] > 0,
          "the dropped turns stay in the file in " + lab)
    check(all(t["session_files_in_its_folder"] == 1 for t in tr), "one session file in " + lab)
    w("| %s | %s | %s | `%s` | %s | %d, %d / %d, %d |" % (lab, ", ".join(nb), ", ".join(nc), rd[0]["subtype"], " / ".join(str(t["lines"]) for t in tr),
                                                      tr[0]["lines_that_name"]["pebble"], tr[0]["lines_that_name"]["violet"],
                                                      tr[3]["lines_that_name"]["pebble"], tr[3]["lines_that_name"]["violet"]))
w("""
What the adapter can rely on (n=2):
- The rewind works: after `--resume-session-at <uuid of turn 1's last assistant line>` the next turn knew only turn 1
  (`lantern`), not the words of turns 2 and 3.
- The session id stays the same (in `init` and in the `result`): a rewind is no new session. The stream says nothing
  about the rewind: the turn's lines are those of any resumed turn.
- The old transcript is kept: it is the same file (one session file in the folder), it grows with each process, and
  its lines that name the dropped words are still there.
- A plain `--resume` after the rewind goes on from the REWOUND branch (c knew only `lantern`).
- The dropped part cannot be resumed by its uuid once a rewound turn was added: `--resume-session-at <uuid of turn 3's
  last assistant line>` gave no `init`, one `result` `error_during_execution` with
  `errors:["No message found with message.uuid of: <uuid>"]` (the same words on stderr), and exit code 1.
- Where the host gets the uuid: the `uuid` key of each assistant line of the stream (the last one of a turn is the
  point to keep). With `--replay-user-messages` the CLI echoes each user line with the host's own `uuid` and
  `"isReplay":true`, and the turn's `result` has `user_message_uuid` = that uuid. Bundle: the help text of the hidden
  flag says `--resume-session-at` takes "any chain-entry UUID, typically the kept turn's last entry", and a second
  hidden flag `--resume-drops-turn <message id>` names the one turn the rewind means to drop, and the CLI then refuses
  it when the dropped part holds anything else (queued messages, task notifications, other turns; not recorded).
- Not recorded: a rewind to a user message's uuid; a rewind in a session with tool calls or helpers.
""")

# ================================================================= 3
w("## 3. Fork a conversation: `--resume <id> --fork-session`\n")
w("Question: the new session id, what the first `result` says about cost (`total_cost_usd`: the old session's or zero), and does the old "
  "session answer as before? Alone and with `--resume-session-at`.\n")
L3 = sorted({n[len("r8-fork-"):-2] for n in named("r8-fork-")})
check(len(L3) == 2, "two labels of section 3")
a, b, c, d = ["r8-fork-%s-%s" % (L3[0], p) for p in "abcd"]
w("Fixtures: `r8-fork-<label>-a` … `-d`, labels %s. Process a: a new session, two turns (`lantern`, `pebble`). Process b: `--resume <a's "
  "session> --fork-session`; c: the same with `--resume-session-at <uuid of turn 1's last assistant line>`; d: a plain `--resume` of a's "
  "session. Each of b, c, d asks for every secret word. Flags of c: %s.\n" % (", ".join(L3), flags(c)))
w("The `init` line and the result of b (first label), then b's note:\n")
w(block([j({k: l[k] for k in ("type", "subtype", "session_id", "model")} | {"…": "…"}) for l in fx(b) if sub(l) == "init"]
        + [j({k: r[k] for k in ("type", "subtype", "result", "session_id", "total_cost_usd")}) for r in results(b)]
        + [j(one(b, "asked", "b"))]))
w("| label | a's last `total_cost_usd` | b: words, new session?, `total_cost_usd` = a + own turn | c: words, new session?, total = a + own turn | d: words, same session?, total = a + own turn | session files after a / b / c / d |")
w("|---|---|---|---|---|---|")
for lab in L3:
    a, b, c, d = ["r8-fork-%s-%s" % (lab, p) for p in "abcd"]
    cells, files = [], [note(a, "transcript")["session_files_in_its_folder"]]
    for x, words, new in ((b, ["lantern", "pebble"], True), (c, ["lantern"], True), (d, ["lantern", "pebble"], False)):
        m = one(x, "asked", x[-1])
        check(m["words_named"] == words, "the words of " + x)
        check(m["init_session_id_is_the_first_session"] == (not new) and m["result_session_id_is_the_first_session"] == (not new), "the session of " + x)
        check(abs(m["total_cost_usd"] - (m["first_sessions_last_total_cost_usd"] + m["cost_of_this_turn"])) < 1e-9, "the cost carried over in " + x)
        sids = {l["session_id"] for l in fx(x) if is_cli(l) and "session_id" in l}
        check(len(sids) == 1, "one session id in the stream of " + x)
        cells.append("%s, %s, %.8f = %.8f + %.8f" % (", ".join(m["words_named"]), "new" if new else "the same", m["total_cost_usd"], m["first_sessions_last_total_cost_usd"], m["cost_of_this_turn"]))
        files.append(note(x, "transcript_of_the_first_session")["session_files_in_its_folder"])
    check(files == [1, 2, 3, 3], "one new session file per fork in " + lab)
    w("| %s | %.8f | %s | %s | %s | %s |" % (lab, one(a, "turn", 2)["total_cost_usd"], cells[0], cells[1], cells[2], " / ".join(map(str, files))))
w("""
What the adapter can rely on (n=2):
- `--fork-session` gives a NEW session id: in `init`, in every line of the stream and in the `result` (one id per
  process). Its transcript is a new file in the same folder (one more session file per fork).
- The fork knows the old conversation (b named both words); with `--resume-session-at` it knows only up to that
  message (c named `lantern` only). So "try again from here" is `--resume <id> --fork-session --resume-session-at
  <uuid>`, and the old session stays as it was.
- COST: the fork's first `total_cost_usd` is the OLD session's total PLUS the fork's own turn (to the last digit), not
  zero. A host that sums `total_cost_usd` per session counts the old part twice; the turn's own cost is in
  `model_usage_since_last_result` (`costUSD`).
- The old session answers as before after the forks: a plain `--resume` (d) keeps its id and knows both words; its
  total goes on from a's.
""")

# ================================================================= 4
w("## 4. Claude Code's own file rewind (`rewind_files`)\n")
w("Question: is there a flag, a setting or a variable that turns it on in stream-json mode? With it on: the answer to a dry run and to a real "
  "one, and which files change.\n")
N4 = {v: named("r8-file-rewind-%s-" % v) for v in ("env", "setting", "off")}
check(all(len(x) == 2 for x in N4.values()), "two recordings per variant of section 4")
w("How it was looked for: `claude --help` lists no switch for it, only `--rewind-files <user-message-id>` (\"Restore files to state at the "
  "specified user message and exit (requires --resume)\"). Bundle: the gate of `rewind_files` (`bgrep.py \"File rewinding is not "
  "enabled\"`, then the function it calls) is: in a run that is not interactive, ONLY the variable `CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING` "
  "turns it on (and `CLAUDE_CODE_DISABLE_FILE_CHECKPOINTING` off); the setting `fileCheckpointingEnabled` (\"Snapshot files before edits so "
  "/rewind can restore them\") counts only in the terminal UI. The recordings test the three ways.\n")
w("Fixtures: `r8-file-rewind-env-*` (the variable `CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING=1`), `-setting-*` "
  "(`--settings '{\"fileCheckpointingEnabled\": true}'`), `-off-*` (neither). Flags of `env-1`: %s. Turn 1 (user message uuid U1) edits "
  "`hello.txt` from `hello` to `hello one`, turn 2 (U2) to `hello two`. Then `rewind_files`: U2 dry run, U2, U1 dry run, U1, a uuid that no "
  "message has (dry run).\n" % flags(N4["env"][0]))
w("The five requests and answers of `%s`:\n" % N4["env"][0])
w(block(qpairs(N4["env"][0], "rewind_files", maxlist=9)))
w("The same requests of `%s` (no variable), the first two:\n" % N4["setting"][0])
w(block(qpairs(N4["setting"][0], "rewind_files")[:4]))
STEPS4 = ("rewind_files U2 dry run", "rewind_files U2", "rewind_files U1 dry run", "rewind_files U1", "rewind_files unknown uuid dry run")
w("| recording | answers (U2 dry, U2, U1 dry, U1, unknown dry) | `hello.txt` after each |")
w("|---|---|---|")


def short(ans):
    r = ans["response"]
    if r["subtype"] == "error":
        return "error: " + r["error"]
    x = r["response"]
    return "canRewind %s%s" % (str(x["canRewind"]).lower(), (", " + x["error"]) if "error" in x else (", filesChanged %d" % len(x["filesChanged"])) if "filesChanged" in x else "")


for v, ns in N4.items():
    for n in ns:
        answers = [short(ans) for _, ans in pairs(n, "rewind_files")]
        texts = [one(n, "after", s)["hello_txt"] for s in STEPS4]
        if v == "env":
            check(answers == ["canRewind true, filesChanged 1", "canRewind true", "canRewind true, filesChanged 1", "canRewind true",
                              "canRewind false, No file checkpoint found for this message."] and texts == ["hello two", "hello one", "hello one", "hello", "hello"], "the rewinds of " + n)
            fl = note(n, "rewind_files_flag")
            check(fl["exit_code"] == 0 and fl["first_line_of_its_output"].startswith("Files rewound to state at message ") and fl["hello_txt"] == "hello one", "the flag in " + n)
        else:
            check(answers == ["canRewind false, File rewinding is not enabled.", "error: File rewinding is not enabled."] * 2 + ["canRewind false, File rewinding is not enabled."]
                  and texts == ["hello two"] * 5, "nothing rewound in " + n)
        w("| `%s` | %s | %s |" % (n, "; ".join(answers), ", ".join("`%s`" % t for t in texts)))
FL = note(N4["env"][0], "rewind_files_flag")
w("""
After the exit of each `env` run: `claude -p --resume <session> --rewind-files <U2>` with the same variable answered
`%s` (exit code %d) and `hello.txt` was `%s` again (n=2).

What the adapter can rely on (n=2 per way):
- Only the VARIABLE turns it on: `CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING=1` in the CLI's environment. The setting
  `fileCheckpointingEnabled` in `--settings` does not, nor nothing (the same answers).
- When it is off, a dry run answers success with `{"canRewind":false,"error":"File rewinding is not enabled."}`, and a
  real one answers a control ERROR with the same words. No file changes.
- When it is on, `rewind_files {"user_message_id": U}` puts the files back to their state BEFORE the turn of U (U2 ->
  `hello one`, U1 -> `hello`). `user_message_id` is the uuid the host put on its user message. A dry run answers
  `{"canRewind":true,"filesChanged":[<absolute paths>],"insertions":N,"deletions":N}` and changes nothing; the real
  one answers `{"canRewind":true,"skippedLinks":0}`. A uuid no message has: `{"canRewind":false,"error":"No file
  checkpoint found for this message."}`.
- So Codly's restore could be the CLI's own: start the agent with the variable, send a uuid with each user message,
  and `rewind_files` that uuid. Not recorded: a file the agent wrote with Bash (the CLI snapshots files before ITS
  edits: Edit here), a file made new in the turn, helpers' edits, and whether the conversation is told.
""" % (FL["first_line_of_its_output"].replace("user-msg-2", "<U2>"), FL["exit_code"], FL["hello_txt"]))

# ================================================================= 5
w("## 5. `stop_task` in the last moment of a turn; `stop_task` on a background command while a turn runs\n")
w("Question: `stop_task` on a background helper during the turn's LAST model request: does a turn start by itself after the `result`? And "
  "`stop_task` on a background COMMAND while a turn runs: the lines, and whether a turn follows.\n")
N5 = {v: named("r8-stop-%s-" % v) for v in ("helper-last-request", "command-last-request", "command-mid-turn")}
check(len(N5["helper-last-request"]) == 2 and len(N5["command-last-request"]) == 2 and len(N5["command-mid-turn"]) >= 2, "the recordings of section 5")
w("Fixtures: `r8-stop-helper-last-request-*` (turn 1 starts a helper in the background, its Bash call `echo begin; sleep 211; echo end`; "
  "turn 2 asks for a poem with no tool, one model request; `stop_task` with the helper's id at turn 2's first streamed text delta), "
  "`r8-stop-command-last-request-*` (turn 1 starts `echo bg; sleep 227` in the background; the same turn 2 and moment), "
  "`r8-stop-command-mid-turn-*` (turn 1 starts `echo bg; sleep 223` in the background; turn 2 runs `echo go; sleep 22.5` in the foreground; "
  "`stop_task` with the command's id 2 s into that call). Flags: %s. The host reads 25 s after turn 2's result.\n" % flags(N5["helper-last-request"][0]))
n = N5["helper-last-request"][0]
w("From the note before `stop_task` to the end (`%s`):\n" % n)
w(block(tl(n, lambda l: task_or_list(l) or has_text(l) or ends(l) or l.get("type") == "control_response" or (l.get("type") == "user" and is_cli(l))
           or (kind(l) == "meta" and ("before_stop_task" in l["_meta"] or "transcript" in l["_meta"])), start=from_note("before_stop_task"))))
n = N5["command-last-request"][0]
w("The same moment with a background command (`%s`):\n" % n)
w(block(tl(n, lambda l: task_or_list(l) or has_text(l) or ends(l) or l.get("type") == "control_response"
           or (kind(l) == "meta" and ("after_stop_task" in l["_meta"] or "transcript" in l["_meta"])), start=from_note("before_stop_task"))))
w("| recording | lines from `stop_task` on | results after it (origin of the 2nd) | `init` after it (s) | queue (enqueued / handed to a running turn / removed) |")
w("|---|---|---|---|---|")
SELF5 = {}
for v, ns in N5.items():
    for n in ns:
        a5, s5, q = note(n, "after_stop_task"), note(n, "seconds_from_stop_task"), note(n, "transcript")
        check(a5["lines"][:4] == ['system/task_updated "killed"', 'system/task_notification "stopped"', "system/background_tasks_changed 0 listed", "control_response success"], "the stop's lines in " + n)
        check(a5["bg_sleep_alive"] is False and a5["list"] == [] and q["transcript_found"], "the stop in " + n)
        SELF5[n] = a5["results_since_the_request"] == 2
        origin = a5["result_origins"][1]["kind"] if SELF5[n] else "-"
        w("| `%s` | %s | %d (%s) | %s | %d / %d / %d |" % (n, ", ".join("`%s`" % k for k in a5["lines"]), a5["results_since_the_request"], origin,
                                                         "%.2f" % s5["init_after_it"] if s5["init_after_it"] is not None else "none",
                                                         q["enqueued"], q["attached_to_a_running_turn"], q["removed"]))
for n in N5["helper-last-request"]:
    a5, q = note(n, "after_stop_task"), note(n, "transcript")
    check(SELF5[n] and a5["result_origins"][1]["kind"] == "task-notification" and (q["enqueued"], q["attached_to_a_running_turn"]) == (1, 0), "the turn by itself in " + n)
for n in N5["command-last-request"]:
    q = note(n, "transcript")
    check(not SELF5[n] and (q["enqueued"], q["attached_to_a_running_turn"], q["removed"]) == (1, 0, 0), "no turn after a command's stop in " + n)
MID5 = {n: [l for l in fx(n) if sub(l) == "task_notification" and l["tool_use_id"] != next(x for x in fx(n) if sub(x) == "task_started" and x.get("is_backgrounded"))["tool_use_id"]]
        for n in N5["command-mid-turn"]}
for n in N5["command-mid-turn"]:
    q = note(n, "transcript")
    check(not SELF5[n] and (q["enqueued"], q["attached_to_a_running_turn"], q["removed"]) == (1, 1, 1), "the command's notification handed to the turn in " + n)
FAILED5 = [n for n, ls in MID5.items() if [l["status"] for l in ls] == ["failed"]]
check(len(FAILED5) == 1 and all([l["status"] for l in MID5[n]] == ["completed"] for n in MID5 if n not in FAILED5), "the turn's own call in the mid-turn runs")
FAILED5_CALL = MID5[FAILED5[0]][0]["tool_use_id"]
FAILED5_RESULT = next(b["content"] for l in fx(FAILED5[0]) if l.get("type") == "user" and is_cli(l) and isinstance(l["message"]["content"], list)
                      for b in l["message"]["content"] if isinstance(b, dict) and b.get("type") == "tool_result" and b.get("tool_use_id") == FAILED5_CALL)


def stamp_of(name, pred):
    """The time of the first line of a fixture that matches (the last time marker before it, or its own `_t`)."""
    t = 0.0
    for l in fx(name):
        if "_t" in l:
            t = l["_t"]
        if pred(l):
            return t
    return None


FAILED5_SECS = (stamp_of(FAILED5[0], lambda l: sub(l) == "task_notification" and l["tool_use_id"] == FAILED5_CALL)
                - stamp_of(FAILED5[0], lambda l: has_tool_use(l) and any(b.get("id") == FAILED5_CALL for b in l["message"]["content"])))
check(FAILED5_RESULT.startswith("Exit code 144") and FAILED5_SECS < 10, "the call that ended early in " + FAILED5[0])
w("""
What the adapter can rely on:
- A helper stopped during the turn's LAST model request (n=2): the stop itself as always (`killed`, `stopped`, the
  list `[]`, the answer `{}`, the helper's two `user` lines), the running turn ends normally, and then the CLI STARTS A
  TURN BY ITSELF: an `init` %s s after the request, and a second `result` with
  `"origin":{"kind":"task-notification",…}`. The transcript shows the notification put in the queue and handed to no
  running turn. So part 5's "partly" is settled: a stop with no later model request of the running turn is told in a
  turn of its own after the result; a stop while a tool call of the turn runs is told inside the turn (part 5,
  section 11).
- A background COMMAND stopped while a turn runs: the same four lines (`killed`, `stopped`, the list `[]`, the answer
  `{}`), and NO turn by itself, both during the turn's last model request (n=2) and during its tool call (n=%d). Its
  notification is put in the queue too: during a tool call it was handed to the turn's next model request (and taken
  out of the queue); during the last request it stayed in the queue until the exit (enqueued, not handed, not
  removed) and started nothing in 25 s.
- In 1 of the %d `command-mid-turn` runs (`%s`) the turn's own foreground call (`echo go; sleep 22.5`) ended %.2f s
  after its `tool_use` line, not after 22.5 s, with the tool result `%s` and its `task_notification` `failed`; in the
  other runs it ran to its end (`completed`). The cause was not found (no script of this part signals a `sleep 22.5`);
  it is not taken as a rule.
""" % (", ".join("%.2f" % note(n, "seconds_from_stop_task")["init_after_it"] for n in N5["helper-last-request"]), len(N5["command-mid-turn"]),
       len(N5["command-mid-turn"]), FAILED5[0], FAILED5_SECS, FAILED5_RESULT.replace("\n", "\\n")))

# ================================================================= 6
w("## 6. After a budget stop: the call that runs on, and a helper with requests left\n")
w("Question: with a long Bash call still running when `error_max_budget_usd` ends the turn: does an `interrupt` end that call (stdin still "
  "open)? Does `stop_task` with the call's task id? And a helper that still has model requests to make when the limit is passed: does it go "
  "on, what does it cost?\n")


def tries(variant):
    """{label: [fixtures]} of a variant, calib first; the last try of a label is the one that passed (the script stops there)."""
    out = {}
    for n in named("r8-after-budget-%s-" % variant):
        lab = n[len("r8-after-budget-%s-" % variant):].split("-")[0]
        out.setdefault(lab, []).append(n)
    return {lab: sorted(ns, key=lambda x: (not x.endswith("-calib"), x)) for lab, ns in out.items()}


T6 = {v: tries(v) for v in ("interrupt", "stop-task", "helper-goes-on")}
check(all(len(t) == 2 for t in T6.values()), "two labels per variant of section 6")
PASS6 = {v: [ns[-1] for ns in t.values()] for v, t in T6.items()}
w("Fixtures: `r8-after-budget-<variant>-<label>-calib`, `-t1` …. As in part 5 (section 10): run `calib` has no budget; each try gets "
  "`--max-budget-usd` = the cost when the Bash call began + a share of the helper's cost, both from the run before; the script stops at the "
  "first try that stops as the scenario needs. Tries that passed: %s. Flags of `%s`: %s.\n" % (both(sum(PASS6.values(), [])), PASS6["interrupt"][0], flags(PASS6["interrupt"][0])))
w("- `interrupt`: a helper writes a story of about 600 words with no tool; the main thread runs `echo go; sleep 53.5`. 2 s after the budget "
  "stop the host sends `interrupt` and reads 5 s, then closes stdin.\n- `stop-task`: the same (`sleep 57.5`); the host sends `stop_task` with the "
  "task id of the call (its `task_started` line).\n- `helper-goes-on`: the helper runs four Bash calls one after the other (`echo h1` … `echo h4`); "
  "the main thread runs `echo go; sleep 47.5`; the host reads 40 s after the stop.\n")
n = PASS6["interrupt"][0]
w("`%s`, from the try's note to the end:\n" % n)
w(block(tl(n, lambda l: task_or_list(l) or ends(l) or l.get("type") == "control_response" or (kind(l) == "meta" and ("after_the_request" in l["_meta"])),
           start=from_note("run"))))
n = PASS6["stop-task"][0]
w("`%s`, from the try's note to the end:\n" % n)
w(block(tl(n, lambda l: task_or_list(l) or ends(l) or l.get("type") == "control_response" or (kind(l) == "meta" and ("after_the_request" in l["_meta"])),
           start=from_note("run"))))
w("| try | budget (USD) | `result` (s) | the call's `sleep` alive at the result | the request | lines in the 5 s after it | `sleep` alive 5 s after | after stdin close |")
w("|---|---|---|---|---|---|---|---|")
for v in ("interrupt", "stop-task"):
    for n in PASS6[v]:
        r, ar, ae = whole(n, "run"), note(n, "after_the_request"), note(n, "after_the_exit")
        check(r["result_subtype"] == "error_max_budget_usd" and r["sleep_alive_at_the_result"] and r["t_bash_tool_result"] is None, "the stop with the call running in " + n)
        if v == "interrupt":
            check(ar["lines"] == ["control_response success"] and ar["sleep_alive"] and ae["lines_after_stdin_was_closed"] == ['system/task_updated "killed"', 'system/task_notification "stopped"']
                  and not ae["sleep_alive"], "the interrupt leaves the call in " + n)
            check(pairs(n, "interrupt")[0][1]["response"]["response"] == {"still_queued": []}, "the interrupt's answer in " + n)
        else:
            check(ar["lines"] == ['system/task_updated "killed"', 'system/task_notification "stopped"', "control_response success"] and not ar["sleep_alive"]
                  and ae["lines_after_stdin_was_closed"] == [], "stop_task ends the call in " + n)
        w("| `%s` | %.6f | %.2f | %s | `%s` | %s | %s | %s |" % (n, r["budget_usd"], r["t_result"], "yes" if r["sleep_alive_at_the_result"] else "no", v.replace("-", "_"),
                                                         ", ".join("`%s`" % k for k in ar["lines"]), "yes" if ar["sleep_alive"] else "no",
                                                         ", ".join("`%s`" % k for k in ae["lines_after_stdin_was_closed"]) or "nothing"))
HELPER6 = {}
for n in PASS6["helper-goes-on"]:
    w("\n`%s`: the helper's lines, the results and stderr:\n" % n)
    w(block(tl(n, lambda l: (l.get("type") == "assistant" and l.get("parent_tool_use_id") and (has_tool_use(l) or has_text(l))) or task_or_list(l) or "_stderr" in l
               or (l.get("type") == "user" and is_cli(l) and l.get("parent_tool_use_id") and not any(isinstance(b, dict) and b.get("type") == "tool_result" for b in l["message"]["content"]))
               or kind(l) == "result" or (kind(l) == "meta" and ("forty_s_after_the_stop" in l["_meta"] or "after_the_exit" in l["_meta"])), text=320)))
w("| try | budget (USD) | the helper's Bash calls before the stop | after it | the helper's end | stderr | results (subtype) | `total_cost_usd` at the end |")
w("|---|---|---|---|---|---|---|---|")
for n in PASS6["helper-goes-on"]:
    r, f = whole(n, "run"), note(n, "forty_s_after_the_stop")
    rs = results(n)
    calls = r["helper_bash_calls_at_the_result"]
    hid = next(x for x in fx(n) if sub(x) == "task_started" and x.get("task_type") == "local_agent")["task_id"]
    hend = [l for l in fx(n) if sub(l) == "task_notification" and l["task_id"] == hid]
    err = [l["_stderr"] for l in fx(n) if "_stderr" in l]
    check(r["result_subtype"] == "error_max_budget_usd" and len(calls) < 4 and f["helper_bash_calls_after_the_stop"] == [] and f["results"] == [], "the helper stops with the turn in " + n)
    check([x["subtype"] for x in rs] == ["error_max_budget_usd"] * len(rs) and len({x["total_cost_usd"] for x in rs}) == 1, "no cost after the stop in " + n)
    check(len(hend) == 1 and fx(n).index(hend[0]) < next(i for i, l in enumerate(fx(n)) if from_note("run")(l)), "the helper ends by the stop in " + n)
    HELPER6[n] = (hend[0]["status"], err)
    w("| `%s` | %.6f | %s | none | `%s` | %s | %s | %.6f |" % (n, r["budget_usd"], ", ".join("`%s`" % x for x in calls), hend[0]["status"], " ".join("`%s`" % e for e in err) or "none",
                                                          ", ".join("`%s`" % x["subtype"] for x in rs), rs[-1]["total_cost_usd"]))
ENDS6 = sorted(HELPER6.values(), key=lambda x: x[0])
check([e[0] for e in ENDS6] == ["completed", "stopped"] and ENDS6[0][1] == [] and len(ENDS6[1][1]) == 1
      and ENDS6[1][1][0].startswith("Budget limit reached (") and ENDS6[1][1][0].endswith("); stopping background agents."), "the two ends of the helper")
w("""
What the adapter can rely on (n=2 each):
- An `interrupt` after the budget stop does NOT end the call that runs on: the answer `{"still_queued":[]}` and nothing
  else; the `sleep` was still alive 5 s later. Only the close of stdin ended it (`killed`, `stopped`), as in part 5.
- `stop_task` with the call's task id (the `task_id` of its `task_started` line, which a foreground call gets after
  about 3 s) ends it at once: `task_updated` `killed`, `task_notification` `stopped`, the answer `{}`; the `sleep` was
  gone. So after a budget stop the host can end such a call without ending the process: `stop_task` for every
  foreground call that has a `task_started` and no end yet.
- A helper that still had calls to make when the limit passed made NO model request after the turn's stop (n=2): it
  had made two of its four Bash calls, none came in the 40 s after the stop, and `total_cost_usd` did not grow. It
  ended in one of two ways (n=1 each): the CLI STOPPED it at the turn's stop (stderr `%s`, then `task_updated`
  `killed`, `task_notification` `stopped`, the helper's line `[Request interrupted by user]`); or its last request
  came back first, with a text that says it stopped because the budget was used up, and it ended `completed` just
  before the turn's `result`. The request that passed the limit was paid: the total ended above the budget.
- So `run_limit_turn_end` (ending the run after a budget stop) is not needed to stop spending: the CLI spends nothing
  more and stops its helpers. It is needed for the call that runs on, unless the host sends `stop_task` for it.
""" % ENDS6[1][1][0])

# ================================================================= 7
w("## 7. A foreground Bash call that becomes a task and FAILS, and one cut by an `interrupt`\n")
w("Question: the status word and the keys of its `task_notification`.\n")
N7 = named("r8-bash-task-fails-")
check(len(N7) == 2, "two recordings of section 7")
w("Fixtures: %s. Flags: %s. Turn 1: `echo q7; sleep 4; exit 3` in the foreground. Turn 2: `echo q7b; sleep 37` in the foreground; 1 s "
  "after its `task_started` and its `sleep`, the host sends `interrupt`.\n" % (both(N7), flags(N7[0])))
w("The task lines and the tool results of both turns (`%s`):\n" % N7[0])
w(block(tl(N7[0], lambda l: task_or_list(l) or (l.get("type") == "user" and is_cli(l) and not l.get("isReplay")) or kind(l) == "result" or is_sent("interrupt")(l), brief=False)))
for n in N7:
    f, i = one(n, "turn", "fails"), one(n, "turn", "interrupted")
    check([x["subtype"] for x in f["task_lines"]] == ["task_started", "task_notification"] and f["task_lines"][1]["status"] == "failed"
          and f["task_lines"][1]["output_file"] == "" and f["tool_results"] == [{"is_error": True, "content": "Exit code 3\nq7"}], "the failed call in " + n)
    check([x["subtype"] for x in i["task_lines"]] == ["task_started", "task_notification"] and i["task_lines"][1]["status"] == "stopped"
          and i["task_lines"][1]["output_file"] == "" and i["lines_since_the_interrupt"][:2] == ["control_response success", 'system/task_notification "stopped"']
          and i["result_subtype"] == "error_during_execution" and not i["sleep_37_alive"], "the cut call in " + n)
    for t in (f, i):
        check(sorted(t["task_lines"][1]) == sorted(["type", "subtype", "task_id", "run_id", "tool_use_id", "status", "output_file", "summary"]), "the keys in " + n)
w("""
What the adapter can rely on (n=2 each):
- A call that ends by itself with an exit code other than 0: ONE `task_notification` with `"status":"failed"`,
  `"output_file":""`, the call's `tool_use_id` and its description in `summary`, just before the tool result
  (`is_error:true`, `Exit code 3` and the output). No `task_updated`, no list line.
- A call cut by an `interrupt`: ONE `task_notification` with `"status":"stopped"` and the same keys, right after the
  interrupt's answer, then the rejected tool result, `[Request interrupted by user for tool use]` and the turn's
  `result` `error_during_execution`. The `sleep` was gone.
- The keys are always `task_id`, `run_id`, `tool_use_id`, `status`, `output_file`, `summary` (and `uuid`, `session_id`).
  The status words of such a line seen so far: `completed`, `failed`, `stopped`. Each belongs to a Bash call (its
  `tool_use_id`): it must end no helper row and no background row.
""")

# ================================================================= 8
w("## 8. An MCP switch while a turn runs; a server that cannot start, turned off and on in a NEW process\n")
w("Question: `mcp_toggle` off and on during a turn that is about to call that server's tool. And a server that cannot start at the process's "
  "start, then off and on in a new process (the saved switch).\n")
w("NOT RECORDED: the switch while a turn runs. A switch is saved in `~/.claude.json` (part 5, section 1), so the rules of these recordings "
  "make every run that switches a server use a throw-away HOME, and such a HOME has no login: no turn can run there. A run with the real HOME "
  "would write the user's own `~/.claude.json`. Bundle, not recorded: the CLI also reads `CLAUDE_CONFIG_DIR` for its config folder; a "
  "throw-away config folder with the real login could make this recordable, but that was not tried (it touches how the login is found).\n")
L8 = sorted({n[len("r8-mcp-cannot-start-saved-"):-2] for n in named("r8-mcp-cannot-start-saved-")})
check(len(L8) == 2, "two labels of section 8")
w("Fixtures: `r8-mcp-cannot-start-saved-<label>-a`, `-b`, `-c`, labels %s. A THROW-AWAY HOME (no login, no model request). `ping-flaky` "
  "(exits at once with code 1 while the file `switch.on` is missing; one line in `started.txt` per start) comes with `--mcp-config`. Flags: "
  "%s. Process a: `switch.on` missing; `mcp_status`, `mcp_toggle` off, `mcp_status`. Process b, a new process in the same folder: "
  "`mcp_status`, `mcp_toggle` on (still missing), `mcp_status`, then `switch.on` is made, `mcp_reconnect`, `mcp_status`. Process c, a new "
  "process (`switch.on` there): `mcp_status`.\n" % (", ".join(L8), flags("r8-mcp-cannot-start-saved-%s-a" % L8[0])))
a8 = "r8-mcp-cannot-start-saved-%s-" % L8[0]
w("The requests and answers of b, first label:\n")
w(block(qpairs(a8 + "b", "mcp_status")[:2] + qpairs(a8 + "b", "mcp_toggle") + qpairs(a8 + "b", "mcp_status")[2:4] + qpairs(a8 + "b", "mcp_reconnect")))
w("| process, step | answer | `ping-flaky` after it | saved `disabledMcpServers` | starts so far |")
w("|---|---|---|---|---|")


def status8(ans):
    x = ans["response"]["response"]["mcpServers"][0]
    return x["status"] + (" (`%s`)" % x["error"] if x.get("error") else "")


ROWS8 = {}
for lab in L8:
    rows = []
    for p in "abc":
        n = "r8-mcp-cannot-start-saved-%s-%s" % (lab, p)
        st = [ans for _, ans in pairs(n, "mcp_status")]
        steps = notes(n, "after")
        acts = [ans for sub_ in ("mcp_toggle", "mcp_reconnect") for _, ans in pairs(n, sub_)]
        for k, s8 in enumerate(steps):
            ans = "" if k == 0 else ("success" if acts[k - 1]["response"]["subtype"] == "success" else "error: " + acts[k - 1]["response"]["error"])
            rows.append((p + ", " + s8["after"], ans, status8(st[k]), s8["saved_disabledMcpServers"], s8["server_starts_so_far"]))
    ROWS8[lab] = rows
check(ROWS8[L8[0]] == ROWS8[L8[1]], "the two labels of section 8 agree")
for r in ROWS8[L8[0]]:
    w("| %s | %s | %s | `%s` | %d |" % (r[0], r[1] and "`%s`" % r[1], r[2], json.dumps(r[3]), r[4]))
R8 = ROWS8[L8[0]]
check([r[2] for r in R8] == ["failed (`Connection closed`)", "disabled", "disabled", "failed (`Connection closed`)", "connected", "connected"]
      and [r[1] for r in R8][2:5] == ["", "error: Connection closed", "success"] and R8[2][4] == R8[1][4], "the steps of section 8")
w("""
What the adapter can rely on (n=2, the two labels agree step for step):
- A server that cannot start at the process start is `failed` (`Connection closed`); the CLI started it twice
  (two lines in `started.txt`). `mcp_toggle` off makes it `disabled` and saves the switch.
- A NEW process in the folder starts it `disabled` and does not start it (no new line).
- `mcp_toggle` on there while it still cannot start: a control ERROR `Connection closed`, status `failed` with the same
  words, and the saved switch is ON again (`[]`), as for a failed "Turn on" in the same process (part 5, section 9).
- Once the cause is gone, `mcp_reconnect` answers success (`connected`), and the next new process starts it
  `connected`.
""")

# ================================================================= 9
w("## 9. The request's own mode suggestion on a BASH card\n")
w("Question: an allow answer to a Bash request that carries several suggestions: with only its `setMode` sent back; with all of them; and a "
  "`setMode` to another mode than `acceptEdits`.\n")
V9 = ("only-set-mode", "all", "plan", "dont-ask")
N9 = {v: named("r8-bash-set-mode-%s-" % v) for v in V9}
check(all(len(x) == 2 for x in N9.values()), "two recordings per variant of section 9")
w("Fixtures: `r8-bash-set-mode-<variant>-<label>`. Flags: %s. Turn 1: Bash `touch r8-one.txt`; its request gets the variant's allow "
  "(`only-set-mode`: the request's `setMode` item only; `all`: every suggestion of the request; `plan` / `dont-ask`: the `setMode` item "
  "with `mode` changed to `plan` / `dontAsk`). Every later request gets a plain allow. Turn 2: Bash `touch r8-two.txt`. Turn 3: an Edit of "
  "`hello.txt`.\n" % flags(N9["all"][0]))
n = N9["all"][0]
w("The request and the answer of `%s`, and the status line after it:\n" % n)
w(block([l for l in tl(n, lambda l: l.get("type") == "control_request" or ("_sent" in l and l["_sent"].get("type") == "control_response")
                       or (sub(l) == "status" and "permissionMode" in l)) if not l.startswith("# t=")]))
w("| recording | suggestions of the request | sent back | status lines | `init` mode of turns 2, 3 | requests in turns 2, 3 | `r8-two.txt` made | `hello.txt` at the end | `.claude/settings.local.json` |")
w("|---|---|---|---|---|---|---|---|---|")
for v, ns in N9.items():
    for n in ns:
        t1, t2, t3 = one(n, "turn", "1"), one(n, "turn", "2"), one(n, "turn", "3")
        sent9 = note(n, "updated_permissions_sent")[0]
        mode = {"only-set-mode": "acceptEdits", "all": "acceptEdits", "plan": "plan", "dont-ask": "dontAsk"}[v]
        check(t1["requests"] == [["Bash", ["addRules", "addDirectories", "setMode:acceptEdits"]]] and t1["init_permissionMode"] == "default", "turn 1 of " + n)
        check(t1["status_lines_with_permissionMode"] == [mode] and t2["init_permissionMode"] == mode and t3["init_permissionMode"] == mode, "the mode of " + n)
        check(t2["requests"] == [] and t3["requests"] == [] and t1["files"]["r8-one.txt"], "no more requests in " + n)
        made = t3["files"]["r8-two.txt"]
        check(made == (mode == "acceptEdits") and (t3["hello_txt"] == "hello three") == (mode == "acceptEdits"), "what the later turns did in " + n)
        slj = note(n, "settings_local_json")
        check((slj is not None) == (v == "all"), "the local settings file of " + n)
        w("| `%s` | %s | %s | %s | `%s`, `%s` | %d, %d | %s | `%s` | %s |" % (n, ", ".join("`%s`" % x for x in t1["requests"][0][1]), ", ".join("`%s`" % (x["type"] + (":" + x["mode"] if "mode" in x else "")) for x in sent9),
                                                                    ", ".join("`%s`" % x for x in t1["status_lines_with_permissionMode"]), t2["init_permissionMode"], t3["init_permissionMode"],
                                                                    len(t2["requests"]), len(t3["requests"]), "yes" if made else "no", t3["hello_txt"],
                                                                    "`%s`" % json.dumps(slj, separators=(",", ":")) if slj else "none"))
PLAN_MODELS = sorted({l["message"]["model"] for n in N9["plan"] for l in fx(n) if l.get("type") == "assistant"})
w("""
What the adapter can rely on (n=2 per variant):
- On a Bash card too, the request's own `setMode` item ALONE in `updatedPermissions` switches the session to
  `acceptEdits`: the status line `{"type":"system","subtype":"status","status":null,"permissionMode":"acceptEdits"}`
  at once, the next turns' `init` say `acceptEdits`, and the next `touch` and the next Edit ran with no request. So one
  answer for "approve and accept edits" works on any card that carries a `setMode` suggestion: send that item only.
- Sending ALL suggestions does the same and more: the `addRules` item (`destination` `localSettings`) WRITES the work
  folder's `.claude/settings.local.json` with an allow rule for that exact command. Do not send all of them back.
- A `setMode` with another mode is obeyed: `plan` and `dontAsk` switched the session (status line, next `init`). In
  both, the later turns asked nothing and changed nothing (`r8-two.txt` not made, `hello.txt` unchanged): in `plan` the
  agent plans only, in `dontAsk` every tool use that is not allowed already is refused without a request. (The `plan`
  runs cost more: in plan mode the CLI sent some requests to another model; models seen in them: %s.)
""" % ", ".join("`%s`" % m for m in PLAN_MODELS))

# ================================================================= 10
w("## 10. A slow tool of an HTTP MCP server: the other server kinds and the settings\n")
w("Question: a tool that takes 70 s on an HTTP server that streams its answer (an event stream) and on one that sends progress notifications: "
  "does the call still end at 60 s with \"The operation timed out.\"? Does a setting or a variable the CLI documents move that limit?\n")
V10 = ("sse", "progress", "json-env", "json-timeout")
N10 = {v: named("r8-http-slow-%s-" % v) for v in V10}
check(all(len(x) == 2 for x in N10.values()), "two recordings per variant of section 10")
w("Fixtures: `r8-http-slow-<variant>-<label>`. The server `ping-http` runs in the driver (Streamable HTTP on 127.0.0.1) and comes with "
  "`--mcp-config` as type `http`; its tool waits 70 s. `sse`: `tools/call` is answered with an event stream (headers at once, the result as "
  "one event at 70 s). `progress`: the same stream with a `notifications/progress` event every 10 s. `json-env`: the JSON answer of part 5 "
  "(nothing until the tool is done) with `MCP_TOOL_TIMEOUT=120000` in the CLI's environment. `json-timeout`: the JSON answer with "
  "`\"timeout\": 120000` in the server's entry of the config file. Flags of `sse-1`: %s. Bundle: an HTTP request of an MCP client gets a limit "
  "of 60 s, or the server's `timeout` / `MCP_TOOL_TIMEOUT` when one is set and larger; it is cleared when the answer's headers come.\n" % flags(N10["sse"][0]))
n = N10["progress"][0]
w("The call of `%s`, its `tool_progress` lines and its result:\n" % n)
w(block(tl(n, lambda l: l.get("type") == "tool_progress" or has_tool_use(l) or (l.get("type") == "user" and is_cli(l)), brief=True)))
w("| recording | headers sent (s after the call came) | progress events sent | `progressToken` in the call | `tool_progress` lines (s) | `tool_use` -> tool result (s) | result |")
w("|---|---|---|---|---|---|---|")
for v, ns in N10.items():
    for n in ns:
        m = meta(n, "server_requests")[0]
        call = [h for h in m["server_requests"] if h.get("jsonrpc_method") == "tools/call"]
        check(len(call) == 1 and call[0].get("answered_at") is not None and "answer_failed" not in call[0], "the server answered the one call of " + n)
        res = m["tool_results"]
        check(res == [{"is_error": False, "content": '[{"type": "text", "text": "pong after 70 s"}]'}], "the call returned in " + n)
        check([p["elapsed_time_seconds"] for p in m["tool_progress_lines"]] == [30, 60] and all(p["heartbeat"] for p in m["tool_progress_lines"]), "only heartbeats in " + n)
        check(70 < m["seconds_from_tool_use_to_tool_result"] < 75, "the call's time in " + n)
        if v == "progress":
            check(call[0]["progress_token_in_request"] and call[0]["progress_sent"] == 6, "the progress events of " + n)
        w("| `%s` | %s | %s | %s | %s | %.3f | `pong after 70 s` |" % (n, "%.3f" % (call[0]["headers_sent_at"] - call[0]["at"]) if "headers_sent_at" in call[0] else "at the end",
                                                               call[0].get("progress_sent", 0), "yes" if call[0]["progress_token_in_request"] else "no",
                                                               ", ".join("%.2f" % x for x in m["seconds_from_tool_use_to_each_tool_progress"]), m["seconds_from_tool_use_to_tool_result"]))
w("""
What the adapter can rely on (n=2 per variant):
- A server that streams its answer (an event stream whose headers come at once) is NOT cut at 60 s: the 70 s call
  returned its result, with or without progress events. Part 5's 60 s end (`The operation timed out.`) belongs to a
  server that sends nothing until the tool is done.
- That 60 s limit can be moved: `MCP_TOOL_TIMEOUT=120000` in the CLI's environment, or `"timeout": 120000` in the
  server's entry of the MCP config, and the same JSON server's 70 s call returned.
- Progress notifications of the server make no `tool_progress` line of their own: in every run the only lines were the
  CLI's heartbeats at 30 and 60 s (`"heartbeat":true`), as for a stdio server. The call did carry a `progressToken`.
- So the watchdog's comment (`has_heartbeat`) can say that an HTTP call gets the same heartbeat, and the sentence for
  the user when an HTTP tool ends at 60 s can name the cause: a server that answers only when done, and the two
  documented ways to give it more time (`MCP_TOOL_TIMEOUT`, the server's `timeout`).
""")

# ================================================================= 11
w("## 11. A shutdown with everything at once: a turn, a background helper and a background command\n")
w("Question: `interrupt`, the lines until the `result`, `stop_task` for the command, stdin closed: the order, the exit time, anything left "
  "running?\n")
N11 = named("r8-shutdown-all-")
check(len(N11) == 2, "two recordings of section 11")
w("Fixtures: %s. Flags: %s. The CLI is the leader of its own process group, as the app starts it. Turn 1 starts a background command "
  "(`echo bg; sleep 229`) and a background helper (its Bash call: `echo begin; sleep 233; echo end`) and ends. Turn 2 runs `echo fg; sleep "
  "239` in the foreground; 2 s after the host sees all three `sleep` processes: `interrupt`, the wait for turn 2's `result`, `stop_task` with "
  "the command's id, stdin closed 0.5 s after its answer.\n" % (both(N11), flags(N11[0])))
for n in N11:
    w("From the note before the stop to the end (`%s`):\n" % n)
    w(block(tl(n, lambda l: task_or_list(l) or ends(l) or l.get("type") == "control_response" or (l.get("type") == "user" and is_cli(l))
               or (kind(l) == "meta" and any(k in l["_meta"] for k in ("before_the_stop", "before_stdin_is_closed"))), start=from_note("before_the_stop"))))
EXIT11, CALLS11 = [], []
for n in N11:
    bs, bc, ae = note(n, "before_the_stop"), note(n, "before_stdin_is_closed"), note(n, "after_the_exit")
    lines = bc["lines_since_the_interrupt"]
    check(bs["a_turn_runs"] and bs["sleep_229_alive"] and bs["sleep_233_alive"] and bs["sleep_239_alive"] and len(bs["list"]) == 2, "everything runs before the stop in " + n)
    check(lines[:4] == ['system/task_updated "killed"', 'system/task_notification "stopped"', "system/background_tasks_changed 1 listed", "control_response success"], "the interrupt's first lines in " + n)
    k = lines.index("result/error_during_execution")
    check(sorted(lines[4:k]) == sorted(['system/task_notification "stopped"'] * (len(lines[4:k]) - 4) + ["user (helper)", "user (helper)", "user", "user"]), "the lines up to the result in " + n)
    check(lines[k + 1:] == ['system/task_updated "killed"', 'system/task_notification "stopped"', "system/background_tasks_changed 0 listed", "control_response success"], "stop_task's lines in " + n)
    check(not any(bc[x] for x in ("sleep_229_alive", "sleep_233_alive", "sleep_239_alive")) and bc["list"] == [], "nothing runs before the close in " + n)
    check(ae["lines_after_stdin_was_closed"] == [] and ae["processes_left_in_the_group"] == [] and not any(ae[x] for x in ("sleep_229_alive", "sleep_233_alive", "sleep_239_alive")), "nothing left in " + n)
    closed = [l["_t"] for l in fx(n) if "_meta" in l and "closing_stdin" in l["_meta"]][0]
    gone = [l["_t"] for l in fx(n) if "_meta" in l and "process_exited" in l["_meta"]][0]
    EXIT11.append(gone - closed)
    check(note(n, "exit_code") == 1, "exit code 1 in " + n)
    # Which Bash calls had become tasks (a `task_started` that is not the command's or the helper's) and how they ended.
    calls = {l["task_id"]: l for l in fx(n) if sub(l) == "task_started" and l["task_id"] not in (bs["command_task"], bs["helper_task"])}
    ends11 = [l for l in fx(n) if sub(l) == "task_notification" and l["task_id"] in calls]
    check(len(ends11) == len(calls) and all(l["status"] == "stopped" for l in ends11), "each call that became a task ends stopped in " + n)
    CALLS11.append(sorted("the helper's" if calls[l["task_id"]].get("owned_by_subagent") else "the turn's" for l in ends11))
check(sorted(CALLS11) == [["the helper's"], ["the helper's", "the turn's"]], "the calls that became tasks in section 11")
w("""What the adapter can rely on (n=2):
- `interrupt` with a turn running ends the HELPER first (`killed`, `stopped`, a list line that still names the
  command: 1 listed) and answers. Then each Bash call that had become a task gets a `task_notification` `stopped` (the
  helper's own call in both runs; the turn's foreground call in the one run where it had run long enough, about 3 s,
  to become a task), the helper's two `user` lines and the turn's two `user` lines (rejected tool result,
  `[Request interrupted by user for tool use]`) come in either order, then the turn's `result`
  `error_during_execution` (%s s after the request). The command is not touched.
- `stop_task` with the command's id then ends it (`killed`, `stopped`, the list `[]`, the answer `{}`).
- After stdin close the process was gone in %s s, exit code 1 (the interrupted turn had failed). Nothing was left: no
  `sleep` of the scenario, an empty process group.
- So the order of part 5 holds with a turn running: `interrupt`, wait for the turn's `result`, `stop_task` for each
  listed command, close.
""" % (" / ".join("%.3f" % note(n, "seconds_from_the_interrupt")["result"] for n in N11), " / ".join("%.2f" % x for x in EXIT11)))

# ================================================================= 12
w("## 12. `apply_flag_settings` with an effort level while a turn runs\n")
w("Question: `effortLevel: null` and a level, sent while a turn runs: the answer, and what `get_settings` says during and after the turn.\n")
N12 = {v: named("r8-effort-mid-turn-%s-" % v) for v in ("null", "high")}
check(all(len(x) == 2 for x in N12.values()), "two recordings per variant of section 12")
w("Fixtures: `r8-effort-mid-turn-null-*` (`{\"effortLevel\": null}`), `r8-effort-mid-turn-high-*` (`{\"effortLevel\": \"high\"}`). Flags: %s. "
  "The turn runs `echo go; sleep 13.5`; 1 s after the host sees the `sleep`: `get_settings`, the request, `get_settings`; then `get_settings` "
  "after the turn's result and after one more turn.\n" % flags(N12["null"][0]))
w("The request and its answer while the call runs (`%s`):\n" % N12["high"][0])
w(block(qpairs(N12["high"][0], "apply_flag_settings")))
STEPS12 = ("as started", "during the call, before the request", "during the call, after the request", "after the turn's result", "after the next turn")
w("| recording | %s | the request's answer |" % " | ".join(STEPS12))
w("|---|---|---|---|---|---|---|")
for v, ns in N12.items():
    for n in ns:
        eff = [one(n, "step", s)["applied_effort"] for s in STEPS12]
        runs = [one(n, "step", s)["a_turn_runs"] for s in STEPS12]
        ap = one(n, "step", "apply_flag_settings")
        want = {"null": "medium", "high": "high"}[v]
        check(eff == ["low", "low", want, want, want] and runs == [False, True, True, False, False] and ap["answer_subtype"] == "success" and ap["a_turn_runs"] and ap["sleep_13_5_alive"], "the steps of " + n)
        w("| `%s` | %s | `%s` |" % (n, " | ".join("`%s`" % e for e in eff), ap["answer_subtype"]))
w("""
What the adapter can rely on (n=2 per variant):
- The request is taken while a turn runs (its call still running): answer success, and `get_settings` says the new
  level at once (`null` -> `medium`, `high` -> `high`), after the turn's result and after the next turn.
- So an effort change need not be refused while a turn runs, as far as the CLI's own state goes. Still not visible:
  whether the model request that follows in the same turn uses the new level (no line says which level a request
  carries).
""")

# ================================================================= late notes
w("## Notes written late\n")
w("Each note of this part that lists lines is compared with the lines in the fixture between the step the script marked just before it "
  "(named in the last column) and the note:\n")


def kindline(l):
    return is_cli(l) and l.get("type") not in ("stream_event", "rate_limit_event") and l.get("subtype") not in ("thinking_tokens", "commands_changed", "status")


def last_request(l):
    return "_sent" in l and l["_sent"].get("type") == "control_request" and l["_sent"]["request"]["subtype"] in ("interrupt", "stop_task")


SINCE8 = {"after_the_exit": ("the close of stdin", lambda l: "_meta" in l and "closing_stdin" in l["_meta"]),
          "before_stdin_is_closed": ("the `interrupt` request", is_sent("interrupt")),
          "after_stop_task": ("the `stop_task` request", is_sent("stop_task")),
          "after_the_request": ("the `interrupt` or `stop_task` request", last_request),
          "forty_s_after_the_stop": ("the note `run`", from_note("run")),
          "turn": ("the `interrupt` request", is_sent("interrupt"))}
w("| fixture | note | lines in its list | lines in the file before it | counted from |")
w("|---|---|---|---|---|")
LATE8, CHECKED8 = [], 0
for n in ALL:
    lines = fx(n)
    for i, l in enumerate(lines):
        m = l.get("_meta") or {}
        found = [(key, listed) for key, v in m.items() for k2, listed in (v.items() if isinstance(v, dict) else []) if k2.startswith("lines") and isinstance(listed, list)]
        found += [("turn", listed) for k2, listed in m.items() if k2.startswith("lines") and isinstance(listed, list)]
        for key, listed in found:
            check(key in SINCE8, "a starting point for the note %r of %s" % (key, n))
            label, at = SINCE8[key]
            since = [x for x, y in enumerate(lines[:i]) if at(y)][-1]
            seen = sum(1 for x in lines[since:i] if kindline(x))
            CHECKED8 += 1
            if seen != len(listed):
                LATE8.append((n, key))
            w("| `%s` | `%s` | %d | %d | %s |" % (n, key, len(listed), seen, label))
check(LATE8 == [], "no late note in part 6: %r" % LATE8)
w("""
All %d notes of this part that list lines list exactly the lines in their window: none is late.
""" % CHECKED8)

# ================================================================= code
w("""## Can the waiting code be built now? (as of the day of recording)

| # | the code that waits | can it be built now? |
|---|---|---|
| 1 | `claude-part5-truth` item 1: `build_args` drops `--strict-mcp-config` in a folder you have not trusted when "Use my Claude setup" is on | partly: with `--setting-sources user` yes (the repo's `.mcp.json` server never loads, %d recordings here and 16 before; a plugin the user turned off stays off). With `user,local` NO: the repo's `.claude/settings.local.json` turns on a plugin the user turned off, and its server starts (n=4). Take the item back for `user,local` |
| 2 | "Undo a turn" for the conversation (`checkpoints.rs`; the rewind list of `rewind-from-thread`) | yes: `--resume <id> --resume-session-at <uuid of the kept turn's last assistant line>` drops the later turns (n=2), same session id, the old lines stay in the file; the uuid comes from the stream's assistant lines. A dropped part cannot be resumed again by its uuid once a new turn was added |
| 3 | "Try again from here" and "branch this agent" | yes: `--resume <id> --fork-session` (with `--resume-session-at <uuid>` to cut) gives a new session id and leaves the old session as it was (n=2). Count cost from `model_usage_since_last_result`: the fork's `total_cost_usd` carries the old session's total |
| 4 | Codly's restore note vs the CLI's own rewind (`with_restore_note`) | partly: `CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING=1` turns `rewind_files` on (the setting does not); it puts Edit's files back to the state before the turn of a user message uuid of the host (n=2). Not recorded: files written by Bash, new files, helpers' edits |
| 5 | `stopHint` (`session.ts`) and the gate's stop (`background.rs`) | yes: a helper stopped during the turn's last model request gets a turn by itself after the `result` (n=2); during a tool call of the turn it is told inside the turn (part 5). A stopped background COMMAND started no turn (n=%d) |
| 6 | `run_limit_turn_end` (`breaker_run.rs`) and the comment at `SpawnSpec::max_budget_usd` | yes: after the stop the CLI spends nothing more (a helper with requests left stops, n=2); a call that runs on ends by `stop_task` with its task id (n=2) or by closing stdin, not by `interrupt` (n=2). Ending the run is needed only for that call |
| 7 | `on_task` (a line that must end no helper and no background row) | yes: a foreground call that fails ends with `task_notification` `failed`, one cut by an interrupt with `stopped`, same keys, `output_file` `""` (n=2 each) |
| 8 | "Turn off" while a turn runs (`AgentMcp.svelte`, `session.ts`) | no: the switch while a turn runs was not recorded (a throw-away HOME has no login). Recorded: a server that cannot start, off, then on in a new process: error `Connection closed`, the saved switch on again (n=2) |
| 9 | One answer for "approve and accept edits" on any card (`grantable`) | yes: on a Bash card too, the request's `setMode` item alone switches to `acceptEdits` (n=2). Never send all suggestions back: `addRules` writes `.claude/settings.local.json`. Another mode (`plan`, `dontAsk`) is obeyed |
| 10 | The `has_heartbeat` comment (`watchdog.rs`) and the sentence for an HTTP tool that ends at 60 s | yes: a streaming HTTP server is not cut at 60 s; the limit moves with `MCP_TOOL_TIMEOUT` or the server's `timeout` (n=2 each); progress notifications add no line, only the heartbeat |
| 11 | `begin_shutdown` with a turn running (`claude.rs`) | yes: `interrupt`, the turn's `result`, `stop_task` for the command, close: exit in %s s, nothing left (n=2) |
| 12 | An effort change while a turn runs (`live_model_switch`, `runs.rs`) | partly: the CLI takes it mid-turn and `get_settings` shows it at once (n=2 per variant); which level the turn's next model request uses is not visible |
""" % (len(ALL1), len(N5["command-last-request"]) + len(N5["command-mid-turn"]), " / ".join("%.2f" % x for x in EXIT11)))

# ================================================================= table
w("""## Table: question -> answer -> fixture

| # | question | answer | n | fixture |
|---|---|---|---|---|
| 1 | Your own setup in a folder you have not trusted: user flags for project servers, plugins | The repo's `.mcp.json` server never starts; a user's plugin loads; under `user,local` the repo's `settings.local.json` turns on a plugin the user turned off | 2 per form | `r8-untrusted-mcp-*` |
| 2 | Rewind with `--resume-session-at` | Later turns dropped, same session id, old lines kept; a plain resume follows the rewound branch; the dropped uuid is not found later | 2 | `r8-rewind-*` |
| 3 | Fork with `--fork-session` | New session id; knows the old conversation (or up to `--resume-session-at`); `total_cost_usd` = old total + own turn; the old session unchanged | 2 | `r8-fork-*` |
| 4 | The CLI's own file rewind | On only with `CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING`; puts files back to before the message's turn; off: "File rewinding is not enabled." | 2 per way | `r8-file-rewind-*` |
| 5 | `stop_task` in the last moment; on a command mid-turn | Helper during the last request: a turn by itself follows; a command: no turn | 2, %d | `r8-stop-*` |
| 6 | After a budget stop | `interrupt` leaves the running call; `stop_task` ends it; a helper with requests left makes no more | 2 each | `r8-after-budget-*` |
| 7 | A Bash task that fails; one cut by an interrupt | `task_notification` `failed` / `stopped`, same keys, `output_file` `""` | 2 | `r8-bash-task-fails-*` |
| 8 | MCP switch mid-turn; a failing server off and on in a new process | Mid-turn: not recorded. New process: `disabled`, not started; "on" fails with `Connection closed` and saves the switch on | 2 | `r8-mcp-cannot-start-saved-*` |
| 9 | `setMode` on a Bash card | The `setMode` item alone switches to `acceptEdits`; all suggestions also write `settings.local.json`; `plan` and `dontAsk` obeyed | 2 per variant | `r8-bash-set-mode-*` |
| 10 | A slow HTTP MCP tool, other kinds and settings | An event stream outlasts 60 s; `MCP_TOOL_TIMEOUT` and the server's `timeout` move the limit; only heartbeats | 2 per variant | `r8-http-slow-*` |
| 11 | Shutdown with a turn, a helper and a command | interrupt ends the helper and the turn, `stop_task` the command, exit after close, nothing left | 2 | `r8-shutdown-all-*` |
| 12 | Effort switch while a turn runs | Taken at once (`null` -> `medium`, `high`), shown by `get_settings` during and after | 2 per variant | `r8-effort-mid-turn-*` |

## Not verified

Not in this part (still open from the earlier lists):
- The text printed when `failIfUnavailable` stops a start (needs a machine where the sandbox cannot start).
- `mcp_authenticate`, `mcp_clear_auth`, `mcp_oauth_callback_url` (a real sign-in; may write to the login keychain).
- `claude --resume` typed into the app's terminal on a session Codly started headless.
- Linux behaviour of process groups.
- A `tool_progress` line for WebFetch (needs a slow https page with a trusted certificate).
- A network allow that lasts hours (part 3 recorded 304 s).
- Whether a model request carries the effort level that `get_settings` shows.
- The `can_use_tool` request of a user-scope server (needs the user's real setup).
- A turn that fails by an API error while a helper works.
- A REAL home for the saved MCP switch (all switch recordings use a throw-away HOME).

Open after these recordings:
- 1: a plugin the user never installed, named by a repo file (with `extraKnownMarketplaces`); a plugin's hooks and
  skills under `user,local`; the logged-in forms of this section.
- 2: a rewind to a user message's uuid; a rewind in a turn with tool calls or helpers; `--resume-drops-turn`.
- 3: a fork of a session with a helper or a background command still running.
- 4: a file written by Bash, a new file, a helper's edit; whether the agent is told of the rewind.
- 5: the foreground call that ended with `Exit code 144` in one run of `command-mid-turn` (cause not found).
- 6: a helper whose limit passes during a long single request (the story) while it still has more to do.
- 8: `mcp_toggle` while a turn runs (needs a throw-away HOME with a login).
- 10: an HTTP server whose event stream sends nothing for more than 5 minutes (bundle: an idle limit of 300 s for
  servers that are not stdio, `CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT`).
- 12: the level a model request carries after the switch.
""" % (len(N5["command-last-request"]) + len(N5["command-mid-turn"])))

text = "\n".join(P) + "\n"
dest = sys.argv[1] if len(sys.argv) > 1 else os.path.join(OUT, "NOTES-part6.md")
with open(dest, "w") as f:
    f.write(text)
print("wrote", dest, len(text), "characters")
