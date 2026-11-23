"""C10b: tool names that do not exist in --tools and --allowedTools. What does the CLI say?"""
from drv import Session
s = Session("r4-unknown-tool-names", mode="manual", extra=["--tools", "Bash,Read,NoSuchToolXyz,LSP,Monitor,TaskStop,Skill", "--allowedTools", "NoSuchToolAbc,mcp__nosuch__*,Bash(echo *)"], hard_timeout=90)
s.control("initialize", hooks=None)
s.user("Reply with the single word: pong")
print("result:", s.wait_results(1, 60))
init = s.wait_for(lambda e: e.get("type") == "system" and e.get("subtype") == "init", 5)
print("init.tools:", init and init["tools"])
s.control("list_permission_rules")
print("cost:", s.cost(), "exit:", s.close())
