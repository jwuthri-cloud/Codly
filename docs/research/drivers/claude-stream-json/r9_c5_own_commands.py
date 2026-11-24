"""C5: your own slash commands and skills. What `init` lists, and what a user message "/<command> <words>" does in
stream-json: does the command's prompt run, which lines mark it; an unknown "/nope".

The scenario's files (no file of the user's):
  a user command    HOME/.claude/commands/codly-hello.md     (only in the throw-away HOME variants)
  a user skill      HOME/.claude/skills/codly-skill/SKILL.md (only in the throw-away HOME variants)
  a project command <work folder>/.claude/commands/codly-proj.md
  a project skill   <work folder>/.claude/skills/codly-proj-skill/SKILL.md
Each command or skill asks for one fixed reply ("hello says <words>", "proj says <words>", "skill ran", "proj skill
ran"), so a reply shows that its prompt ran.

argv[1]: home-all    a THROW-AWAY HOME (no login: each message ends at once with "Not logged in"; the recording shows
                     what is listed and which lines a message gets), `--setting-sources user,project,local`
         home-user   the same with `--setting-sources user` (the form Codly uses in a folder you have not trusted)
         login       the REAL login and HOME, `--strict-mcp-config --setting-sources project`: only the work folder's
                     own files are a source; `disableAllHooks` in --settings, as in every logged-in run. Your own user
                     commands and skills are not touched (no user source; the sanitizer keeps only the scenario's
                     names)
argv[2]: a label (1, 2: the same scenario again)

Messages, one turn each, with `--replay-user-messages`: "/codly-hello world", "/codly-proj world", "/codly-skill",
"/codly-proj-skill", "/nope world". A note per message: the turn's lines, the result's text, whether the replayed
user lines are marked `isReplay`.
"""
import json, os, sys, time
from drv import Session, make_repo
from r5_lib import next_turn
from r6_lib import kinds
from r7_lib import home
from r8_lib import last_result
VARIANT = sys.argv[1] if len(sys.argv) > 1 else "home-all"
LABEL = sys.argv[2] if len(sys.argv) > 2 else "1"
if VARIANT not in ("home-all", "home-user", "login"):
    raise SystemExit("unknown variant " + VARIANT)
name = "r9-own-commands-%s-%s" % (VARIANT, LABEL)
cwd = make_repo(name)


def put(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def command(reply):
    return "---\ndescription: A test command of the recording.\n---\nReply with exactly these words and nothing else: %s $ARGUMENTS\n" % reply


def skill(skill_name, reply):
    return ("---\nname: %s\ndescription: A test skill of the recording. Use it only when it is asked for by name.\n---\n"
            "Reply with exactly these words and nothing else: %s\n" % (skill_name, reply))


put(os.path.join(cwd, ".claude", "commands", "codly-proj.md"), command("proj says"))
put(os.path.join(cwd, ".claude", "skills", "codly-proj-skill", "SKILL.md"), skill("codly-proj-skill", "proj skill ran"))
kw = dict(mode="default", hard_timeout=150, on_can_use_tool=lambda req: {"behavior": "deny", "message": "The user denied this action.", "interrupt": False})
if VARIANT == "login":
    s = Session(name, extra=["--tools", "Bash", "--replay-user-messages", "--settings", json.dumps({"disableAllHooks": True})],
                setting_sources="project", **kw)
else:
    h = home(name)
    put(os.path.join(h, ".claude", "commands", "codly-hello.md"), command("hello says"))
    put(os.path.join(h, ".claude", "skills", "codly-skill", "SKILL.md"), skill("codly-skill", "skill ran"))
    s = Session(name, extra=["--tools", "Bash", "--replay-user-messages"], strict_mcp=False,
                setting_sources="user,project,local" if VARIANT == "home-all" else "user", env={"HOME": h}, **kw)
OWN = ("codly-hello", "codly-proj", "codly-skill", "codly-proj-skill")
for text in ("/codly-hello world", "/codly-proj world", "/codly-skill", "/codly-proj-skill", "/nope world"):
    m = s.mark()
    ok = next_turn(s, text, 60)
    time.sleep(0.5)
    with s.cv:
        users = [e for e in s.events[m:] if e.get("type") == "user" and not e.get("parent_tool_use_id")]
    r = last_result(s, m)
    s.note(sent=text, ok=ok, lines=kinds(s, m), result_subtype=r.get("subtype"), result=r.get("result"),
           user_lines=[{"isReplay": e.get("isReplay"), "isSynthetic": e.get("isSynthetic"),
                        "content_kind": "text" if isinstance(e["message"].get("content"), str) else "blocks"} for e in users])
    print(text, "->", r.get("subtype"), repr((r.get("result") or "")[:120]), kinds(s, m))
init = s.wait_for(lambda e: e.get("type") == "system" and e.get("subtype") == "init", 1) or {}
names = lambda xs: sorted(x for x in ((c.get("name") if isinstance(c, dict) else c) for c in xs) if x in OWN)
s.note(init_slash_commands_of_the_scenario=names(init.get("slash_commands", [])), init_skills_of_the_scenario=names(init.get("skills", [])))
print("init commands:", names(init.get("slash_commands", [])), "skills:", names(init.get("skills", [])), "cost:", s.cost(), "exit:", s.close())
