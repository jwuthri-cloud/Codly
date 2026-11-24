"""Rules of sanitize.py that a fixture would otherwise leak. Run: python3 -m unittest test_sanitize (in this folder)."""
import unittest

import sanitize


def clean(msg):
    ids, sig = sanitize.Ids(), sanitize.Ids()
    return sanitize.transform(msg, ids, sig)


class Sanitize(unittest.TestCase):
    def test_host_name_is_replaced(self):
        self.addCleanup(setattr, sanitize, "HOSTNAMES", sanitize.HOSTNAMES)
        sanitize.HOSTNAMES = ["Someones-MacBook-Pro.local", "Someones-MacBook-Pro"]
        line = '{"serverName":"Someones-MacBook-Pro.local","note":"on Someones-MacBook-Pro"}'
        out = sanitize.scrub(line, sanitize.Ids(), None)
        self.assertNotIn("Someones", out)
        self.assertEqual(out, '{"serverName":"my-mac","note":"on my-mac"}')

    def test_machine_session_start_hook_output_is_removed(self):
        msg = {"type": "system", "subtype": "hook_response", "hook_event": "SessionStart",
               "output": "PRIVATE RULES", "stdout": "PRIVATE RULES", "stderr": "", "exit_code": 0}
        out = clean(msg)
        self.assertEqual(out["output"], sanitize.HOOK_GONE)
        self.assertEqual(out["stdout"], sanitize.HOOK_GONE)
        self.assertEqual(out["stderr"], "")

    def test_scenario_hook_output_is_kept(self):
        msg = {"type": "system", "subtype": "hook_response", "hook_event": "PreToolUse", "output": "pre-tool-hook-ran\n"}
        self.assertEqual(clean(msg)["output"], "pre-tool-hook-ran\n")

    def test_context_report_lists_no_skill_or_plugin_agent(self):
        resp = {"categories": [], "skills": {"totalSkills": 2, "skillFrontmatter": [{"name": "private-skill", "path": "/x"}]},
                "agents": [{"agentType": "acme:helper", "source": "plugin"}, {"agentType": "Explore", "source": "built-in"}],
                "mcpTools": []}
        out = clean(resp)
        self.assertEqual(out["skills"], {"totalSkills": 2, "skillFrontmatter": []})
        self.assertEqual(out["agents"], [{"agentType": "Explore", "source": "built-in"}])

    def test_init_lists_are_emptied(self):
        init = {"type": "system", "subtype": "init", "skills": ["private-skill"], "plugins": [{"name": "p", "path": "/x"}],
                "agents": ["general-purpose", "acme:helper"], "slash_commands": ["clear", "private-command"],
                "mcp_servers": [{"name": "my-server"}, {"name": "ping-user"}], "tools": ["Bash", "mcp__my-server__x", "mcp__ping-user__ping"]}
        out = clean(init)
        self.assertEqual((out["skills"], out["plugins"], out["agents"]), ([], [], ["general-purpose"]))
        self.assertEqual(out["slash_commands"], ["clear"])
        self.assertEqual(out["mcp_servers"], [{"name": "ping-user"}])
        self.assertEqual(out["tools"], ["Bash", "mcp__ping-user__ping"])

    def test_init_keeps_only_the_scenarios_plugin(self):
        init = {"type": "system", "subtype": "init", "plugins": [{"name": "ping-plugin", "source": "ping-plugin@codly-test-market"},
                                                                 {"name": "my-own-plugin", "path": "/x"}, "a-plain-name"],
                "mcp_servers": [{"name": "plugin:ping-plugin:ping-plugged"}, {"name": "plugin:my-own-plugin:srv"}],
                "tools": ["mcp__plugin_ping-plugin_ping-plugged__ping", "mcp__plugin_my-own-plugin_srv__x"]}
        out = clean(init)
        self.assertEqual(out["plugins"], [{"name": "ping-plugin", "source": "ping-plugin@codly-test-market"}])
        self.assertEqual(out["mcp_servers"], [{"name": "plugin:ping-plugin:ping-plugged"}])
        self.assertEqual(out["tools"], ["mcp__plugin_ping-plugin_ping-plugged__ping"])

    def test_machine_made_file_names_are_replaced(self):
        note = ('{"files_changed":["h/.claude.json","h/.claude/backups/.claude.json.backup.1795472505724",'
                '"h/.claude/sessions/13447.' + "cb" * 32 + '.key","h/.claude/sessions/13447.json",'
                '"h/Library/Caches/claude-cli-nodejs/-tmp-work-x/mcp-logs-ping-live/2026-11-23T22-21-45-801Z.jsonl","x/started.txt"]}')
        out = sanitize.machine_names(note)
        self.assertEqual(out, '{"files_changed":["h/.claude.json","h/.claude/backups/.claude.json.backup.<time>",'
                              '"h/.claude/sessions/<pid>.<hash>.key","h/.claude/sessions/<pid>.json",'
                              '"h/Library/Caches/claude-cli-nodejs/-tmp-work-x/mcp-logs-ping-live/<time>.jsonl","x/started.txt"]}')
        self.assertEqual(sanitize.scrub(note, sanitize.Ids(), None), out)


if __name__ == "__main__":
    unittest.main()
