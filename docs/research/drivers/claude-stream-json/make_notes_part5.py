"""Builds the part-5 document from the sanitized r7-* fixtures, so every JSON line quoted there is copied from a fixture.

python3 make_notes_part5.py [output file]      (default: $CAPTURE_DIR/NOTES-part5.md)

Numbers and words that the text states about a fixture are checked against it (`check`): the build fails when a
recording no longer says what the text says. The section "Notes of part 4 written late" reads the r6-* fixtures.
"""
import json
import os
import sys

from notes_lib import OUT, block, check, cost, flags, fx, is_cli, j, kind, meta, quiet, reqs, results, sub, tl

P = []
w = P.append

FIX = os.environ.get("FIXTURES_DIR") or os.path.join(OUT, "fixtures")
ALL = sorted(f[:-len(".ndjson")] for f in os.listdir(FIX) if f.startswith("r7-"))
for n in ALL:
    check("claude 2.1.311, recorded 2026-11-23." in fx(n)[0]["_note"], "the version and the day of recording of " + n)


def note(name, key):
    """The one driver note of a fixture that has this key: the value of the key."""
    found = meta(name, key)
    check(len(found) == 1, "one note %r in %s" % (key, name))
    return found[0][key]


def notes(name, key, value=None):
    """Every driver note with this key (and this value of it, when given), whole."""
    return [m for m in meta(name, key) if value is None or m[key] == value]


def stamp(name, key):
    found = [l["_t"] for l in fx(name) if "_meta" in l and key in l["_meta"]]
    check(len(found) == 1, "one note %r in %s" % (key, name))
    return found[0]


def from_note(key):
    return lambda l: "_meta" in l and key in l["_meta"]


def is_sent(subtype=None, kind_=None):
    return lambda l: "_sent" in l and (kind_ is None or l["_sent"].get("type") == kind_) and (
        subtype is None or l["_sent"].get("request", {}).get("subtype") == subtype)


def pairs(name, subtype):
    """[(request, answer)] of the host's control requests of that subtype, in order."""
    lines, out = fx(name), []
    for i, l in enumerate(lines):
        if is_sent(subtype)(l):
            rid = l["_sent"]["request_id"]
            ans = next((m for m in lines[i + 1:] if m.get("type") == "control_response" and m["response"].get("request_id") == rid), None)
            out.append((l, ans))
    return out


def qpair(name, subtype, i, **kw):
    req, ans = pairs(name, subtype)[i]
    return [j(req["_sent"], **kw), j(ans, **kw)]


def both(names):
    return "`%s`" % "`, `".join(names)


def secs(values, fmt="%.2f"):
    return ", ".join(fmt % v for v in values)


def task_or_list(l):
    return sub(l) in ("task_started", "task_updated", "task_notification", "background_tasks_changed")


def tool_results_of(name, helper=None):
    out = []
    for l in fx(name):
        if l.get("type") == "user" and is_cli(l) and isinstance(l["message"]["content"], list) and (helper is None or bool(l.get("parent_tool_use_id")) == helper):
            out += [b for b in l["message"]["content"] if isinstance(b, dict) and b.get("type") == "tool_result"]
    return out


ALLCOST = cost(ALL)
w("""# Claude Code CLI stream-json: recorded protocol shapes, part 5

- CLI: `claude` 2.1.311 (the same version as parts 1 to 4; checked with `claude --version` before the runs).
- Recorded: 2026-11-23, macOS, `--model haiku` (`claude-haiku-5-5`), subscription login (`"apiKeySource":"none"`).
- Method, base command, fixture line format and sanitizing: as in part 1 ("Base command", "Files", "Sanitizing") with
  the additions of parts 2 to 4. Fixtures of this part are `fixtures/claude-2.1.311/r7-*.ndjson`; the scripts are
  `drivers/claude-stream-json/r7_*.py` (README there says how to record again).
- List price of the %d recordings (sum of `total_cost_usd` of their `result` lines, per session): %.4f USD. The runs
  in a throw-away HOME have no login and cost nothing.
- Everything below was observed in these recordings. Each answer says how many runs it rests on (n) and says no more
  than the recordings show.
- The sections are numbered as the 12 questions this part was recorded for; the last two tables have them in the same
  order.

Differences from the part 4 method:
- Times are time stamps of the recording, as in part 4. A note that lists lines is now written under the driver's lock
  in the same step as the list, and the process checks are made before that step (`r7_a5`, and `r6_a2` fixed the same
  way): no line can come between a list and its note's stamp. Part 4 had notes written late: see "Notes of part 4
  written late".
- EVERY RUN THAT SWITCHES AN MCP SERVER uses a throw-away HOME: section 1 shows that Claude Code saves the switch in
  `~/.claude.json`. (Earlier parts switched servers with the real HOME: `r4-mcp-needs-auth`, `r5-mcp-reconnect-toggle`.
  Both turned their server on again before they ended, which empties the saved list again: section 1.)
- Two recordings of section 2 run with the REAL login and the user's own settings (`--setting-sources user…`, no
  `--strict-mcp-config`), because that is the question. Same safety as part 4: `"disableAllHooks":true` in
  `--settings`, the claude.ai connectors off, one tool allowed and every other request denied, no `mcp_status`; the
  sanitizer removes the user's own servers and names the output style with a placeholder.
- Section 11 also counts, from the session's own transcript file that Claude Code writes under `~/.claude/projects/`,
  the task notifications that waited in its queue and those it handed to a running turn (`r7_lib.queued_notifications`:
  counts only, the transcript itself is never copied).
- Section 10 splits a run's cost into its model requests: each request of the main thread at the list prices of
  `claude-haiku-5-5` from its final usage (the `message_delta` stream event), the helper's share as the rest of
  `total_cost_usd` (the CLI streams no events of a helper). The prices are the ones that give the `costUSD` of every
  `model_usage_since_last_result` of these recordings to the last digit.
- New sanitizing: the MCP servers `ping-http` and `ping-local` are kept in `init`, `mcp_status` and `can_use_tool`.
- In quoted lines `"…":"…"` stands for keys left out; `# t=` lines are seconds since the process started. A quoted
  line of a subagent keeps `parent_tool_use_id` and `agent_id`.
""" % (len(ALL), ALLCOST))

# ================================================================= 1
w("## 1. `mcp_toggle` off, the process exits, a NEW process in the same folder\n")
w("Question: what does `mcp_status` say about that server in a new process (fresh, and with `--resume`)? Is the switch saved, and where?\n")
N1 = {lab: ["r7-mcp-toggle-saved-%s-%s" % (lab, p) for p in "abc"] for lab in "12"}
w("Fixtures: %s (label 1), %s (label 2). A THROW-AWAY HOME (no login: the one message of each process ends at once with "
  "`Not logged in`). Each process: `ping-live` with `--mcp-config` (one line in `started.txt` per start), flags %s. Process a: "
  "`mcp_status`, `mcp_toggle` off, `mcp_status`, one message, stdin closed. Process b: a NEW session in the same folder: "
  "`mcp_status`, one message. Process c: `--resume` of a's session in the same folder: `mcp_status`, then `mcp_toggle` on, "
  "`mcp_status`, one message. After each step a note names the files under the throw-away HOME and the work folder that "
  "changed, and the folder's entry in `HOME/.claude.json` (its MCP keys).\n" % (both(N1["1"]), both(N1["2"]), flags(N1["1"][0])))
a, b, c = N1["1"]
w("Process a, the switch and the note after it:\n")
w(block(qpair(a, "mcp_toggle", 0) + [j(notes(a, "after", "mcp_toggle off")[0], maxlist=9)] + qpair(a, "mcp_status", 1)))
w("Process b (a new session), `mcp_status` and the `init` line of its one message:\n")
w(block(qpair(b, "mcp_status", 0) + tl(b, lambda l: sub(l) == "init")))
w("Process c (`--resume` of a's session): `mcp_status`, the switch back on, the note after it:\n")
w(block(qpair(c, "mcp_status", 0) + qpair(c, "mcp_toggle", 0) + [j(notes(c, "after", "mcp_toggle on")[0], maxlist=9)]))
for lab, (a, b, c) in N1.items():
    off = notes(a, "after", "mcp_toggle off")[0]
    check(off["files_changed"] == ["r7-mcp-toggle-saved-%s-home/.claude.json" % lab], "the switch writes HOME/.claude.json alone in " + a)
    check(off["project_entry_mcp_keys"]["disabledMcpServers"] == ["ping-live"], "disabledMcpServers after the switch in " + a)
    check([x["status"] for x in pairs(a, "mcp_status")[1][1]["response"]["response"]["mcpServers"]] == ["disabled"], "disabled in a, " + lab)
    for p in (b, c):
        st = pairs(p, "mcp_status")[0][1]["response"]["response"]["mcpServers"]
        check([(x["name"], x["status"], x["source"]) for x in st] == [("ping-live", "disabled", "dynamic")], "disabled at the start of " + p)
        check(notes(p, "after", "start + 3 s")[0]["server_starts_so_far"] == 1, "the server was not started in " + p)
        check(notes(p, "after", "start + 3 s")[0]["project_entry_mcp_keys"]["disabledMcpServers"] == ["ping-live"], "still saved at the start of " + p)
    check(note(b, "init_mcp_servers") == [{"name": "ping-live", "status": "disabled", "source": "dynamic"}] and not note(b, "tool_in_init"), "b's init")
    check(fx(c)[0]["_argv"][fx(c)[0]["_argv"].index("--resume") + 1] == "sess-1" and all(r["session_id"] == "sess-1" for r in results(c)), "c resumed a's session")
    on = notes(c, "after", "mcp_toggle on")[0]
    check(on["project_entry_mcp_keys"]["disabledMcpServers"] == [] and on["server_starts_so_far"] == 2, "the switch on in " + c)
    check(note(c, "init_mcp_servers") == [{"name": "ping-live", "status": "connected", "source": "dynamic"}], "c's init after the switch on")
ENTRY_KEYS = sorted(notes(N1["1"][0], "after", "mcp_toggle off")[0]["project_entry_mcp_keys"])
w("""What the adapter can rely on (n=2, the two labels agree step for step):
- THE SWITCH IS SAVED. `mcp_toggle` off writes ONE file: `HOME/.claude.json` (the throw-away HOME's; for a real run,
  the user's own `~/.claude.json`). The folder's entry there (under `projects`, keyed by the folder's path) gets
  `"disabledMcpServers":["ping-live"]`. The entry's MCP keys: %s.
- It holds for EVERY later process in that folder that reads that HOME: a new session (b) and a resumed one (c) both
  report the server `disabled` in `mcp_status` and in `init.mcp_servers` (`"source":"dynamic"`), its tool is not in
  `init.tools`, and the server is NOT started (no new line in `started.txt`). The saved list is read with
  `--strict-mcp-config --setting-sources local` (all six processes; other setting sources were not tried).
- The switch goes by the server's NAME in that folder: the server came from `--mcp-config` (Codly's way), so any
  server of that name in that folder stays off, also one that Codly passes in a later run.
- Only `mcp_toggle` on undoes it: the list is `[]` again in the same step, and the server starts (`connected`).
- So "Turn off" in the app is not "for this run": it is for this folder, for every later Claude Code run there under
  the same HOME. Expected, not recorded: the user's own `claude` sessions in that folder read the same entry (for a
  server of the same name), and a worktree, being another folder, has its own entry.
""" % ", ".join("`%s`" % k for k in ENTRY_KEYS))

# ================================================================= 2
w("## 2. Your own MCP servers in a folder you have not trusted, a second time\n")
w("Question: without `--strict-mcp-config`, with `--setting-sources user` or `user,local`: is the repo's `.mcp.json` server loaded? "
  "Also with a local-scope server of the folder and with an approval of the project's servers kept in `~/.claude.json`.\n")
V2 = ["user-login", "user-local-login", "local-scope-user", "local-scope-user-local", "approval-user", "approval-user-local"]
N2 = {v: [n for n in ALL if n.startswith("r7-untrusted-mcp-%s-" % v) and n[len("r7-untrusted-mcp-%s-" % v):].isdigit()] for v in V2}
w("Fixtures: `r7-untrusted-mcp-<variant>-<label>`. The work folder's `.mcp.json` defines `ping-project` (one line in `started.txt` per start). "
  "No run passes `--strict-mcp-config`. The two `-login` variants use the REAL login and your own user settings (safety as described above, as "
  "in part 4); the others a THROW-AWAY HOME (no login: they show what is LOADED). Flags of `user-login`: %s.\n" % flags(N2["user-login"][0]))
w("- `local-scope-*`: in the throw-away HOME, `claude mcp add --scope local ping-local -- python3 mcp_ping_server.py started-local.txt label=local` "
  "was run in the folder first (the CLI's own command writes the server into `HOME/.claude.json` under the folder's path).\n"
  "- `approval-*`: the CLI first wrote the folder's entry (`claude mcp add --scope local`, then `remove`: the entry stays), and the script "
  "set two of its keys: `\"enabledMcpjsonServers\":[\"ping-project\"]`, `\"hasTrustDialogAccepted\":true` (an approval of the repo's server "
  "kept there). It added no key.\n")
w("The set-up note and the `init` line of `local-scope-user-local-1`:\n")
w(block([j(note("r7-untrusted-mcp-local-scope-user-local-1", "setup"), maxlist=9)] + tl("r7-untrusted-mcp-local-scope-user-local-1", lambda l: sub(l) == "init")))
w("The set-up note of `approval-user-1`:\n")
w(block([j(note("r7-untrusted-mcp-approval-user-1", "setup"), maxlist=9)]))
w("| recording | login | `--setting-sources` | what the folder or HOME has | `ping-project` started | `ping-local` started | scenario servers in `init` | your own servers in `init` |")
w("|---|---|---|---|---|---|---|---|")
HAS = {"user-login": "`.mcp.json` only", "user-local-login": "`.mcp.json`; `.claude/settings.local.json` turns the project's servers on",
       "local-scope-user": "`.mcp.json`; a local-scope `ping-local` in `HOME/.claude.json`", "local-scope-user-local": "the same",
       "approval-user": "`.mcp.json`; the approval in `HOME/.claude.json`", "approval-user-local": "the same"}
COUNT2 = 0
for v in V2:
    for n in N2[v]:
        m = meta(n, "server_starts")[0]
        src = fx(n)[0]["_argv"][fx(n)[0]["_argv"].index("--setting-sources") + 1]
        check("--strict-mcp-config" not in fx(n)[0]["_argv"], "no --strict-mcp-config in " + n)
        check(m["server_starts"]["project"] == 0 and not any(x["name"] == "ping-project" for x in m["init_entries_for_the_scenario_servers"]), "ping-project not loaded in " + n)
        local = v == "local-scope-user-local"
        check(m["server_starts"]["local"] == (1 if local else 0), "ping-local starts in " + n)
        check(m["init_entries_for_the_scenario_servers"] == ([{"name": "ping-local", "status": "connected", "source": "local"}] if local else []), "init entries of " + n)
        check(m["servers_of_the_users_own_setup_in_init"] == v.endswith("login"), "the user's own servers in " + n)
        check((m["result"] == "Not logged in · Please run /login") != v.endswith("login"), "login or not in " + n)
        COUNT2 += 1
        w("| `%s` | %s | `%s` | %s | %d | %d | %s | %s |" % (n, "real" if v.endswith("login") else "none (throw-away HOME)", src, HAS[v], m["server_starts"]["project"],
                                                          m["server_starts"]["local"], ", ".join("`%s` (%s)" % (x["name"], x["source"]) for x in m["init_entries_for_the_scenario_servers"]) or "none",
                                                          "yes" if m["servers_of_the_users_own_setup_in_init"] else "no"))
check(note("r7-untrusted-mcp-approval-user-1", "setup")["project_entry_in_home_claude_json"]["enabledMcpjsonServers"] == ["ping-project"], "the approval was set")
P4_ROWS = ["r6-project-mcp-user-only", "r6-project-mcp-user-only-home", "r6-project-mcp-user-local-home", "r6-project-mcp-user-local-file",
           "r6-project-mcp-user-local-file-home-1", "r6-project-mcp-user-local-file-home-2"]
for n in P4_ROWS:
    check(meta(n, "server_starts")[0]["server_starts"] == 0, "part 4 row " + n)
w("""
What the adapter can rely on:
- The repo's `.mcp.json` server was loaded in NONE of these %d recordings (no start, no `init` entry, no tool), nor in
  the %d recordings of the same kind in part 4 (`%s`). Logged in: n=2 for `user` (part 4 `user-only`, here
  `user-login-1`) and n=2 for `user,local` with a local file that turns the project's servers on (part 4
  `user-local-file`, here `user-local-login-1`).
- An approval of the project's server kept in `~/.claude.json` under the folder's path does not load it either, with
  `user` or with `user,local` (n=2 each, no login). Whether these keys count when `project` is a source was not
  recorded (an untrusted start does not give that source).
- A LOCAL-SCOPE server (the user's own, added with `claude mcp add --scope local`, kept in `~/.claude.json` under the
  folder's path) is loaded only with `local` among the sources: with `user,local` it starts and its entry says
  `"source":"local"`; with `user` alone it is not loaded (n=2 each, no login). It is the user's own server, not the
  repo's: the repo cannot add it.
- With your own setup, your own servers are loaded (the two logged-in runs: yes; the sanitizer removed them).
""" % (COUNT2, len(P4_ROWS), "`, `".join(P4_ROWS)))

# ================================================================= 3
w("## 3. A turn that FAILS by itself while a background helper works\n")
w("Question: does the helper go on, what does the list say, what ends it?\n")
N3 = ["r7-failed-turn-helper-1", "r7-failed-turn-helper-2"]
w("Fixtures: %s. Flags: %s. Turn 1 starts a helper in the background (its Bash call: `echo begin; sleep 41; echo end`) and "
  "ends within the limit (two model requests). Turn 2 asks for three Bash calls one after the other: its third model request "
  "passes `--max-turns 2`.\n" % (both(N3), flags(N3[0])))
w("From turn 2's message on: the task and list lines, the turns' `init` and `result` lines, the driver's notes (first recording):\n")
w(block(tl(N3[0], lambda l: task_or_list(l) or sub(l) == "init" or kind(l) in ("result", "meta") or is_sent(kind_="user")(l),
           start=lambda l: is_sent(kind_="user")(l) and "echo one" in json.dumps(l), text=100, maxlist=4)))
for n in N3:
    r = note(n, "right_after_the_failed_turn")
    check(r["result_subtype"] == "error_max_turns" and r["terminal_reason"] == "max_turns", "turn 2 fails by the limit in " + n)
    check(len(r["list"]) == 1 and r["list"][0]["task_type"] == "local_agent" and r["sleep_41_alive"], "the helper goes on in " + n)
    check(not any(task_or_list(l) for l in fx(n)[[i for i, l in enumerate(fx(n)) if is_sent(kind_="user")(l)][1]:] if l.get("type") != "result" and
                  fx(n).index(l) < [i for i, x in enumerate(fx(n)) if from_note("right_after_the_failed_turn")(x)][0] and sub(l) in ("task_updated", "background_tasks_changed")),
          "no task_updated or list line during the failed turn in " + n)
    e = note(n, "after_it")
    check(e["helper_end"] == "completed" and e["turn_by_itself"] and e["its_results"] == [["success", {"kind": "task-notification", "producer": "session-task", "runId": "run-1"}]]
          and e["list"] == [] and not e["sleep_41_alive"], "the helper ends by itself in " + n)
w("""What the adapter can rely on (n=2, the two recordings agree line for line):
- A turn that ends with an error by itself (here `error_max_turns`, `terminal_reason:"max_turns"`) does NOT end a
  background helper. No task line and no list line comes with the failed turn; right after its `result` the list still
  names the helper and its `sleep 41` runs.
- The helper ends later by itself, as without the failure: `task_updated` `completed`, `task_notification` `completed`
  with its report in `summary`, the list `[]`, and then the CLI starts a turn by itself (`result` with
  `origin.kind:"task-notification"`).
- So the end of a turn says nothing about background helpers, whatever its status: interrupted (part 4: the stream
  ends every helper itself), failed (here: they go on) or done. A host that ends helpers only at their own
  `task_notification` is right in all three; a host that ends them at a failed turn is wrong.
- Not recorded: a turn that fails by an API error or by the budget while a helper works (the budget: section 10, where
  the helper had already finished).
""")

# ================================================================= 4
w("## 4. An `interrupt` with TWO helpers at work, and with a helper that waits for its model answer\n")
w("Question: are all ended, with which lines? (Part 4 recorded one helper, always inside a Bash call.)\n")
N4T = ["r7-interrupt-two-helpers-1", "r7-interrupt-two-helpers-2"]
N4M = ["r7-interrupt-helper-model-1", "r7-interrupt-helper-model-2"]
w("Fixtures: %s (two background helpers, each in a Bash call: `echo a; sleep 181; echo end` and `echo b; sleep 191; echo end`), "
  "%s (one background helper that writes a story with no tool). Flags: %s. No turn of the main agent runs when the interrupt comes. "
  "The CLI streams no partial lines of a helper: the `model` variant sends the interrupt 1.5 s after the result of turn 1, and its note "
  "lists the block types of the helper's `assistant` lines so far.\n" % (both(N4T), both(N4M), flags(N4T[0])))
w("Two helpers: every line from the driver's note before the interrupt to the note after it (first recording):\n")
w(block(tl(N4T[0], lambda l: quiet(l) and sub(l) not in ("status", "task_progress"), start=from_note("before_the_interrupt"), stop=from_note("after_the_interrupt"),
           text=100, maxlist=4)))
w("A helper that waits for its model answer (first recording):\n")
w(block(tl(N4M[0], lambda l: quiet(l) and sub(l) not in ("status", "task_progress"), start=from_note("before_the_interrupt"), stop=from_note("after_the_interrupt"),
           text=100, maxlist=4)))
T4 = []
for n in N4T + N4M:
    two = n in N4T
    bi, ai = note(n, "before_the_interrupt"), note(n, "after_the_interrupt")
    check(not bi["a_turn_runs"] and bi["helpers"] == (2 if two else 1) and len(bi["list"]) == bi["helpers"], "the helpers at work before the interrupt in " + n)
    if two:
        check(bi["sleep_181_alive"] and bi["sleep_191_alive"], "both sleeps ran in " + n)
    else:
        check(not any("text" in blocks for blocks in bi["assistant_lines_of_helpers"]), "the helper had not written its answer in " + n)
    check(ai["helpers_ended"] == bi["helpers"] and all(x == ["killed", "stopped"] for x in ai["end_words_per_helper"]), "every helper ended in " + n)
    check(ai["list"] == [] and ai["results_in_the_10s_after"] == 0 and not ai["sleep_181_alive"] and not ai["sleep_191_alive"], "nothing left in " + n)
    answer_at = ai["lines"].index("control_response success")
    check(all(x.startswith("system/task_") or x.startswith("system/background") for x in ai["lines"][:answer_at]), "the helpers' task lines come before the answer in " + n)
    check(ai["lines"][:answer_at].count("system/task_updated \"killed\"") == bi["helpers"], "one task_updated per helper before the answer in " + n)
    check([x for x in ai["lines"][:answer_at] if x.startswith("system/background")] ==
          ["system/background_tasks_changed %d listed" % k for k in range(bi["helpers"] - 1, -1, -1)], "the list shrinks one by one in " + n)
    t = meta(n, "after_the_interrupt")[0]["seconds_from_the_interrupt_request"]
    check(t["next_init"] is None, "no turn by itself in " + n)
    T4.append(t["answer"])
    if not two:
        texts = [b["text"] for l in fx(n) if l.get("type") == "user" and l.get("parent_tool_use_id") for b in l["message"]["content"] if isinstance(b, dict) and b.get("type") == "text"]
        check(texts == ["[Request interrupted by user]"], "the helper's one user line in " + n)
w("""What the adapter can rely on (n=2 per variant):
- TWO helpers: the interrupt ends both. For each: `task_updated` (`patch.status:"killed"`), `task_notification`
  (`"status":"stopped"`) and a list line (the list goes from 2 to 1 to 0), all BEFORE the answer to `interrupt`. Then
  each helper's running Bash call (a task of its own when it had run 3 s or more: section 12) gets its own
  `task_notification` `stopped`, and each helper's two `user` lines (the rejected call,
  `[Request interrupted by user for tool use]`) follow.
- A helper that waits for its MODEL answer (no tool running) is ended the same way: `task_updated` `killed`,
  `task_notification` `stopped`, the list `[]`, the answer; then ONE `user` line of the helper:
  `[Request interrupted by user]` (not "for tool use"). Its answer is never written.
- In all four: no `result` (no turn ran), no turn by itself in the 10 s after, the list empty, no `sleep` left. From the
  request to the answer: %s s.
- With part 4 (one helper inside a Bash call, n=8): every helper that was at work when the interrupt came was ended
  by it, in 12 of 12 recordings. Still not recorded: a nested helper, a helper with a background command of its own.
""" % secs(T4, "%.3f"))

# ================================================================= 5
w("## 5. A shutdown with a background helper AND a background command\n")
w("Question: `interrupt`, `stop_task` for the command, stdin closed: the order of the lines, the exit time, anything left running?\n")
N5 = ["r7-shutdown-helper-and-command-1", "r7-shutdown-helper-and-command-2"]
w("Fixtures: %s. Flags: %s. The CLI is the leader of its own process group, as the app starts it. Turn 1 starts a background "
  "command (`echo bg; sleep 197`) and a background helper (its Bash call: `echo begin; sleep 199; echo end`) and ends; the host waits "
  "until it sees both `sleep` processes, and 2 s more.\n" % (both(N5), flags(N5[0])))
w("Every line from the driver's note before the stop (first recording):\n")
w(block(tl(N5[0], lambda l: quiet(l) and sub(l) not in ("status", "task_progress"), start=from_note("before_the_stop"), text=100, maxlist=4)))
E5 = []
for n in N5:
    bs, bc, ae = note(n, "before_the_stop"), note(n, "before_stdin_is_closed"), note(n, "after_the_exit")
    check(not bs["a_turn_runs"] and [x["task_type"] for x in bs["list"]] == ["local_agent", "local_bash"] and bs["sleep_197_alive"] and bs["sleep_199_alive"], "both at work in " + n)
    check(bc["lines_since_the_interrupt"][:8] == ["system/task_updated \"killed\"", "system/task_notification \"stopped\"", "system/background_tasks_changed 1 listed",
                                                 "control_response success", "system/task_updated \"killed\"", "system/task_notification \"stopped\"",
                                                 "system/background_tasks_changed 0 listed", "control_response success"], "the order of the lines in " + n)
    check(bc["list"] == [] and not bc["sleep_197_alive"] and not bc["sleep_199_alive"], "nothing runs before the close in " + n)
    check(ae["lines_after_stdin_was_closed"] == [] and ae["processes_left_in_the_group"] == [] and not ae["sleep_197_alive"] and not ae["sleep_199_alive"], "nothing left in " + n)
    E5.append((stamp(n, "process_exited") - stamp(n, "closing_stdin"), note(n, "exit_code")))
check(all(c == 0 for _, c in E5), "exit code 0")
w("""What the adapter can rely on (n=2, the two recordings have the same lines in the same order):
- `interrupt` with no turn running ends the HELPER: `task_updated` `killed`, `task_notification` `stopped`, a list line
  that still names the COMMAND (1 listed), then the answer. The command is not touched by the interrupt (part 3,
  section 4, already showed that).
- `stop_task` with the command's id then ends the COMMAND: `task_updated` `killed`, `task_notification` `stopped`, the
  list `[]`, the answer `{}`. After it come the helper's two `user` lines (its rejected Bash call).
- No turn starts by itself, no `result` is written. After stdin close the process was gone in %s s, exit code 0
  (no turn had failed). After the exit no `sleep` of the scenario was alive and the CLI's process group was empty.
- So the shutdown order of part 4 holds with both kinds at once: `interrupt` (also with no turn running), then
  `stop_task` for each command of the list, then close stdin. A helper needs no `stop_task`.
""" % " / ".join("%.2f" % e for e, _ in E5))

# ================================================================= 6
w("## 6. `tool_progress` for a tool of an HTTP MCP server\n")
w("Question: is there a heartbeat line every 30 s, as for a stdio server (part 3, section 3)?\n")
N6 = ["r7-tool-progress-http-70-1", "r7-tool-progress-http-70-2"]
N6C = "r7-tool-progress-http-45-1"
w("Fixtures: %s (the tool waits 70 s), `%s` (control: 45 s). The server `ping-http` runs in the driver (Streamable HTTP on 127.0.0.1, "
  "JSON answers, no event stream: GET gets 405) and comes with `--mcp-config` as type `http`. Flags: %s.\n" % (both(N6), N6C, flags(N6[0])))
w("The call, its request, every `tool_progress` line, the tool result (first recording of 70 s):\n")
w(block(tl(N6[0], lambda l: l.get("type") in ("tool_progress",) or (l.get("type") == "assistant" and any(b["type"] == "tool_use" for b in l["message"]["content"]))
           or (l.get("type") == "control_request") or (l.get("type") == "user" and is_cli(l)), brief=False)))
w("The control of 45 s: the `tool_progress` line and the result:\n")
w(block(tl(N6C, lambda l: l.get("type") == "tool_progress" or (l.get("type") == "user" and is_cli(l)), brief=True)))
w("What the server got (first recording of 70 s; times are seconds since the server started, a moment before the CLI):\n")
w(block([j(note(N6[0], "server_requests"), maxlist=9)]))
for n in N6 + [N6C]:
    tp = note(n, "tool_progress_lines")
    long_ = n in N6
    check([x["elapsed_time_seconds"] for x in tp] == ([30, 60] if long_ else [30]), "heartbeats of " + n)
    check(all(x["tool_name"] == "mcp__ping-http__ping" and x["parent_tool_use_id"] == "toolu-1" and x["heartbeat"] for x in tp), "the heartbeat line of " + n)
    res = tool_results_of(n)
    check(len(res) == 1 and (res[0]["is_error"] and res[0]["content"] == "The operation timed out." if long_ else
                             not res[0].get("is_error") and "pong after 45 s" in json.dumps(res[0]["content"])), "the tool result of " + n)
    took = note(n, "seconds_from_tool_use_to_tool_result")
    check((60 <= took < 61) if long_ else (45 <= took < 46), "how long the call took in " + n)
    rq = [r for r in fx(n) if r.get("type") == "control_request"]
    check(len(rq) == 1 and rq[0]["request"]["mcp_server"] == {"name": "ping-http", "source": "dynamic"}, "the request of " + n)
T6 = [note(n, "seconds_from_tool_use_to_tool_result") for n in N6]
w("""What the adapter can rely on:
- Yes: the same heartbeat line as for a stdio server (`tool_use_id` `<call id>-heartbeat-N`, `parent_tool_use_id` =
  the call id, `tool_name` = the tool, `"heartbeat":true`) at 30 s and 60 s (n=2), at 30 s for the 45 s call (n=1).
- NEW: an HTTP MCP tool call does not outlast 60 s. At %s s after its `tool_use` line (n=2) the call ended with
  the error tool result `The operation timed out.` (`tool_use_result` `"Error: The operation timed out."`), while
  the server was still working on it; the turn went on (the model got the error). The 45 s call returned its answer.
  The stdio tool of part 3 ran 100 s and returned. Recorded with a server that answers with a JSON body; a server that
  streams its answer (an event stream) is not recorded.
- The approval request of an HTTP server's tool has `"mcp_server":{"name":"ping-http","source":"dynamic"}`, as for a
  stdio server of `--mcp-config`.
""" % secs(T6, "%.3f"))

# ================================================================= 7
w("## 7. An allow answer with the request's own `setMode` suggestion\n")
w("Question: an allow that carries `{\"type\":\"setMode\",\"mode\":\"acceptEdits\",\"destination\":\"session\"}` in `updatedPermissions`: what does "
  "`get_settings` say after it, does the next Edit ask, does the stream say that the mode changed?\n")
N7 = ["r7-allow-set-mode-1", "r7-allow-set-mode-2"]
w("Fixtures: %s. Flags: %s. Turn 1 asks for one Edit of `hello.txt`; the first Edit request is answered `allow` with `updatedPermissions` = "
  "the request's own `setMode` suggestion, copied unchanged. Turn 2 asks for a second Edit.\n" % (both(N7), flags(N7[0])))
w("The request, the driver's answer, the status line after it (first recording):\n")
w(block(reqs(N7[0]) + tl(N7[0], lambda l: sub(l) == "status" and "permissionMode" in l)))
w("The driver's notes (turns and `get_settings`) and the `init` line of turn 2:\n")
w(block([j(m, maxlist=9) for m in meta(N7[0], "turn") + meta(N7[0], "step")] + tl(N7[0], lambda l: sub(l) == "init")[2:]))
for n in N7:
    rq = [l for l in fx(n) if l.get("type") == "control_request" and l["request"].get("subtype") == "can_use_tool"]
    check(len(rq) == 1 and rq[0]["request"]["permission_suggestions"] == [{"type": "setMode", "mode": "acceptEdits", "destination": "session"}], "one request, its suggestion in " + n)
    ans = [l["_sent"] for l in fx(n) if is_sent(kind_="control_response")(l)]
    check(ans[0]["response"]["response"]["updatedPermissions"] == [{"type": "setMode", "mode": "acceptEdits", "destination": "session"}], "the answer of " + n)
    t1, t2 = notes(n, "turn", "1")[0], notes(n, "turn", "2")[0]
    check(t1["init_permissionMode"] == "default" and t1["status_lines_with_permissionMode"] == ["acceptEdits"], "turn 1 of " + n)
    check(t2["init_permissionMode"] == "acceptEdits" and t2["requests"] == [] and t2["ok"], "turn 2 of " + n)
    check(all(m["effective_permissions"] is None and "permissionMode" not in json.dumps(m["applied"]) for m in meta(n, "step")), "get_settings has no mode in " + n)
    edits = [b for l in fx(n) if l.get("type") == "assistant" for b in l["message"]["content"] if b["type"] == "tool_use" and b["name"] == "Edit"]
    check(len(edits) == 2 and all(not r.get("is_error") for r in tool_results_of(n) if r["tool_use_id"] == edits[1]["id"]) and
          any(r["tool_use_id"] == edits[1]["id"] for r in tool_results_of(n)), "the second Edit ran in " + n)
w("""What the adapter can rely on (n=2):
- One answer does both: the Edit runs, and the session is in `acceptEdits` from then on.
- The stream says it at once: one line `{"type":"system","subtype":"status","status":null,"permissionMode":"acceptEdits"}`
  after the answer, and the next turn's `init` has `"permissionMode":"acceptEdits"`.
- The next Edit does NOT ask (no `can_use_tool` request in turn 2), and it ran (its tool result is no error).
- `get_settings` does not tell the mode: its answer has no permission mode at all (`effective` `{}`, `sources` `[]`,
  `applied` with model and effort only), before and after. Read the mode from the `status` line or the next `init`.
- The same as part 1 (scenario 10: a `setMode` item among all the suggestions of a Bash request switched the session);
  here it is the Edit request's own and only suggestion.
""")

# ================================================================= 8
w("## 8. `effortLevel: null` on a process started with `--effort low` or `--effort max`\n")
w("Question: what does `get_settings` say after it?\n")
N8 = ["r7-effort-null-low-1", "r7-effort-null-low-2", "r7-effort-null-max-1", "r7-effort-null-max-2"]
w("Fixtures: %s. Flags of the first: %s. Order: `initialize`, `get_settings`, `apply_flag_settings {\"effortLevel\": null}`, `get_settings`, "
  "one turn, `get_settings`.\n" % (both(N8), flags(N8[0])))
w("The request and its answer (first recording):\n")
w(block(qpair(N8[0], "apply_flag_settings", 0)))
w("| recording | `applied.effort` as started | after `null` | after the next turn |")
w("|---|---|---|---|")
for n in N8:
    steps = {m["step"]: m["applied_effort"] for m in meta(n, "step")}
    level = n.split("-")[3]
    check(steps == {"as started": level, "after effortLevel null": "medium", "after the next turn": "medium"}, "the effort steps of " + n)
    check(all(m["effective_effortLevel"] == "(no such key)" and m["sources_that_name_effortLevel"] == [] for m in meta(n, "step")), "no effort in effective or sources in " + n)
    w("| `%s` | `%s` | `%s` | `%s` |" % (n, steps["as started"], steps["after effortLevel null"], steps["after the next turn"]))
w("""
What the adapter can rely on (n=2 per level):
- `null` gives `medium` at once, whatever level the process was started with: `low` -> `medium`, `max` -> `medium`
  (and `high` -> `medium` in part 4, section 4). It stays `medium` through the next turn. `medium` is also what a
  process started with no `--effort` reports (part 3, `r5-effort-started-none`).
- `effective` and `sources` never name an effort level.
- So taking an effort level away needs no restart for any level that was recorded. Still not visible: the level a
  model request carries.
""")

# ================================================================= 9
w("## 9. \"Turn on\" of a server that cannot start; `mcp_reconnect` on a server whose program is gone\n")
w("Question: the answer (success or error, which words) and the status after it.\n")
N9 = ["r7-mcp-cannot-start-1", "r7-mcp-cannot-start-2"]
w("Fixtures: %s. A THROW-AWAY HOME (a switch is saved: section 1). Flags: %s. `ping-flaky` exits at once with code 1 while the file "
  "`switch.on` is missing; `no-such-command` names a program that does not exist. Each step is followed by a note (starts so far, which files "
  "exist, the saved `disabledMcpServers`) and `mcp_status`.\n" % (both(N9), flags(N9[0])))
w("`mcp_toggle` on while the program exits at once, then `mcp_status` (first recording):\n")
w(block(qpair(N9[0], "mcp_toggle", 1) + qpair(N9[0], "mcp_status", 2, maxlist=9)))
w("`mcp_reconnect` after the program file was deleted, and the answers for the program that does not exist:\n")
w(block(qpair(N9[0], "mcp_reconnect", 1) + qpair(N9[0], "mcp_toggle", 4) + qpair(N9[0], "mcp_reconnect", 2)))
STEPS9 = [("start + 3 s", None, None), ("mcp_toggle off", "mcp_toggle", 0), ("mcp_toggle on while the program exits at once", "mcp_toggle", 1),
          ("mcp_reconnect after switch.on was made again", "mcp_reconnect", 0), ("mcp_reconnect after the program file was deleted", "mcp_reconnect", 1),
          ("mcp_toggle off", "mcp_toggle", 2), ("mcp_toggle on while the program file is gone", "mcp_toggle", 3),
          ("mcp_toggle off (no-such-command)", "mcp_toggle", 4), ("mcp_toggle on (no-such-command)", "mcp_toggle", 5),
          ("mcp_reconnect (no-such-command)", "mcp_reconnect", 2)]


def row9(n, i):
    label, subtype, k = STEPS9[i]
    ans = pairs(n, subtype)[k][1]["response"] if subtype else None
    st = {x["name"]: (x["status"], x.get("error")) for x in pairs(n, "mcp_status")[i][1]["response"]["response"]["mcpServers"]}
    m = [m for m in meta(n, "after") if m["after"] == label][0 if label != "mcp_toggle off" else (0 if i == 1 else 1)]
    return label, (ans["subtype"] + (": " + ans["error"] if ans.get("error") else "")) if ans else "", st, m["saved_disabledMcpServers"], m["server_starts_so_far"]


w("| step | answer | `ping-flaky` after it | `no-such-command` after it | saved `disabledMcpServers` | `ping-flaky` starts so far |")
w("|---|---|---|---|---|---|")
for i in range(len(STEPS9)):
    rows = [row9(n, i) for n in N9]
    check(rows[0] == rows[1], "the two recordings agree at step %d" % i)
    label, ans, st, saved, starts = rows[0]
    show = lambda s: "`%s`" % s[0] + (" (`%s`)" % s[1] if s[1] else "")
    w("| %s | %s | %s | %s | `%s` | %d |" % (label, "`%s`" % ans if ans else "", show(st["ping-flaky"]), show(st["no-such-command"]), json.dumps(saved), starts))
check(row9(N9[0], 2)[1:] == ("error: Connection closed", {"ping-flaky": ("failed", "Connection closed"), "no-such-command": ("failed", 'Executable not found in $PATH: "stdio"')}, [], 3),
      "a failed Turn on is an error, starts the program twice and leaves the switch on")
check(row9(N9[0], 0)[3] is None, "no entry for the folder at the start")
check(row9(N9[0], 4)[1] == "error: Connection closed" and row9(N9[0], 8)[1] == 'error: Executable not found in $PATH: "stdio"', "the words of the errors")
w("""
(`null`: the throw-away HOME had no entry for the folder yet. A start is a line the program wrote: a program file that
is gone or does not exist writes none.)
""")
w("""
What the adapter can rely on (n=2, the two recordings agree step for step):
- A "Turn on" whose program cannot start is a control ERROR: `{"subtype":"error","error":"Connection closed"}` for a
  program that starts and exits at once, and for a program file that is gone (`python3` cannot open it). `mcp_status`
  then says `failed` with the same words in `error`. For the program that exits at once the CLI started it TWICE
  before it answered (starts 1 -> 3).
- `mcp_reconnect` on a server whose program is gone: the same error and the same status.
- A program that does not exist: `Executable not found in $PATH: "stdio"` in the answer and in `mcp_status`. The words
  name the transport type (`stdio`), not the program (part 1, `broken-mcp`, has the same quirk in another form). Do
  not show these words as the name of what is missing.
- A failed "Turn on" still counts as ON for the saved switch: `disabledMcpServers` is `[]` right after it. The next
  process in the folder will try to start the server again (and fail the same way).
- `mcp_reconnect` once the cause is gone (the file `switch.on` back) answers success and the server is `connected`.
""")

# ================================================================= 10
w("## 10. The spending limit reached while a long tool call runs, and while a helper works\n")
w("Question: is the call cut off, or does it run to its end? What happens to the helper? What does the stop say?\n")
L10 = ["1", "2"]
N10 = {lab: sorted(n for n in ALL if n.startswith("r7-budget-during-call-%s-" % lab)) for lab in L10}
TRY = {lab: [n for n in N10[lab] if not n.endswith("-calib")] for lab in L10}
w("Fixtures: %s. Flags of a try: %s. The cost grows only when a model request ends, and a running Bash call makes none, so the limit can only be "
  "passed DURING the call by a helper's request. One turn: the model starts a helper in the background that writes a story of about 600 words "
  "with no tool (one long request), then runs `echo go; sleep 61.5` in the foreground, then answers. Run `calib` has no budget; each try gets "
  "`--max-budget-usd` = the cost when the Bash call began + half the helper's cost, both from the run before. The script stops at the first try "
  "whose budget lay between those two costs, whose story came while the call ran, and whose turn ended with `error_max_budget_usd`. Each label "
  "needed one try.\n" % (both(N10["1"] + N10["2"]), flags(TRY["1"][0])))
n10 = TRY["1"][-1]
w("From the Bash call's `tool_use` line to the end (`%s`):\n" % n10)
w(block(tl(n10, lambda l: (l.get("type") == "assistant" and any(b["type"] in ("tool_use", "text") for b in l["message"]["content"])) or task_or_list(l)
           or kind(l) in ("result", "sent") or sub(l) == "init" or (kind(l) == "meta" and ("closing_stdin" in l["_meta"] or "after_the_exit" in l["_meta"] or "exit_code" in l["_meta"]))
           or (l.get("type") == "user" and is_cli(l)),
           start=lambda l: l.get("type") == "assistant" and any(b["type"] == "tool_use" and b["name"] == "Bash" for b in l["message"]["content"]), text=90, maxlist=3)))
w("| try | budget (USD) | cost when the call began | cost once the story was written | story line (s) | `result` (s) | call's result | `sleep` alive at the result / 3 s later / after one more message | after stdin close |")
w("|---|---|---|---|---|---|---|---|---|")
D10 = []
for lab in L10:
    for n in TRY[lab]:
        m = meta(n, "run")[0]
        k = m["costs"]
        later, second, ae = note(n, "three_s_after_the_result"), note(n, "second_turn"), note(n, "after_the_exit")
        check(m["result_subtype"] == "error_max_budget_usd" and m["result_errors"] == ["Reached maximum budget ($%.6f)" % m["budget_usd"]] or
              m["result_errors"] == ["Reached maximum budget ($%s)" % ("%.6f" % m["budget_usd"]).rstrip("0")], "the stop of " + n)
        check(k["when_the_bash_call_began"] < m["budget_usd"] < k["once_the_story_was_written"], "the budget lies between in " + n)
        check(m["t_bash_tool_use"] < m["t_helper_story"] < m["t_result"] and m["t_bash_tool_result"] is None, "the limit came during the call in " + n)
        check(m["sleep_61_5_alive_at_the_result"] and later["sleep_61_5_alive"] and second["sleep_61_5_alive"] and later["list"] == [] and second["list"] == [], "the call runs on, unlisted, in " + n)
        check(second["result_subtype"] == "error_max_budget_usd", "one more message fails in " + n)
        check(ae["lines_after_stdin_was_closed"] == ["system/task_updated \"killed\"", "system/task_notification \"stopped\""] and not ae["sleep_61_5_alive"], "the call ends at the close in " + n)
        check([x["patch"]["status"] if x["subtype"] == "task_updated" else x["status"] for x in m["task_lines"]] == ["completed", "completed"], "the helper completed in " + n)
        rs = results(n)
        check(rs[0]["terminal_reason"] == "budget_exhausted" and rs[0]["stop_reason"] == "tool_use", "the words of the stop in " + n)
        check(rs[1].get("origin", {}).get("kind") == "task-notification" and rs[1]["subtype"] == "error_max_budget_usd" and rs[1]["num_turns"] == 1, "the self-started turn fails at once in " + n)
        D10.append(m["t_result"] - m["t_helper_story"])
        w("| `%s` | %.6f | %.6f | %.6f | %.2f | %.2f | none | %s / %s / %s | %s |" % (n, m["budget_usd"], k["when_the_bash_call_began"], k["once_the_story_was_written"], m["t_helper_story"],
                                                                         m["t_result"], *["yes" if x else "no" for x in (m["sleep_61_5_alive_at_the_result"], later["sleep_61_5_alive"], second["sleep_61_5_alive"])],
                                                                         ", ".join("`%s`" % x for x in ae["lines_after_stdin_was_closed"])))
for lab in L10:
    m = meta("r7-budget-during-call-%s-calib" % lab, "run")[0]
    check(m["result_subtype"] == "success" and m["t_bash_tool_result"] - m["t_bash_tool_use"] > 61.5, "the call runs to its end with no budget in label " + lab)
w("""
What the adapter can rely on (n=2):
- The turn is cut off, NOT the call. %s s after the helper's request that passed the limit ended (its story line), the
  turn's `result` came: `error_max_budget_usd`, `terminal_reason:"budget_exhausted"`, `stop_reason:"tool_use"`,
  `errors:["Reached maximum budget ($<budget>)"]`. The Bash call had no tool result.
- The CALL RUNS ON, unseen: its `sleep 61.5` was alive at the `result`, 3 s later and after one more message; it is
  not in `background_tasks_changed` (the list was `[]`). It ended only when stdin was closed: `task_updated` `killed`
  and `task_notification` `stopped` for its task (a foreground Bash call that had become a task: section 12), then the
  exit. (In the runs with no budget the same call ran its 61.5 s and returned.)
- The helper's request that passed the limit was its last: the helper ended `completed` (`task_updated`,
  `task_notification`) before the turn's `result`. So "reached while a helper works" ends no helper here; a helper
  that still had requests to make is not recorded.
- Right after the stop the CLI starts a turn by itself for the helper's notification, and that turn fails at once
  with the same `result` (`num_turns` 1, `origin.kind:"task-notification"`, no model request). One more message fails
  the same way (part 4, section 10: the process does no more work).
- For the stop text: the work of the turn is cut off, but a command it started may still run; after a budget stop the
  host should close the process (stdin) so that such a command ends.
""" % secs(D10))

# ================================================================= 11
w("## 11. `stop_task` on a background helper WHILE a turn of the agent runs\n")
w("Question: does a turn start by itself after the running one?\n")
N11 = ["r7-stop-helper-mid-turn-1", "r7-stop-helper-mid-turn-2"]
w("Fixtures: %s. Flags: %s. Turn 1 starts a helper in the background (its Bash call: `echo begin; sleep 173; echo end`) and ends. Turn 2 runs "
  "`echo go; sleep 21.5`; 2 s into that call the host sends `stop_task` with the helper's id, reads until turn 2's result and 20 s more.\n" % (both(N11), flags(N11[0])))
w("Every line from the driver's note before `stop_task` to the end (first recording):\n")
w(block(tl(N11[0], lambda l: quiet(l) and sub(l) not in ("status", "task_progress"), start=from_note("before_stop_task"), text=100, maxlist=4)))
for n in N11:
    bs, a11 = note(n, "before_stop_task"), note(n, "after_stop_task")
    check(bs["a_turn_runs"] and len(bs["list"]) == 1 and bs["sleep_173_alive"] and bs["sleep_21_5_alive"], "a turn runs and the helper works in " + n)
    check(a11["lines"][:4] == ["system/task_updated \"killed\"", "system/task_notification \"stopped\"", "system/background_tasks_changed 0 listed", "control_response success"], "the stop in " + n)
    check(a11["results_since_the_request"] == 1 and a11["result_origins"] == [None] and a11["lines"][-1] == "result/success", "only the running turn's result in " + n)
    check(meta(n, "after_stop_task")[0]["seconds_from_stop_task"]["init_after_it"] is None, "no turn by itself in " + n)
    check(note(n, "transcript") == {"transcript_found": True, "enqueued": 1, "attached_to_a_running_turn": 1, "removed": 1, "dequeued": 0}, "the transcript counts of " + n)
w("""What the adapter can rely on (n=2):
- The stop itself is as with no turn running: `task_updated` `killed`, `task_notification` `stopped`, the list `[]`,
  the answer `{}`, then the helper's two `user` lines.
- The running turn goes on and ends normally (`success`). NO turn starts by itself after it: no `system/init` in the
  20 s after that `result`. (With no turn running the CLI starts one at once: part 3, section 1; part 4, section 2.)
- Where the helper's notification went: the session's transcript shows ONE task notification put in the CLI's queue
  and handed to a model request of the running turn (`queued_command`), then taken out of the queue, in each run. So
  the agent is told about the stop inside the turn that runs; the stream itself shows no line for that.
- So the hint "goes on with a turn of its own" is right only when no turn runs. When a turn runs, the agent is told
  within that turn.
""")

# ================================================================= 12
w("## 12. A foreground Bash call that becomes a task and ends by itself\n")
w("Question: its `task_notification` (status word, `output_file`); how long a call must run before it becomes a task.\n")
N12 = ["r7-bash-becomes-task-1", "r7-bash-becomes-task-2"]
w("Fixtures: %s. Flags: %s. One process, one turn per call: `echo q12; sleep D` for D = 1, 2, 3, 4, 6, 9, 14 s, in the foreground. "
  "The commands end by themselves.\n" % (both(N12), flags(N12[0])))
w("The call of 4 s, from its `tool_use` line to its tool result (first recording):\n")
w(block(tl(N12[0], lambda l: task_or_list(l) or (l.get("type") == "assistant" and any(b["type"] == "tool_use" for b in l["message"]["content"])) or (l.get("type") == "user" and is_cli(l)),
           start=lambda l: l.get("type") == "assistant" and any(b["type"] == "tool_use" and b["input"].get("command") == "echo q12; sleep 4" for b in l["message"]["content"]),
           stop=lambda l: l.get("type") == "user" and is_cli(l), text=100)))
w("| D (s) | a task? run 1 / run 2 | `tool_use` -> `task_started` (s) | `tool_use` -> tool result (s) | `task_notification` |")
w("|---|---|---|---|---|")
TS12, NOTASK12 = [], []
for d in (1, 2, 3, 4, 6, 9, 14):
    ms = [[m for m in meta(n, "duration") if m["duration"] == d][0] for n in N12]
    for m in ms:
        check(m["turn_ok"] and m["command_in_the_call"] == ["echo q12; sleep %d" % d], "the call of %d s" % d)
        check(m["task_started"] == (d >= 3), "a task or not at %d s" % d)
        if not m["task_started"]:
            NOTASK12.append(m["seconds_from_tool_use_to_tool_result"])
        if d == 3:
            check(0 < m["seconds_from_tool_use_to_tool_result"] - m["seconds_from_tool_use_to_task_started"] < 0.03, "the 3 s call became a task just before it returned")
        if m["task_started"]:
            check(m["task_notification"]["status"] == "completed" and m["task_notification"]["output_file"] == "" and m["task_notification_before_tool_result"], "the end of the %d s call" % d)
            TS12.append(m["seconds_from_tool_use_to_task_started"])
    w("| %d | %s | %s | %s | %s |" % (d, " / ".join("yes" if m["task_started"] else "no" for m in ms),
                                       " / ".join("%.3f" % m["seconds_from_tool_use_to_task_started"] if m["task_started"] else "-" for m in ms),
                                       " / ".join("%.3f" % m["seconds_from_tool_use_to_tool_result"] for m in ms),
                                       "`completed`, `\"output_file\":\"\"`" if ms[0]["task_started"] else "none"))
for n in N12:
    bash_tasks = {l["task_id"] for l in fx(n) if sub(l) == "task_started" and l["task_type"] == "local_bash"}
    check(len(bash_tasks) == 5 and not any(sub(l) == "task_updated" and l["task_id"] in bash_tasks for l in fx(n)), "no task_updated for a Bash call in " + n)
    check(sorted((l["task_id"], l["status"], l["output_file"]) for l in fx(n) if sub(l) == "task_notification") == sorted((t, "completed", "") for t in bash_tasks),
          "one task_notification `completed` with no output_file per Bash task in " + n)
    check(not any(sub(l) == "background_tasks_changed" for l in fx(n)), "no list line in " + n)
    check(all(l["is_backgrounded"] is False for l in fx(n) if sub(l) == "task_started"), "never backgrounded in " + n)
inside = [l for l in fx(N3[0]) if sub(l) == "task_started" and l.get("owned_by_subagent")]
ends = [l for l in fx(N3[0]) if sub(l) == "task_notification" and inside and l["task_id"] == inside[0]["task_id"]]
check(len(inside) == 1 and [e["status"] for e in ends] == ["completed"] and ends[0]["output_file"] == "", "the helper's Bash call in section 3")
w("""
What the adapter can rely on (n=2 per duration; 10 calls became tasks, 4 did not):
- A foreground Bash call becomes a task about 3 s after its `tool_use` line: `task_started` came %.3f to %.3f s after it,
  with `"task_type":"local_bash"`, `"is_backgrounded":false` and the call's `tool_use_id`. The calls of D = 1 and 2
  (tool results %.3f to %.3f s after the line) got no task line. The 3 s calls became tasks about 0.02 s before they
  returned.
- When the call ends by itself: ONE `task_notification` with `"status":"completed"`, `"output_file":""` and the call's
  description in `summary`, written just BEFORE the call's tool result. No `task_updated`, no list line (it was never
  in the background list).
- The same inside a helper (section 3): the helper's Bash call has `"owned_by_subagent":true`, and its end is one
  `task_notification` `completed` with an empty `output_file`.
- So such a line belongs to a Bash call (its `tool_use_id` is the call's), not to a helper or a background command; it
  must end neither. Status words of these lines seen so far: `completed` (here), `stopped` (an interrupt or a stop:
  part 4, sections 1 and 2; the budget stop's close: section 10).
""" % (min(TS12), max(TS12), min(NOTASK12), max(NOTASK12)))

# ================================================================= part 4 notes
w("## Notes of part 4 written late\n")
w("The script `r6_a2_shutdown_helper.py` listed the lines, then looked for two processes, then wrote its note `before_stdin_is_closed`; a line "
  "that came in between is in the file before the note but not in its list. The other part 4 notes that list lines (`r6_a1`: `after_the_interrupt`; "
  "`r6_a2`: `after_stop_task`, `after_the_exit`) were made in the same order. This build compares each such list with the lines in the fixture "
  "between the note's starting point (the first host request; for `after_the_exit` the close of stdin) and the note:\n")
LATE = []
w("| fixture | note | lines in its list | lines in the file before it |")
w("|---|---|---|---|")


def kindline(l):
    return is_cli(l) and l.get("type") not in ("stream_event", "rate_limit_event") and l.get("subtype") not in ("thinking_tokens", "commands_changed", "status")


for n in sorted(f[:-len(".ndjson")] for f in os.listdir(FIX) if f.startswith("r6-shutdown-helper-") or f.startswith("r6-interrupt-helper-")):
    lines = fx(n)
    first = [i for i, l in enumerate(lines) if "_sent" in l and l["_sent"].get("type") == "control_request"][0]
    closed = [i for i, l in enumerate(lines) if "_meta" in l and "closing_stdin" in l["_meta"]][0]
    for i, l in enumerate(lines):
        for key, v in (l.get("_meta") or {}).items():
            for k2, listed in (v.items() if isinstance(v, dict) else []):
                if k2.startswith("lines") and isinstance(listed, list):
                    since = closed if key == "after_the_exit" else first
                    seen = sum(1 for x in lines[since:i] if kindline(x))
                    if seen != len(listed):
                        LATE.append((n, key))
                        w("| `%s` | `%s` | %d | %d |" % (n, key, len(listed), seen))
check(LATE == [("r6-shutdown-helper-idle-close-now-1", "before_stdin_is_closed"), ("r6-shutdown-helper-idle-close-now-2", "before_stdin_is_closed")], "the late notes of part 4")
w("""
Every other such note of part 4 lists exactly the lines in its window. The two late notes are the ones part 4 quotes in
section 2 (`idle-close-now`): there the `user`, `user` and `system/init` lines of the self-started turn came between
the list and the note, so the note's `init_of_a_turn` is null although that `init` came before stdin was closed. What
part 4 says about that variant (the turn had begun when stdin was closed, and ran to its end after it) follows the
lines of the file, not the note, and stands. Part 4 now says this under its quote. `r6_a2` is fixed (process checks
first, then the list and the note in one step under the driver's lock); its fixtures were not recorded again.
""")

# ================================================================= code
w("""## Can the waiting code be built now? (as of the day of recording)

| # | the code that waits | can it be built now? |
|---|---|---|
| 1 | The hint and the record of "Turn off" (`live_session.rs` "for <agent>'s run"; `session.ts` `MCP_HINT`) | yes: the switch is saved in `~/.claude.json` under the folder's path (`disabledMcpServers`) and holds for every later process in that folder, new or resumed, until "Turn on". The words "for <agent>'s run" are wrong: it is for this folder. "Turn on starts it again" is right |
| 2 | Dropping `--strict-mcp-config` so that your own servers work in a folder you have not trusted (`trust.rs`, `claude.rs` `use_user_setup`) | yes, for the repo's `.mcp.json`: not loaded in %d of %d recordings of parts 4 and 5 under `user` or `user,local` (4 logged in), also with an approval kept in `~/.claude.json`. To know: with `user,local` a local-scope server of the folder (the user's own) loads too. Not recorded: plugins' servers |
| 3 | `on_result` and the store's rule for a FAILED turn (helpers) | yes: a turn that fails by a limit ends no background helper; the helper ends at its own `task_notification`, and a turn by itself follows. Keep helpers past a failed turn, as past a done one. Not recorded: a failure by an API error |
| 4 | The same rule for an interrupt with several helpers, or a helper that waits for the model | yes: each helper gets its own `killed` / `stopped` pair before the answer (2 helpers, n=2; a helper waiting for its answer, n=2) |
| 5 | `begin_shutdown` / `wind_down` | yes: `interrupt` first, also with no turn running (it ends the helpers and leaves the commands listed), then `stop_task` for each listed command, then close: exit in 0.70 s, nothing left (n=2). Today `wind_down` with no turn running skips the interrupt and leaves a helper to the later steps |
| 6 | The watchdog's short limit for `mcp__` tools | yes: an HTTP server's tool gets the same 30 s heartbeat. New: such a call ends with `The operation timed out.` at 60 s (n=2, a server that answers with a JSON body): a limit above 60 s for it is never reached |
| 7 | One answer for "approve and accept edits" (`grantable` never sends `setMode`) | yes: the request's own `setMode` suggestion in the allow answer switches the session to `acceptEdits` (status line at once, next `init`, the next Edit does not ask). Read the mode back from those lines: `get_settings` has none |
| 8 | Taking an effort away with no restart (`runs.rs` `live_model_switch`) | yes: `effortLevel:null` gives `medium` for `low`, `high` and `max` (n=2 each) |
| 9 | The toast of a failed "Turn on" / "Restart" (`AgentMcp.svelte`) | yes: both are control errors with the CLI's words (`Connection closed`; `Executable not found in $PATH: "stdio"`, which names the transport, not the program) and `mcp_status` says `failed` with the same words. After a failed "Turn on" the saved switch is on |
| 10 | The text of the budget stop (`failure_reason`) and package `answers-and-limits` item 3 | partly: the text can say that the work stopped (`budget_exhausted`, `Reached maximum budget ($…)`); it must not say that everything stopped: a Bash call that runs at that moment runs on, unlisted, until the process ends. The host should end the process after such a stop. A helper with requests left at the stop is not recorded |
| 11 | The Stop hint of a helper row (`session.ts` `stopHint`) and the demo | yes: the agent "goes on with a turn of its own" only when no turn runs; during a turn it is told within that turn and no turn follows |
| 12 | `on_task` (a line that must end no helper and no background row) | yes: a foreground Bash call becomes a task about 3 s after its `tool_use`; its end by itself is one `task_notification` `completed` with `"output_file":""` and the call's `tool_use_id`, no `task_updated`, no list line |
""" % (COUNT2 + len(P4_ROWS), COUNT2 + len(P4_ROWS)))

# ================================================================= table
w("""## Table: question -> answer -> fixture

| # | question | answer | n | fixture |
|---|---|---|---|---|
| 1 | `mcp_toggle` off, exit, a new process in the same folder | Saved in `~/.claude.json` (`projects[<folder>].disabledMcpServers`); new and resumed processes say `disabled`, do not start the server; only `mcp_toggle` on undoes it | 2 | `r7-mcp-toggle-saved-1-a` … `-2-c` |
| 2 | The repo's `.mcp.json` without `--strict-mcp-config` under `user` / `user,local` | Never loaded (%d recordings here, 6 in part 4), also with an approval in `~/.claude.json`; a local-scope server loads with `user,local` only | 1 to 2 per form | `r7-untrusted-mcp-*` |
| 3 | A turn that fails by itself while a helper works | The helper goes on (still listed), ends `completed` later, a turn by itself follows | 2 | `r7-failed-turn-helper-1`, `-2` |
| 4 | An interrupt with two helpers; with a helper that waits for the model | All ended: `killed` / `stopped` for each before the answer; the waiting helper's line is `[Request interrupted by user]` | 2 + 2 | `r7-interrupt-two-helpers-1`, `-2`, `r7-interrupt-helper-model-1`, `-2` |
| 5 | Shutdown with a helper and a command | interrupt ends the helper (command still listed), `stop_task` ends the command, exit 0.70 s after close, nothing left | 2 | `r7-shutdown-helper-and-command-1`, `-2` |
| 6 | `tool_progress` of an HTTP MCP tool | Heartbeat at 30 and 60 s, as for stdio; the call ends at 60 s with `The operation timed out.` | 2 (+1 control) | `r7-tool-progress-http-70-1`, `-2`, `-45-1` |
| 7 | Allow with the request's `setMode` suggestion | Session in `acceptEdits`: status line, next `init`, next Edit not asked; `get_settings` has no mode | 2 | `r7-allow-set-mode-1`, `-2` |
| 8 | `effortLevel:null` on `--effort low` / `max` | `medium` at once and after a turn | 2 per level | `r7-effort-null-low-1` … `-max-2` |
| 9 | "Turn on" of a server that cannot start; reconnect of a gone program | control error `Connection closed` (or `Executable not found in $PATH: "stdio"`), status `failed` with the same words; the saved switch is on | 2 | `r7-mcp-cannot-start-1`, `-2` |
| 10 | Budget reached during a long call and while a helper works | The turn ends (`budget_exhausted`), the call runs on unlisted until stdin closes; the helper had completed; a turn by itself fails at once | 2 | `r7-budget-during-call-1-calib`, `-1-t1`, `-2-calib`, `-2-t1` |
| 11 | `stop_task` on a helper while a turn runs | The turn goes on; no turn by itself after it; the notification goes into the running turn | 2 | `r7-stop-helper-mid-turn-1`, `-2` |
| 12 | A Bash call that becomes a task and ends by itself | Task about 3 s after `tool_use`; `task_notification` `completed`, `"output_file":""`, before the tool result, no `task_updated` | 2 per duration | `r7-bash-becomes-task-1`, `-2` |

## Not verified

Not in this part (still open from the earlier lists):
- The text printed when `failIfUnavailable` stops a start (needs a machine where the sandbox cannot start).
- `mcp_authenticate`, `mcp_clear_auth`, `mcp_oauth_callback_url` (a real sign-in; may write to the login keychain).
- `claude --resume` typed into the app's terminal on a session Codly started headless.
- Linux behaviour of process groups.
- A `tool_progress` line for WebFetch (needs a slow https page with a trusted certificate).
- A network allow that lasts hours (part 3 recorded 304 s).
- Whether a model request carries the effort level that `get_settings` shows.
- The `can_use_tool` request of a user-scope server (needs the user's real setup).

Open after these recordings:
- 1: whether a REAL HOME behaves the same (recorded in a throw-away HOME: the same file layout is assumed); a server of
  the same name in another folder (expected: not touched, its own entry); a switch made while a turn runs.
- 2: a server from a plugin; the logged-in forms a third time; `user,local` with a local file that has only one of the
  two keys.
- 3: a turn that fails by an API error, or by the budget while a helper still has requests to make.
- 4: a nested helper; a helper with a background command of its own; `interrupt` with `cancel_queued` or `send_now`
  while helpers work.
- 5: the same with a turn running; a helper that ignores the stop (none did).
- 6: an HTTP server that streams its answer (event stream) or sends progress notifications: does the 60 s limit move?
  Whether a setting changes that limit.
- 7: the same answer on a Bash request (part 1 had it among several suggestions); a `setMode` to another mode.
- 8: `null` while a turn runs.
- 9: a server that cannot start at the process start AND is then turned off and on in a NEW process (the saved switch).
- 10: a helper that still had requests to make when the limit was passed; whether closing stdin is the only end of the
  call that runs on (an `interrupt` after the stop was not tried).
- 11: `stop_task` on a COMMAND while a turn runs.
- 12: the line of a Bash call that becomes a task and FAILS by itself (exit code not 0).
""" % COUNT2)

out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(OUT, "NOTES-part5.md")
with open(out, "w") as f:
    f.write("\n".join(P))
print("wrote", out, sum(len(x) for x in P), "characters")
