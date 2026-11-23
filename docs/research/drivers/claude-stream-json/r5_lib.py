"""Helpers shared by the r5_* scripts (part 3)."""
import json
import os
import shutil
import subprocess

HERE = os.path.dirname(os.path.abspath(__file__))


def allow_all(seen):
    """A can_use_tool handler that allows every request and keeps it in `seen`; a request to run outside the sandbox
    is denied, so that only the sandboxed path is recorded."""
    def handler(req):
        seen.append(req)
        inp = req.get("input", {})
        if inp.get("dangerouslyDisableSandbox"):
            return {"behavior": "deny", "message": "The user denied this action.", "interrupt": False}
        return {"behavior": "allow", "updatedInput": inp}
    return handler


def asked(reqs):
    """Short form of can_use_tool requests for a driver note."""
    out = []
    for r in reqs:
        i = r.get("input", {})
        out.append([r.get("tool_name"), i.get("command") or i.get("file_path") or i.get("host") or "", r.get("decision_reason_type")])
    return out


def ask(s, sub, wait=15, **f):
    """Send a control request, print the answer, return the `response` object."""
    r = s.control(sub, wait=wait, **f)
    resp = (r or {}).get("response", {})
    print("%-20s %-48s -> %s" % (sub, json.dumps(f)[:48], json.dumps({k: v for k, v in resp.items() if k != "request_id"})[:600]))
    return resp


def open_tasks(s):
    """The last background_tasks_changed list (the CLI has no request that lists open tasks)."""
    with s.cv:
        ev = [e for e in s.events if e.get("type") == "system" and e.get("subtype") == "background_tasks_changed"]
    return ev[-1]["tasks"] if ev else []


def task_started(s, start=0, timeout=60, task_type=None, background=None):
    def pred(e):
        return (e.get("type") == "system" and e.get("subtype") == "task_started"
                and (task_type is None or e.get("task_type") == task_type)
                and (background is None or bool(e.get("is_backgrounded")) == background))
    return s.wait_for(pred, timeout, start)


class Procs:
    """Processes whose command line matches a pattern and that did not exist before the run.
    Write the pattern so that it cannot match itself (`[s]leep 113`)."""

    def __init__(self, pattern):
        self.pattern = pattern
        self.before = set(self._all())

    def _all(self):
        return subprocess.run(["pgrep", "-f", self.pattern], capture_output=True, text=True).stdout.split()

    def alive(self):
        return [p for p in self._all() if p not in self.before]

    def kill_left(self):
        """Kill what the run left behind (never a process that existed before the run)."""
        left = self.alive()
        for p in left:
            subprocess.run(["kill", p], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        return left


def results(s):
    return s.count(lambda e: e.get("type") == "result")


def next_turn(s, text, timeout=60, **extra):
    """Send a user message and wait for one more result. Returns True when it came."""
    n = results(s)
    s.user(text, **extra)
    return s.wait_results(n + 1, timeout)


def ping_server(cwd, name="mcp_ping_server.py"):
    """Copy the test MCP server into a work folder and return its path."""
    dst = os.path.join(cwd, name)
    shutil.copy(os.path.join(HERE, "mcp_ping_server.py"), dst)
    return dst


def stdio_server(marker, **opts):
    """An mcpServers entry for the test server in the work folder (relative paths: the CLI starts it there)."""
    return {"type": "stdio", "command": "python3", "args": ["mcp_ping_server.py", marker] + ["%s=%s" % kv for kv in opts.items()]}


def lines_of(path):
    return open(path).read().splitlines() if os.path.exists(path) else []


def git(cwd, *args):
    return subprocess.run(["git", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"] + list(args), cwd=cwd, capture_output=True, text=True)
