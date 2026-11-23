"""Scenario 8: --mcp-config with one stdio server that exits at once and one whose command does not exist."""
import json, os, time
from drv import Session, OUT
cfg = os.path.join(OUT, "work", "broken-mcp.json")
os.makedirs(os.path.dirname(cfg), exist_ok=True)
with open(cfg, "w") as f:
    json.dump({"mcpServers": {
        "exits-at-once": {"type": "stdio", "command": "/usr/bin/false", "args": []},
        "no-such-command": {"type": "stdio", "command": "/nonexistent/bin/mcp-server-xyz", "args": []},
    }}, f)
s = Session("broken-mcp", extra=["--tools", "Bash", "--mcp-config", cfg], hard_timeout=120)
r = s.control("mcp_status")
s.user("Reply with the single word: pong")
print("result:", s.wait_results(1, 60))
s.control("mcp_status")
s.control("mcp_reconnect", serverName="exits-at-once", wait=30)
s.control("mcp_status")
s.control("mcp_toggle", serverName="no-such-command", enabled=False)
s.control("mcp_status")
# The switch is saved in ~/.claude.json under the work folder (part 5, section 1): turn the server on again so that the
# folder's `disabledMcpServers` ends empty (a failed "on" still empties it: part 5, section 9).
s.control("mcp_toggle", serverName="no-such-command", enabled=True)
time.sleep(1)
print("exit:", s.close())
