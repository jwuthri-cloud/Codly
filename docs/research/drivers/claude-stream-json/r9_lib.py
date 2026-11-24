"""Helpers shared by the r9_* scripts (part 7): a plugin with hooks, the files a throw-away HOME got, a session's
transcript as counts and line types, a socket that refuses connections, a small CONNECT proxy, a terminal (pty) run.
Processes are found with r5_lib.Procs (the whole command text)."""
import fcntl
import glob
import json
import os
import pty
import select
import shutil
import socket
import struct
import termios
import threading
import time

from drv import OUT, CLAUDE, clean_env
from r5_lib import HERE
from r8_lib import MARKET, PLUGIN, PLUGIN_SERVER, write_json

PLUGIN_ID = "%s@%s" % (PLUGIN, MARKET)


def plugin_market(name, server_marker, hook_marker):
    """A local marketplace folder under the capture folder with ONE plugin, `ping-plugin`: its `.mcp.json` brings the
    stdio server `ping-plugged` (one line in `server_marker` per start), and its `hooks/hooks.json` has a SessionStart
    and a UserPromptSubmit hook, each appends one line (the event's name) to `hook_marker`. Both markers are absolute
    paths in the work folder. Made new at each run (it is this script's own folder). Returns (folder, plugin folder)."""
    d = os.path.join(OUT, "work", name + "-market")
    shutil.rmtree(d, ignore_errors=True)
    root = os.path.join(d, PLUGIN)
    write_json(os.path.join(d, ".claude-plugin", "marketplace.json"), {
        "name": MARKET, "owner": {"name": "demo"},
        "plugins": [{"name": PLUGIN, "source": "./" + PLUGIN, "description": "A test plugin with one MCP server and two hooks."}]})
    write_json(os.path.join(root, ".claude-plugin", "plugin.json"), {"name": PLUGIN, "version": "0.0.1", "description": "A test plugin with one MCP server and two hooks."})
    shutil.copy(os.path.join(HERE, "mcp_ping_server.py"), os.path.join(root, "mcp_ping_server.py"))
    write_json(os.path.join(root, ".mcp.json"), {"mcpServers": {PLUGIN_SERVER: {
        "command": "python3", "args": ["${CLAUDE_PLUGIN_ROOT}/mcp_ping_server.py", server_marker, "label=plugin"]}}})
    write_json(os.path.join(root, "hooks", "hooks.json"), {"hooks": {
        ev: [{"hooks": [{"type": "command", "command": "echo %s >> %s" % (ev, hook_marker)}]}] for ev in ("SessionStart", "UserPromptSubmit")}})
    return d, root


def home_files(home_dir, keep=(PLUGIN, MARKET)):
    """The files under a throw-away HOME's `.claude/plugins` whose path names the scenario's plugin or marketplace,
    relative to the HOME (other plugins the CLI brings are left out: their names are not the scenario's), and the
    number of the others."""
    root = os.path.join(home_dir, ".claude", "plugins")
    mine, others = [], 0
    for d, _, names in os.walk(root):
        for n in names:
            rel = os.path.relpath(os.path.join(d, n), home_dir)
            if any(k in rel for k in keep):
                mine.append(rel)
            else:
                others += 1
    return sorted(mine), others


def session_dir(cwd):
    """The folder of ~/.claude/projects that holds this work folder's sessions (the CLI names it after the path, each
    character that is not a letter or a digit becoming '-')."""
    import re
    for p in (cwd, os.path.realpath(cwd)):
        d = os.path.join(os.path.expanduser("~"), ".claude", "projects", re.sub(r"[^A-Za-z0-9]", "-", p))
        if os.path.isdir(d):
            return d
    return None


def transcript_types(sid, start=0):
    """The `type` of each line of the session's transcript from line `start` on, with the `subtype`/`attachment.type`
    when there is one. Words of the CLI's own format only: nothing of the conversation is copied."""
    paths = glob.glob(os.path.join(os.path.expanduser("~"), ".claude", "projects", "*", sid + ".jsonl"))
    if len(paths) != 1:
        return None
    out = []
    for line in open(paths[0]).read().splitlines()[start:]:
        try:
            o = json.loads(line)
        except ValueError:
            out.append("(not json)")
            continue
        t = o.get("type", "?")
        extra = o.get("subtype") or (o.get("attachment") or {}).get("type")
        out.append(t + ("/" + extra if extra else ""))
    return out


def transcript_uuids(sid):
    """The set of `uuid` values in the session's transcript (to check that a uuid of the stream is in it)."""
    paths = glob.glob(os.path.join(os.path.expanduser("~"), ".claude", "projects", "*", sid + ".jsonl"))
    if len(paths) != 1:
        return set()
    out = set()
    for line in open(paths[0]):
        try:
            u = json.loads(line).get("uuid")
        except ValueError:
            continue
        if u:
            out.add(u)
    return out


class Refuser:
    """A TCP port on 127.0.0.1 that is bound but not listening: every connection to it is refused, and no other
    program can listen on it while this object lives. `start_proxy()` then makes it a small HTTP CONNECT proxy that
    only copies bytes both ways (the TLS inside stays end to end: it sees host names, never content)."""

    def __init__(self):
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.sock.bind(("127.0.0.1", 0))
        self.port = self.sock.getsockname()[1]
        self.connects = []  # (seconds since start, host:port) of each CONNECT the proxy got
        self.t0 = time.time()

    def refuses(self):
        c = socket.socket()
        try:
            c.connect(("127.0.0.1", self.port))
            return False
        except OSError:
            return True
        finally:
            c.close()

    def start_proxy(self):
        self.sock.listen(16)
        threading.Thread(target=self._serve, daemon=True).start()

    def _serve(self):
        while True:
            try:
                c, _ = self.sock.accept()
            except OSError:
                return
            threading.Thread(target=self._tunnel, args=(c,), daemon=True).start()

    def _tunnel(self, c):
        try:
            head = b""
            while b"\r\n\r\n" not in head and len(head) < 8192:
                part = c.recv(4096)
                if not part:
                    return
                head += part
            first = head.split(b"\r\n", 1)[0].decode("latin-1").split()
            if len(first) < 2 or first[0] != "CONNECT":
                c.sendall(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n")
                return
            host, port = first[1].rsplit(":", 1)
            self.connects.append((round(time.time() - self.t0, 3), first[1]))
            u = socket.create_connection((host, int(port)), timeout=30)
            c.sendall(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            rest = head.split(b"\r\n\r\n", 1)[1]
            if rest:
                u.sendall(rest)
            socks = [c, u]
            while True:
                r, _, _ = select.select(socks, [], [], 300)
                if not r:
                    return
                for s in r:
                    data = s.recv(65536)
                    if not data:
                        return
                    (u if s is c else c).sendall(data)
        except OSError:
            return
        finally:
            c.close()

    def close(self):
        self.sock.close()


class Terminal:
    """`claude` in a pseudo-terminal, as a person starts it (the interactive program). Its screen output is kept only
    to wait for words; it is never written to a recording. `keys()` types."""

    def __init__(self, args, cwd, env=None):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.chdir(cwd)
            e = clean_env() | {"TERM": "xterm-256color", "COLUMNS": "120", "LINES": "40"} | (env or {})
            os.execvpe(CLAUDE, [CLAUDE] + list(args), e)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
        self.screen = b""
        self.t0 = time.time()
        self.exit_code = None

    def read(self, secs):
        end = time.time() + secs
        while time.time() < end:
            r, _, _ = select.select([self.fd], [], [], 0.2)
            if r:
                try:
                    data = os.read(self.fd, 65536)
                except OSError:
                    return
                if not data:
                    return
                self.screen += data

    def wait_text(self, text, secs):
        """Read until the screen output (escape codes taken out) has this text; True when it came."""
        end = time.time() + secs
        while time.time() < end:
            self.read(0.5)
            if text.lower() in plain(self.screen).lower():
                return True
            if self.done():
                return text.lower() in plain(self.screen).lower()
        return False

    def keys(self, s):
        os.write(self.fd, s.encode())

    def done(self):
        if self.exit_code is not None:
            return True
        p, st = os.waitpid(self.pid, os.WNOHANG)
        if p == 0:
            return False
        self.exit_code = os.waitstatus_to_exitcode(st)
        return True

    def wait_exit(self, secs):
        end = time.time() + secs
        while time.time() < end:
            self.read(0.3)
            if self.done():
                return self.exit_code
        return None

    def kill(self):
        if not self.done():
            os.kill(self.pid, 15)
            self.wait_exit(5)


def plain(b):
    """Screen bytes without escape codes, spaces folded."""
    import re
    t = b.decode("utf-8", "replace")
    t = re.sub(r"\x1b\[[0-9;?<>=]*[ -/]*[@-~]", " ", t)
    t = re.sub(r"\x1b\][^\x07\x1b]*(\x07|\x1b\\)", " ", t)
    t = re.sub(r"\x1b[()][0-9A-Za-z]", " ", t)
    t = re.sub(r"\x1b.", " ", t)
    t = re.sub(r"[\x00-\x08\x0b-\x1f\x7f]", " ", t)
    return re.sub(r"\s+", " ", t)

