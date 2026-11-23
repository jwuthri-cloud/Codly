"""D6: sandbox on; the network request for a host is DENIED once. Is the next command to the same host asked about
again in the same process?

Four turns, one command each (the prompt forbids a retry):
  turn 1  `curl -sI https://example.com`   the request for the host is denied
  turn 2  the same command                 if a request comes, it is denied again
  turn 3  the same command                 if a request comes, it is allowed
  turn 4  the same command                 after the allow: a request?
  turn 5  `curl -sI https://example.org`   another host (control): allowed
After each turn: get_sandbox_dialog and list_permission_rules. A request to run outside the sandbox is always denied.
Record with CAPTURE_DIR outside the CLI's own temp folder.

argv[1]: a label for the recording (1, 2).
"""
import json, sys
from drv import Session
from r5_lib import asked, results
run = sys.argv[1] if len(sys.argv) > 1 else "1"
DENY = {"behavior": "deny", "message": "The user denied this action.", "interrupt": False}
seen, answers = [], []
deny_host = True  # what the next request for example.com gets


def handler(req):
    seen.append(req)
    inp = req.get("input", {})
    deny = bool(inp.get("dangerouslyDisableSandbox")) or (req["tool_name"] == "SandboxNetworkAccess" and inp.get("host") == "example.com" and deny_host)
    answers.append("deny" if deny else "allow")
    return DENY if deny else {"behavior": "allow", "updatedInput": inp}


s = Session("r6-sandbox-network-deny-" + run, extra=["--tools", "Bash", "--settings", json.dumps({"sandbox": {"enabled": True}})],
            hard_timeout=260, on_can_use_tool=handler)
s.control("get_sandbox_dialog")


def turn(cmd):
    before, n, m = len(seen), results(s), s.mark()
    s.user("Run exactly this Bash command once (do not retry it, do not change it) and report the first line of its output: " + cmd)
    ok = s.wait_results(n + 1, 70)
    with s.cv:
        calls = sum(1 for e in s.events[m:] if e.get("type") == "assistant" for b in e["message"]["content"] if b.get("type") == "tool_use")
    s.note(turn=n + 1, command=cmd, can_use_tool_requests=asked(seen[before:]), answers=answers[before:], tool_calls_of_the_turn=calls)
    print("turn %d ok=%s tool calls=%d can_use_tool=%s answers=%s" % (n + 1, ok, calls, asked(seen[before:]), answers[before:]))
    s.control("get_sandbox_dialog")
    s.control("list_permission_rules")


turn("curl -sI https://example.com")
turn("curl -sI https://example.com")
deny_host = False
turn("curl -sI https://example.com")
turn("curl -sI https://example.com")
turn("curl -sI https://example.org")
print("cost:", s.cost(), "exit:", s.close())
