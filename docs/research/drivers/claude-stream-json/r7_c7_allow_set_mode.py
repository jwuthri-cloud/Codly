"""C7: an allow answer that carries the request's own suggestion `{"type":"setMode","mode":"acceptEdits",
"destination":"session"}` in `updatedPermissions`: what does `get_settings` say after it, does the next Edit ask, does
the stream say that the mode changed?

The process starts in mode `default` with `--tools Read,Edit`. Turn 1 asks for one Edit of hello.txt. The first
`can_use_tool` request for Edit is answered `allow` with `updatedPermissions` = the request's own `setMode`
suggestion(s), copied unchanged (if the request has none, the run stops: record again). Every other request is
answered with a plain `allow` and noted. After turn 1: get_settings. Turn 2 asks for a second Edit of the same file;
then get_settings again. The notes list the requests, the `system/status` lines that name a `permissionMode`, and the
`permissionMode` of each turn's `init`.

argv[1]: a label for the recording (1, 2): the same scenario, recorded more than once.
"""
import sys
from drv import Session
from r5_lib import ask, next_turn
from r6_lib import is_init
run = sys.argv[1] if len(sys.argv) > 1 else "1"
seen = []


def handler(req):
    seen.append(req)
    answer = {"behavior": "allow", "updatedInput": req.get("input", {})}
    if req.get("tool_name") == "Edit" and sum(1 for q in seen if q.get("tool_name") == "Edit") == 1:
        modes = [x for x in req.get("permission_suggestions") or [] if x.get("type") == "setMode"]
        if not modes:
            raise SystemExit("the first Edit request has no setMode suggestion: record again")
        answer["updatedPermissions"] = modes
    return answer


s = Session("r7-allow-set-mode-%s" % run, mode="default", extra=["--tools", "Read,Edit"], hard_timeout=150, on_can_use_tool=handler)


def mode_lines(start=0):
    with s.cv:
        return [e.get("permissionMode") for e in s.events[start:] if e.get("type") == "system" and e.get("subtype") == "status" and "permissionMode" in e]


def settings(step):
    r = ask(s, "get_settings").get("response", {})
    s.note(step=step, effective_permissions=r.get("effective", {}).get("permissions"), applied=r.get("applied"),
           sources=[x.get("source") for x in r.get("sources", [])])


def turn(label, text):
    m, before = s.mark(), len(seen)
    ok = next_turn(s, text, 60)
    init = s.wait_for(is_init, 1, m) or {}
    s.note(turn=label, ok=ok, init_permissionMode=init.get("permissionMode"),
           requests=[[q.get("tool_name"), q.get("input", {}).get("file_path", "").rsplit("/", 1)[-1], [x.get("type") for x in q.get("permission_suggestions") or []]]
                     for q in seen[before:]],
           status_lines_with_permissionMode=mode_lines(m))
    print(label, "ok:", ok, "init mode:", init.get("permissionMode"), "requests:", [q.get("tool_name") for q in seen[before:]], "mode lines:", mode_lines(m))


turn("1", "Use the Edit tool to change the text of hello.txt from `hello` to `hello one` (read the file first). Then reply with the single word: done")
settings("after turn 1")
turn("2", "Use the Edit tool to change the text of hello.txt from `hello one` to `hello two`. Then reply with the single word: done")
settings("after turn 2")
print("hello.txt:", open(s.cwd + "/hello.txt").read().strip(), "cost:", s.cost(), "exit:", s.close())
