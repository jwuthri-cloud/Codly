"""C9: an allow answer to a BASH request that carries several suggestions (part 1, scenario 10: `touch <file>` in mode
`default` gets addRules, addDirectories and setMode). Part 5 (section 7) sent the request's own `setMode` back for an
Edit request, whose only suggestion it is.

argv[1]: only-set-mode   the allow carries only the request's `setMode` item(s), copied unchanged
         all             the allow carries all of the request's suggestions, copied unchanged
         plan            the allow carries the request's `setMode` item with `mode` changed to `plan`
         dont-ask        the same with `mode` changed to `dontAsk`
argv[2]: a label for the recording (1, 2): the same scenario, recorded more than once.

The process starts in mode `default` with `--tools Bash,Read,Edit`. Turn 1: Bash `touch r8-one.txt`; its first Bash
request gets the variant's answer (if it has no setMode suggestion the run stops: record again). Every later request
gets a plain allow. Turn 2: Bash `touch r8-two.txt`. Turn 3: an Edit of hello.txt. After each turn a note: the
requests (tool, the kinds of their suggestions), the `permissionMode` of the turn's `init`, the `system/status` lines
that name a `permissionMode`, which files exist; at the end the work folder's `.claude/settings.local.json`.
"""
import copy, json, os, sys
from drv import Session
from r5_lib import next_turn
from r6_lib import is_init
from r7_lib import read_json
VARIANT = sys.argv[1] if len(sys.argv) > 1 else "only-set-mode"
LABEL = sys.argv[2] if len(sys.argv) > 2 else "1"
MODE = {"plan": "plan", "dont-ask": "dontAsk"}.get(VARIANT)
if VARIANT not in ("only-set-mode", "all", "plan", "dont-ask"):
    raise SystemExit("unknown variant " + VARIANT)
seen, sent = [], []
name = "r8-bash-set-mode-%s-%s" % (VARIANT, LABEL)


def handler(req):
    seen.append(req)
    answer = {"behavior": "allow", "updatedInput": req.get("input", {})}
    if req.get("tool_name") == "Bash" and sum(1 for q in seen if q.get("tool_name") == "Bash") == 1:
        sugg = req.get("permission_suggestions") or []
        modes = [x for x in sugg if x.get("type") == "setMode"]
        if not modes:
            raise SystemExit("the first Bash request has no setMode suggestion: record again")
        if VARIANT == "all":
            answer["updatedPermissions"] = sugg
        elif VARIANT == "only-set-mode":
            answer["updatedPermissions"] = modes
        else:
            answer["updatedPermissions"] = [dict(copy.deepcopy(modes[0]), mode=MODE)]
        sent.append(answer["updatedPermissions"])
    return answer


s = Session(name, mode="default", extra=["--tools", "Bash,Read,Edit"], hard_timeout=180, on_can_use_tool=handler)
for f in ("r8-one.txt", "r8-two.txt", os.path.join(".claude", "settings.local.json")):
    if os.path.exists(os.path.join(s.cwd, f)):
        os.remove(os.path.join(s.cwd, f))


def mode_lines(start):
    with s.cv:
        return [e.get("permissionMode") for e in s.events[start:] if e.get("type") == "system" and e.get("subtype") == "status" and "permissionMode" in e]


def turn(label, text):
    m, before = s.mark(), len(seen)
    ok = next_turn(s, text, 60)
    init = s.wait_for(is_init, 1, m) or {}
    s.note(turn=label, ok=ok, init_permissionMode=init.get("permissionMode"),
           requests=[[q.get("tool_name"), [x.get("type") + ("" if x.get("type") != "setMode" else ":" + x.get("mode", "")) for x in q.get("permission_suggestions") or []]]
                     for q in seen[before:]],
           status_lines_with_permissionMode=mode_lines(m), files={f: os.path.exists(os.path.join(s.cwd, f)) for f in ("r8-one.txt", "r8-two.txt")},
           hello_txt=open(os.path.join(s.cwd, "hello.txt")).read().strip())
    print(label, "ok:", ok, "init mode:", init.get("permissionMode"), "requests:", [q.get("tool_name") for q in seen[before:]], "mode lines:", mode_lines(m))


turn("1", "Run the Bash command `touch r8-one.txt` and then reply with the single word: done")
turn("2", "Run the Bash command `touch r8-two.txt` and then reply with the single word: done")
turn("3", "Use the Edit tool to change the text of hello.txt from `hello` to `hello three` (read the file first). Then reply with the single word: done")
s.note(updated_permissions_sent=sent, settings_local_json=read_json(os.path.join(s.cwd, ".claude", "settings.local.json")))
print("sent:", json.dumps(sent), "cost:", s.cost(), "exit:", s.close())
