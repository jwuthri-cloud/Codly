"""B7: which process group do the children of a Bash call get, and do they outlive the CLI?

The CLI is started as the leader of its own process group (as the app does). Turns:
  1. `echo start; sleep 301 &`             (a Bash command that STARTS with a long sleep is refused by the CLI: variant "blocked")
  2. `sleep 47` with run_in_background      (short, to see whether the CLI waits for it after stdin is closed)
  3. `ps -o pid,pgid,ppid,command -p <pid of sleep 301>,$$,$PPID`   (the pid is found by the host)
Then the host runs `ps` on the same pids, ends the CLI and looks again.
argv[1]: stdin (close stdin and wait up to 70 s for the exit) | sigterm (SIGTERM to the CLI's pid only)
         | sigkill (SIGKILL to the CLI's pid only) | blocked (only `sleep 301 & echo $! > bg.pid` and `sleep 301 &`)
Every process the run started is killed at the end.
"""
import json, os, signal, subprocess, sys, time
from drv import Session
variant = sys.argv[1] if len(sys.argv) > 1 else "stdin"


def sleeps(pattern="[s]leep (301|47)$"):
    out = subprocess.run(["pgrep", "-f", pattern], capture_output=True, text=True).stdout
    return [int(x) for x in out.split() if int(x) not in BEFORE]


BEFORE = ()
BEFORE = sleeps()  # processes of other programs that happen to match: never counted, never touched
s = Session("r4-process-groups-" + variant, extra=["--tools", "Bash"], hard_timeout=200, own_group=True)
cli = s.p.pid


def ps(pids):
    """ps rows of these pids only (nothing else of the machine is recorded); the command column is cut to 60 characters."""
    if not pids:
        return []
    out = subprocess.run(["ps", "-o", "pid=,pgid=,ppid=,sess=,stat=,command=", "-p", ",".join(str(p) for p in pids)], capture_output=True, text=True).stdout
    rows = []
    for line in out.splitlines():
        f = line.split(None, 5)
        if len(f) == 6:
            rows.append({"pid": int(f[0]), "pgid": int(f[1]), "ppid": int(f[2]), "sess": f[3], "stat": f[4],
                         "command": "claude (the CLI)" if int(f[0]) == cli else f[5][:60]})
    return rows


def children(pid):
    out = subprocess.run(["pgrep", "-P", str(pid)], capture_output=True, text=True).stdout
    return [int(x) for x in out.split()]


def group_members(pgid):
    out = subprocess.run(["ps", "-ax", "-o", "pid=,pgid="], capture_output=True, text=True).stdout
    return [int(a) for a, b in (l.split() for l in out.splitlines() if len(l.split()) == 2) if int(b) == pgid]


n = 0
def turn(text, wait=60):
    global n
    n += 1
    s.user(text)
    ok = s.wait_results(n, wait)
    time.sleep(0.5)
    return ok


s.note(cli_pid=cli, cli_pgid=os.getpgid(cli), driver_pgid=os.getpgid(0))
if variant == "blocked":
    # the two commands as the task first named them: the CLI refuses a Bash command that starts with a long `sleep`
    print("turn 1:", turn("Run exactly this Bash command (foreground, as written, one attempt only): sleep 301 & echo $! > bg.pid"))
    print("turn 2:", turn("Run exactly this Bash command (foreground, as written, one attempt only): sleep 301 &"))
    s.note(sleep_301_pids=sleeps("[s]leep 301$"))
    print("exit:", s.close(), "left:", sleeps())
    sys.exit(0)
print("turn 1:", turn("Run exactly this Bash command (foreground, as written, one attempt only): echo start; sleep 301 &"))
bg = sleeps("[s]leep 301$")
s.note(after="echo start; sleep 301 &", sleep_301_pids=bg)
print("sleep 301 pids:", bg)
print("turn 3:", turn("Run `sleep 47` with the Bash tool using run_in_background: true. Do not wait for it and do not check its output: right after starting it, reply with the single word: started"))
pid_list = ",".join([str(p) for p in bg] + ["$$", "$PPID"])
print("turn 4:", turn("Run exactly this Bash command (foreground, as written): ps -o pid,pgid,ppid,command -p " + pid_list))
mine = sleeps()
parents = [r["ppid"] for r in ps(mine) if r["ppid"] not in (1, cli)]
watch = sorted(set(mine + parents + [cli]))
s.note(when="CLI alive", host_ps=ps(watch), children_of_cli=ps(children(cli)), group_of_cli=ps(group_members(cli)))
print("watch:", ps(watch))
t = time.time()
if variant == "stdin":
    s.note(closing_stdin=True)
    s.p.stdin.close()
else:
    s.note(sending="%s to the CLI pid only" % variant.upper())
    os.kill(cli, signal.SIGTERM if variant == "sigterm" else signal.SIGKILL)
code, checks = None, []
while time.time() - t < 70:
    try:
        code = s.p.wait(1.0)
        break
    except subprocess.TimeoutExpired:
        if int(time.time() - t) % 10 == 0:
            checks.append({"s_after": round(time.time() - t, 1), "cli_alive": True, "host_ps": ps(watch)})
            time.sleep(1.0)
took = round(time.time() - t, 2)
time.sleep(1.0)
after = ps(watch)
s.killer.cancel()
print("exit:", code, "after", took, "s; alive:", after)
log = lambda msg: s.raw.write(json.dumps({"t": round(time.time() - s.t0, 3), "dir": "meta", "msg": msg}) + "\n")
with s.cv:
    for c in checks:
        log(c)
    log({"exit_code": code, "seconds_from_%s_to_exit" % ("stdin_close" if variant == "stdin" else variant): took if code is not None else "no exit within 70 s",
         "when": "1 s after the CLI ended" if code is not None else "70 s later, CLI still alive", "host_ps": after})
# clean up everything this run started
if code is None:
    s.p.kill()
    s.p.wait()
for p in sleeps():
    try:
        os.kill(p, signal.SIGKILL)
    except ProcessLookupError:
        pass
for p in parents:
    try:
        os.kill(p, signal.SIGKILL)
    except ProcessLookupError:
        pass
try:
    os.killpg(cli, signal.SIGKILL)
except (ProcessLookupError, PermissionError):
    pass
time.sleep(0.5)
left = sleeps()
with s.cv:
    log({"cleanup": "killed what was left of this run", "sleeps_left": left})
    s.raw.close()
print("left:", left)
