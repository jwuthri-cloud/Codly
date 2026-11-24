"""D6: a turn with no network. Which lines come (retry lines and their waits, the result, the error text, the exit
code), how long until the turn ends, and what a turn does when the network is back.

argv[1]: proxy   HTTPS_PROXY / HTTP_PROXY (both spellings) point to a port on 127.0.0.1 that the script holds bound but
                 not listening (every connection is refused; no other program can take the port: r9_lib.Refuser).
                 Turn 1 runs with it. Then the script makes that port a CONNECT proxy that only copies bytes (the TLS
                 inside stays end to end), and turn 2 runs in the SAME process: the network is back.
         dns     ANTHROPIC_BASE_URL=https://api.no-such-host.invalid (a name that never resolves: `.invalid`). Turn 1
                 runs with it; stdin is closed. Process b (…-dns-<label>-b) resumes the session with no such variable:
                 the network is back in a new process.
argv[2]: a label for the recording (1, 2)

A note after each turn: the seconds from the host's message to each system line and to the result (time stamps of the
lines), the result's subtype and error text. The proxy notes how many CONNECTs it tunnelled (host and port only).
"""
import sys
from drv import Session
from r5_lib import next_turn
from r6_lib import kinds, raw
from r8_lib import last_result
from r9_lib import Refuser
VARIANT = sys.argv[1] if len(sys.argv) > 1 else "proxy"
LABEL = sys.argv[2] if len(sys.argv) > 2 else "1"
if VARIANT not in ("proxy", "dns"):
    raise SystemExit("unknown variant " + VARIANT)
NAME = "r9-no-network-%s-%s" % (VARIANT, LABEL)
ASK = "Reply with the single word: pong"
LIMIT = 1100  # seconds a turn may take before the script gives up on it


def turn_note(s, m, turn):
    sent = [r["t"] for r in raw(s) if r["dir"] == "send" and r["msg"].get("type") == "user"][-1]
    rows = [r for r in raw(s) if r["dir"] in ("recv", "stderr") and r["t"] >= sent]
    system = [dict({k: v for k, v in r["msg"].items() if k not in ("uuid", "session_id")}, _seconds=round(r["t"] - sent, 3))
              for r in rows if r["dir"] == "recv" and isinstance(r["msg"], dict) and r["msg"].get("type") == "system"
              and r["msg"].get("subtype") not in ("init", "status", "thinking_tokens", "commands_changed")]
    res = [round(r["t"] - sent, 3) for r in rows if r["dir"] == "recv" and isinstance(r["msg"], dict) and r["msg"].get("type") == "result"]
    r = last_result(s, m)
    s.note(turn=turn, seconds_to_the_result=res[0] if res else None, system_lines=system,
           stderr_lines=sum(1 for x in rows if x["dir"] == "stderr"), result_subtype=r.get("subtype"), result_is_error=r.get("is_error"),
           result=r.get("result"), result_errors=r.get("errors"), lines=kinds(s, m))
    print(turn, "->", r.get("subtype"), repr(r.get("result")), "in", res[0] if res else None, "s;", len(system), "system lines")


if VARIANT == "proxy":
    port = Refuser()
    url = "http://127.0.0.1:%d" % port.port
    env = {k: url for k in ("HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy")} | {"NO_PROXY": "", "no_proxy": ""}
    s = Session(NAME, extra=["--tools", "Bash"], hard_timeout=2 * LIMIT + 120, env=env)
    s.note(port_refuses_connections=port.refuses())
    m = s.mark()
    next_turn(s, ASK, LIMIT)
    turn_note(s, m, 1)
    port.start_proxy()
    s.note(proxy_started=True)
    m = s.mark()
    next_turn(s, ASK, LIMIT)
    turn_note(s, m, 2)
    s.note(proxy_connects=len(port.connects), proxy_hosts=sorted({h for _, h in port.connects}))
    print("exit:", s.close(), "cost:", s.cost())
    port.close()
else:
    a = Session(NAME + "-a", repo=NAME, extra=["--tools", "Bash"], hard_timeout=LIMIT + 120,
                env={"ANTHROPIC_BASE_URL": "https://api.no-such-host.invalid"})
    m = a.mark()
    next_turn(a, ASK, LIMIT)
    turn_note(a, m, 1)
    print("a exit:", a.close())
    b = Session(NAME + "-b", repo=NAME, extra=["--tools", "Bash", "--resume", a.sid], session_id=None, hard_timeout=180)
    m = b.mark()
    next_turn(b, ASK, 120)
    turn_note(b, m, 2)
    print("b exit:", b.close(), "cost:", b.cost())
