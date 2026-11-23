"""Small stream-json driver for the Claude Code CLI (python3 standard library only).

Raw log format (raw/<scenario>.ndjson), one JSON object per line:
  {"t": <seconds since spawn>, "dir": "recv"|"send"|"stderr"|"meta", "msg": <parsed JSON or text>}
"""
import json
import os
import shutil
import subprocess
import threading
import time
import uuid

# Where raw/, fixtures/ and work/ live. Default: the folder that holds this drivers/ folder.
OUT = os.path.abspath(os.environ.get("CAPTURE_DIR") or os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CLAUDE = os.environ.get("CLAUDE_BIN") or shutil.which("claude") or "claude"
for _d in ("raw", "fixtures", "work"):
    os.makedirs(os.path.join(OUT, _d), exist_ok=True)


def clean_env():
    # the nested CLI must not inherit the parent session's variables
    return {k: v for k, v in os.environ.items() if not (k.startswith("CLAUDE") or k.startswith("ANTHROPIC"))}


def make_repo(name):
    d = os.path.join(OUT, "work", name)
    if not os.path.isdir(os.path.join(d, ".git")):
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, "README.md"), "w") as f:
            f.write("# demo\n\nA tiny demo repo.\n")
        with open(os.path.join(d, "hello.txt"), "w") as f:
            f.write("hello\n")
        q = dict(cwd=d, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run(["git", "init", "-q"], **q)
        subprocess.run(["git", "add", "-A"], **q)
        subprocess.run(["git", "-c", "user.name=demo", "-c", "user.email=demo@example.invalid", "-c", "commit.gpgsign=false",
                        "-c", "core.hooksPath=/dev/null", "commit", "-qm", "init"], **q)
    return d


class Session:
    def __init__(self, name, extra=(), mode="acceptEdits", session_id="new", hard_timeout=180,
                 on_can_use_tool="allow", repo=None, model="haiku", raw_name=None,
                 strict_mcp=True, setting_sources="local", cwd=None, own_group=False, env=None):
        """strict_mcp / setting_sources: the two flags that keep the user's own setup out.
        cwd: use this folder (it must exist) and not work/<repo>. own_group: start the CLI as the leader of a new
        process group (what the app does), so that the group can be inspected and signalled.
        env: variables set on top of the cleaned environment (a throw-away HOME); the recording notes them."""
        self.name = name
        self.cwd = cwd or make_repo(repo or name)
        self.sid = str(uuid.uuid4())
        args = [CLAUDE, "-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
                "--include-partial-messages", "--permission-prompt-tool", "stdio"]
        if mode:
            args += ["--permission-mode", mode]
        if session_id == "new":
            args += ["--session-id", self.sid]
        if model:
            args += ["--model", model]
        if strict_mcp:
            args += ["--strict-mcp-config"]
        if setting_sources:
            args += ["--setting-sources", setting_sources]
        args += list(extra)
        self.args = args
        self.events = []  # received, parsed
        self.cv = threading.Condition()
        self.on_can_use_tool = on_can_use_tool
        self.n_req = 0
        self.raw = open(os.path.join(OUT, "raw", (raw_name or name) + ".ndjson"), "w")
        self.t0 = time.time()
        self.exit_code = None
        self._log("meta", {"argv": args, "cwd": self.cwd})
        if env:
            self._log("meta", {"env_set": env})
        self.p = subprocess.Popen(args, cwd=self.cwd, env=clean_env() | (env or {}), stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1,
                                  **({"process_group": 0} if own_group else {}))
        threading.Thread(target=self._read_out, daemon=True).start()
        threading.Thread(target=self._read_err, daemon=True).start()
        self.killer = threading.Timer(hard_timeout, self._hard_kill)
        self.killer.daemon = True
        self.killer.start()

    def _log(self, direction, msg):
        with self.cv:
            self.raw.write(json.dumps({"t": round(time.time() - self.t0, 3), "dir": direction, "msg": msg}) + "\n")
            self.raw.flush()

    def _hard_kill(self):
        if self.p.poll() is None:
            self._log("meta", {"hard_timeout_kill": True})
            self.p.kill()

    def _read_out(self):
        for line in self.p.stdout:
            line = line.rstrip("\n")
            if not line:
                continue
            try:
                msg = json.loads(line)
            except ValueError:
                self._log("recv", {"_unparsed": line})
                continue
            self._log("recv", msg)
            with self.cv:
                self.events.append(msg)
                self.cv.notify_all()
            if msg.get("type") == "control_request":
                self._on_control_request(msg)

    def _read_err(self):
        for line in self.p.stderr:
            self._log("stderr", line.rstrip("\n"))

    def _on_control_request(self, msg):
        req = msg.get("request", {})
        if req.get("subtype") != "can_use_tool":
            return
        h = self.on_can_use_tool
        if h is None:
            return
        if h == "allow":
            resp = {"behavior": "allow", "updatedInput": req.get("input", {})}
        else:
            resp = h(req)
            if resp is None:
                return
        self.send({"type": "control_response",
                   "response": {"subtype": "success", "request_id": msg["request_id"], "response": resp}})

    def send(self, obj):
        self._log("send", obj)
        try:
            self.p.stdin.write(json.dumps(obj) + "\n")
            self.p.stdin.flush()
        except (BrokenPipeError, ValueError) as e:
            self._log("meta", {"stdin_write_failed": repr(e)})

    def user(self, content, **extra):
        m = {"type": "user", "session_id": "", "message": {"role": "user", "content": content}, "parent_tool_use_id": None}
        m.update(extra)
        self.send(m)

    def control(self, subtype, wait=15, **fields):
        self.n_req += 1
        rid = "host-%d" % self.n_req
        req = {"subtype": subtype}
        req.update(fields)
        self.send({"type": "control_request", "request_id": rid, "request": req})
        if wait:
            return self.wait_for(lambda e: e.get("type") == "control_response" and e.get("response", {}).get("request_id") == rid, wait)
        return rid

    def wait_for(self, pred, timeout=60, start=0):
        end = time.time() + timeout
        i = start
        with self.cv:
            while True:
                while i < len(self.events):
                    if pred(self.events[i]):
                        return self.events[i]
                    i += 1
                left = end - time.time()
                if left <= 0 or (self.p.poll() is not None and left < timeout - 1):
                    return None
                self.cv.wait(min(left, 0.5))

    def count(self, pred):
        with self.cv:
            return sum(1 for e in self.events if pred(e))

    def wait_results(self, n=1, timeout=90):
        end = time.time() + timeout
        while time.time() < end:
            if self.count(lambda e: e.get("type") == "result") >= n:
                return True
            if self.p.poll() is not None:
                time.sleep(0.5)
                return self.count(lambda e: e.get("type") == "result") >= n
            time.sleep(0.2)
        return False

    def note(self, **kv):
        """A driver note in the recording (becomes a _meta line of the fixture)."""
        self._log("meta", kv)

    def wait_tool_use(self, start=0, timeout=60, name=None):
        """Wait for a complete assistant line with a tool_use block (the tool is about to run)."""
        return self.wait_for(lambda e: e.get("type") == "assistant" and any(
            b.get("type") == "tool_use" and (name is None or b.get("name") == name) for b in e["message"]["content"]), timeout, start)

    def mark(self):
        with self.cv:
            return len(self.events)

    def alive(self):
        return self.p.poll() is None

    def cost(self):
        """total_cost_usd of the last result line (the CLI's running total for the session)."""
        with self.cv:
            r = [e for e in self.events if e.get("type") == "result"]
        return r[-1].get("total_cost_usd") if r else 0.0

    def close(self, wait=15, after_exit=None):
        """Close stdin, wait for the exit, kill if needed. Returns the exit code.
        after_exit: a function called once the process is gone; the dict it returns becomes the last note."""
        self._log("meta", {"closing_stdin": True, "alive_before_close": self.alive()})
        try:
            self.p.stdin.close()
        except Exception:
            pass
        try:
            self.exit_code = self.p.wait(wait)
        except subprocess.TimeoutExpired:
            self._log("meta", {"kill_after_close_timeout": wait})
            self.p.kill()
            self.exit_code = self.p.wait()
        time.sleep(0.3)
        self.killer.cancel()
        self._log("meta", {"exit_code": self.exit_code})
        if after_exit:
            self._log("meta", after_exit())
        self.raw.close()
        return self.exit_code


def summary(path_or_name):
    """Print a compact timeline of a raw recording (no stream_event lines)."""
    p = path_or_name if os.path.sep in path_or_name else os.path.join(OUT, "raw", path_or_name + ".ndjson")
    for line in open(p):
        r = json.loads(line)
        m = r["msg"]
        if r["dir"] == "recv" and isinstance(m, dict) and m.get("type") == "stream_event":
            continue
        s = json.dumps(m)
        print("%7.2f %-6s %s" % (r["t"], r["dir"], s[:400]))


if __name__ == "__main__":
    import sys
    summary(sys.argv[1])
