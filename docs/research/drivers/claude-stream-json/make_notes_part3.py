"""Builds the part-3 document from the sanitized r5-* fixtures, so every JSON line quoted there is copied from a fixture.

python3 make_notes_part3.py [output file]      (default: $CAPTURE_DIR/NOTES-part3.md)

Numbers and words that the text states about a fixture are checked against it (`check`): the build fails when a
recording no longer says what the text says.
"""
import json
import os
import sys

from notes_lib import OUT, argv_flags, block, control, fx, is_cli, j, quiet, sent, slim

P = []
w = P.append


def check(ok, what):
    if not ok:
        raise SystemExit("the text no longer matches the fixtures: " + what)


def flags(name):
    return "`%s`" % argv_flags(name)


def slimp(l, text=200, brief=False):
    """slim(), but a line of a subagent keeps the keys that say so."""
    out = slim(l, text, brief)
    if l.get("type") in ("assistant", "user") and is_cli(l) and l.get("parent_tool_use_id"):
        tail = out.pop("…")
        out["parent_tool_use_id"] = l["parent_tool_use_id"]
        if "agent_id" in l:
            out["agent_id"] = l["agent_id"]
        out["…"] = tail
    return out


def tl(name, pred, start=None, stop=None, text=200, brief=True, **kw):
    """Lines with their time (`# t=…`: seconds since the process started). start / stop: predicates that open and
    close the window (the stop line is included)."""
    out, t, on = [], 0.0, start is None
    for l in fx(name):
        if set(l) == {"_t"}:
            t = l["_t"]
            continue
        if "_t" in l:
            t = l["_t"]
        if not on and start(l):
            on = True
        if on and quiet(l) and pred(l):
            out.append("# t=%.2f" % t)
            out.append(j(slimp(l, text, brief), **kw))
        if on and stop is not None and stop(l):
            break
    return out


def kind(l):
    if "_sent" in l:
        return "sent"
    if "_meta" in l:
        return "meta"
    if "_stderr" in l:
        return "stderr"
    return l.get("type")


def sub(l):
    return l.get("subtype") if l.get("type") == "system" else None


TASK = ("task_started", "task_updated", "task_notification", "background_tasks_changed")


def is_tool_result(l):
    c = l.get("message", {}).get("content")
    return l.get("type") == "user" and isinstance(c, list) and any(isinstance(b, dict) and b.get("type") == "tool_result" for b in c)


def has_text(l):
    return l.get("type") == "assistant" and any(b["type"] == "text" for b in l["message"]["content"])


def has_tool_use(l):
    return l.get("type") == "assistant" and any(b["type"] == "tool_use" for b in l["message"]["content"])


def closing(l):
    return "_meta" in l and ("closing_stdin" in l["_meta"] or "exit_code" in l["_meta"])


def meta(name, key):
    return [l["_meta"] for l in fx(name) if "_meta" in l and key in l["_meta"]]


def metas(name, key=None, **kw):
    return [j(l, **kw) for l in fx(name) if "_meta" in l and not closing(l) and (key is None or key in l["_meta"])]


def reqs(name, tool=None, n=None):
    """can_use_tool requests of a fixture, each followed by the driver's answer."""
    lines, out = fx(name), []
    for i, l in enumerate(lines):
        if l.get("type") == "control_request" and l["request"].get("subtype") == "can_use_tool" and (tool is None or l["request"]["tool_name"] == tool):
            out.append(j(l))
            for m in lines[i + 1:]:
                if "_sent" in m and m["_sent"].get("type") == "control_response" and m["_sent"]["response"].get("request_id") == l["request_id"]:
                    out.append(j(m["_sent"]))
                    break
    return out if n is None else out[:2 * n]


def results(name):
    return [l for l in fx(name) if l.get("type") == "result"]


def cost(names):
    total = 0.0
    for n in names:
        last = {}
        for r in results(n):
            last[r.get("session_id")] = r.get("total_cost_usd") or 0.0
        total += sum(last.values())
    return total


def after_request(name, subtype, nth=0):
    """A predicate that is true from the nth host request of that subtype on."""
    seen = {"n": -1}

    def pred(l):
        if "_sent" in l and l["_sent"].get("type") == "control_request" and l["_sent"]["request"]["subtype"] == subtype:
            seen["n"] += 1
        return seen["n"] >= nth
    return pred


ALL = sorted(f[:-len(".ndjson")] for f in os.listdir(os.environ.get("FIXTURES_DIR") or os.path.join(OUT, "fixtures")) if f.startswith("r5-"))
# The CLI's total for a session goes on after --resume (section 14): the four budget runs are one session.
BUDGET = ["r5-budget-resume-" + x for x in "abcd"]
TOTAL = cost([n for n in ALL if n not in BUDGET]) + max(r.get("total_cost_usd") or 0.0 for n in BUDGET for r in results(n))

w("""# Claude Code CLI stream-json: recorded protocol shapes, part 3

- CLI: `claude` 2.1.311 (the same version as parts 1 and 2; checked with `claude --version` before the runs).
- Recorded: 2026-11-22, macOS, `--model haiku` (`claude-haiku-5-5`), subscription login (`"apiKeySource":"none"`).
- Method, base command, fixture line format and sanitizing: as in part 1 ("Base command", "Files", "Sanitizing") with
  the additions of part 2. Fixtures of this part are `fixtures/claude-2.1.311/r5-*.ndjson`; the scripts are
  `drivers/claude-stream-json/r5_*.py` (README there says how to record again).
- List price of the %d recordings (sum of `total_cost_usd` of their `result` lines): %.2f USD. %.2f USD of it is the one
  recording that leaves haiku on purpose (section 8: two model requests on sonnet and one on the account's default model).
- Everything below was observed in these recordings. Where a statement comes from reading the CLI bundle and not from a
  recording, it says "bundle, not recorded". Each answer says how many runs it rests on (n). Most are n=1.
- The sections are numbered as the 15 questions this part was recorded for; the last table has them in the same order.

Differences from the part 2 method:
- The capture folder was outside the CLI's own temp folder (`/tmp/claude-<uid>/`), so the sandbox scenarios are not
  spoiled by the write access the sandbox gives to that folder. In the fixtures the work folder is `/tmp/work/…` as
  before (a placeholder).
- Three scenarios use a THROW-AWAY HOME (`HOME=<a new empty folder>`, never the real one; the driver notes it as
  `env_set`). A throw-away HOME has no login: the CLI starts, answers control requests and loads MCP servers, but a
  turn ends at once with `"result":"Not logged in · Please run /login"` (`subtype:"success"`, `is_error:true`,
  `terminal_reason:"api_error"`) and costs nothing. So these runs show what is LOADED, not what a tool call looks like.
- New sanitizing: the MCP servers of this part (`ping-slow`, `ping-flaky`, `ping-config`, `ping-same`, `ping-user`)
  are kept in `init` and `mcp_status`; every other server is still removed.
- In quoted lines `"…":"…"` stands for keys left out; `# t=` lines are seconds since the process started (as in part 2).
  A quoted line of a subagent keeps `parent_tool_use_id` and `agent_id`.
""" % (len(ALL), TOTAL, cost(["r5-model-midturn"])))

# ================================================================= 1
w("## 1. `stop_task` on a background SUBAGENT (`task_type:\"local_agent\"`)\n")
w("Question: the answer, the lines that follow, and does the helper end?\n")
N1 = ["r5-stop-subagent-1", "r5-stop-subagent-2"]
w("Fixtures: `r5-stop-subagent-1`, `r5-stop-subagent-2` (the same scenario twice). Flags: %s.\n" % flags(N1[0]))
w("Sent: one message that makes the model start a subagent in the background; the subagent runs "
  "`echo begin; sleep 113; echo end` with Bash. The turn ends (`started`). 3 s after the subagent's Bash call began the host sends "
  "`stop_task` with the subagent's `task_id`, reads for 20 s, then asks `get_task_output`, sends `stop_task` again, and one more message.\n")
w(block([j(x) for x in sent(N1[0], "user")[:1]]))
w("How the subagent shows up (first recording; the task lines and the first lines of the subagent):\n")
w(block(tl(N1[0], lambda l: sub(l) in TASK + ("task_progress",) or (l.get("type") == "assistant" and l.get("parent_tool_use_id")),
           stop=lambda l: "_meta" in l)[:-2]))
w("The stop, every line from the driver's note before it to the end:\n")
w(block(tl(N1[0], lambda l: kind(l) in ("sent", "meta", "control_response", "result", "user") or sub(l) in TASK + ("init",) or has_text(l),
           start=lambda l: "_meta" in l and "list_before_stop" in l["_meta"], text=120, maxlist=9)))


def stop_story(name):
    """Kinds of the lines from the first stop_task request to the next result."""
    out, on = [], False
    for l in fx(name):
        if "_sent" in l and l["_sent"].get("type") == "control_request" and l["_sent"]["request"]["subtype"] == "stop_task":
            on = True
        if on and quiet(l) and is_cli(l) and l.get("type") != "assistant" and sub(l) != "status":
            out.append("%s%s%s" % (l["type"], "/" + l["subtype"] if l.get("subtype") else "", " (subagent)" if l.get("parent_tool_use_id") else ""))
            if l["type"] == "result":
                break
    return out


S1 = [stop_story(n) for n in N1]
check(S1[0] == S1[1], "the two stop-subagent recordings have the same lines after stop_task")
for n in N1:
    m = meta(n, "result_within_20s_of_stop_task")[0]
    check(m["result_within_20s_of_stop_task"] and not m["host_sees_sleep_113_after_stop"] and m["list_after_stop"] == [], "stop-subagent notes of " + n)
w("Both recordings have the same lines in the same order from `stop_task` to the next `result`: `%s`.\n" % "`, `".join(S1[0]))
w("""What the adapter can rely on (n=2):
- A background subagent is in `background_tasks_changed.tasks` as
  `{"task_id","run_id","task_type":"local_agent","subagent_type","description"}` (a Bash task has no `subagent_type`),
  and `task_started` has `task_type:"local_agent"`, `is_backgrounded:true`, `spawn_depth`, `prompt`.
- `stop_task {"task_id": <the subagent's id>}` works on it and looks like the stop of a Bash task: `task_updated`
  (`patch.status:"killed"`), `task_notification` with `status:"stopped"`, `background_tasks_changed` with `[]`, THEN the
  answer `{}`. All in the same 10 ms.
- The helper ends: its running Bash call is rejected (`user` line of the subagent with the "doesn't want to proceed"
  tool result, then `[Request interrupted by user for tool use]`, both with `parent_tool_use_id` and `agent_id`), no
  further line of the subagent came, and the host saw the `sleep 113` process gone when it looked (2 s later).
- Different from a Bash task (part 2, scenario 6: no turn after a stop): after the stop of a subagent the CLI starts a
  turn BY ITSELF at once (`system/init`, the model says the helper was stopped, `result` with
  `origin.kind:"task-notification"`). A host that stops a helper must expect one more turn, and at shutdown it must
  stop that turn too or close before it.
- `get_task_output` does not know a subagent's id: a control ERROR
  (`get_task_output: no shell or Monitor task with that task_id in this session`), before and after the stop.
- `stop_task` on the stopped subagent answers `{}` again and writes nothing (as for a Bash task).
""")
w("The self-started turn's `result` (first recording):\n")
w(block([j(slim(results(N1[0])[1]) | {"origin": results(N1[0])[1].get("origin")})]))
check(results(N1[0])[1].get("origin", {}).get("kind") == "task-notification", "origin of the turn after stop_task on a subagent")

# ================================================================= 2
w("## 2. A background command that fails; who is in `background_tasks_changed`\n")
w("Question: the `task_notification` status word of a background command that ends with exit code 1, and the list line after it. Is a background subagent listed, with which fields? Is a FOREGROUND helper ever in the list?\n")
N2 = "r5-background-fail"
w("Fixture: `%s`. Flags: %s.\n" % (N2, flags(N2)))
w("Turn 1 starts `sleep 4; exit 1` with `run_in_background:true`. Task lines and results up to the end of the turn the CLI then starts by itself:\n")
w(block(tl(N2, lambda l: sub(l) in TASK or kind(l) == "result", stop=lambda l: l.get("type") == "result" and l.get("result_index") == 1, text=120)))
r2 = [l for l in fx(N2) if sub(l) == "task_notification"][0]
check(r2["status"] == "failed", "status word of the failed background command")
w("Turn 3: a subagent in the FOREGROUND (`run_in_background:false`) whose Bash call takes 6 s. Task lines and the driver's count of list lines:\n")
w(block(tl(N2, lambda l: sub(l) in TASK or (kind(l) == "meta" and "background_tasks_changed_lines" in l["_meta"]),
           start=lambda l: "_sent" in l and "run_in_background set to false" in json.dumps(l), stop=lambda l: "_meta" in l and "background_tasks_changed_lines" in l["_meta"], text=120)))
check(meta(N2, "background_tasks_changed_lines")[0]["background_tasks_changed_lines"] == [], "no list line for the foreground subagent")
w("Turn 4: a foreground Bash call (`echo a; sleep 20; echo b`); 8 s into it the host sends `background_tasks`:\n")
w(block(tl(N2, lambda l: sub(l) in TASK or kind(l) in ("sent", "control_response", "result", "meta") or is_tool_result(l),
           start=lambda l: "_sent" in l and "sleep 20" in json.dumps(l), text=160, maxlist=9)))
w("""What the adapter can rely on (n=1):
- A background command that ends with a non-zero exit code: `task_updated` with `patch.status:"failed"`,
  `task_notification` with `"status":"failed"` and `summary` `Background command "…" failed with exit code 1`, then
  `background_tasks_changed` with `[]`, then the self-started turn. So the status words seen so far are `completed`,
  `failed` and `stopped` (`task_updated.patch.status`: `completed`, `failed`, `killed`).
- A background subagent IS in the list (section 1). A FOREGROUND subagent is not: its turn wrote `task_started`
  (`is_backgrounded:false`) and the end lines, and NO `background_tasks_changed` line. The same for a foreground Bash
  call. A Bash call of a subagent has its own `task_started` with `"owned_by_subagent":true` and
  `"parent_task_id":<the subagent's task id>`.
- `background_tasks` (the request with no fields) on a running FOREGROUND Bash call moves it to the background, now
  recorded: `background_tasks_changed` lists it, `task_updated` has `patch.is_backgrounded:true`, the answer is `{}`,
  and the tool call returns at once with `Command was manually backgrounded by user with ID: …` (the text adds that
  it is stopped if it still runs after 30 minutes in the background). The turn goes on
  without the result; when the command ends, the usual end lines and a self-started turn follow. So "is it in the
  list" is the right test for "does the turn wait for it": the same command was not listed before the request and is
  listed after it.
- A side finding for approval rules: in `acceptEdits` the command `sleep 4; exit 1` asked (`can_use_tool`), `sleep 8;
  echo woke` in part 2 did not.
""")

# ================================================================= 3
w("## 3. `tool_progress` for a tool other than Bash\n")
w("Question: the shape (`parent_tool_use_id` null? `tool_use_id`?), the interval, past 90 s.\n")
N3 = ["r5-tool-progress-mcp", "r5-tool-progress-subagent", "r5-tool-progress-webfetch"]
w("Fixtures: `r5-tool-progress-mcp` (a tool of a stdio MCP server that sleeps 100 s; %s), `r5-tool-progress-subagent` (a foreground subagent whose "
  "Bash call runs 100 s; %s), `r5-tool-progress-webfetch` (%s).\n" % (flags(N3[0]), flags(N3[1]), flags(N3[2])))
w("The slow MCP tool: the call, every `tool_progress` line, the result:\n")
w(block(tl(N3[0], lambda l: kind(l) == "tool_progress" or has_tool_use(l) or is_tool_result(l) or kind(l) == "control_request")))
w("The foreground subagent: the two calls, the task lines and every `tool_progress` line:\n")
w(block(tl(N3[1], lambda l: kind(l) == "tool_progress" or has_tool_use(l) or is_tool_result(l) or sub(l) in TASK, text=100)))
for n, tool in ((N3[0], "mcp__ping-slow__ping"), (N3[1], "Agent")):
    tp = [l for l in fx(n) if l.get("type") == "tool_progress"]
    check([x["elapsed_time_seconds"] for x in tp] == [30, 60, 90] and all(x["tool_name"] == tool and x["parent_tool_use_id"] and x["heartbeat"] for x in tp),
          "tool_progress lines of " + n)
w("WebFetch (the driver served a page on `127.0.0.1` that answers after 40 s):\n")
w(block(tl(N3[2], lambda l: has_tool_use(l) or is_tool_result(l) or kind(l) in ("control_request", "meta") and not closing(l), text=160, maxlist=9)))
w("""What the adapter can rely on (n=1 per tool):
- The line is the same for every tool that was recorded: `{"type":"tool_progress","tool_use_id":"<call id>-heartbeat-N",
  "tool_name":<the tool>,"parent_tool_use_id":"<call id>","elapsed_time_seconds":30|60|90,"heartbeat":true}`.
  `tool_name` was `mcp__ping-slow__ping` and `Agent` (part 2: `Bash`). One every 30 s, counted from the start of the
  call, also past 90 s (the calls ended at about 100 s, so 120 s is not recorded).
- `parent_tool_use_id` was never null: in 6 lines here and 3 in part 2 it is the id of the call that is running.
  A `tool_progress` line with a null parent was not seen in any recording.
- A Bash call INSIDE a subagent gets no heartbeat of its own: during its 100 s the only `tool_progress` lines were the
  ones of the `Agent` call of the main thread. Its `task_started` (with `owned_by_subagent`) is the only line that
  says it runs.
- WebFetch: the tool was offered with `--tools Bash,WebFetch` (`init.tools` has it; part 2 saw it missing with the
  app's longer tool list). Its call asks (`can_use_tool`) in `acceptEdits`. The plain-http page could not be fetched:
  the tool spoke TLS to the http port and failed at once (`WRONG_VERSION_NUMBER`). So a slow WebFetch is NOT
  recorded: it needs a slow https page with a certificate the CLI trusts.
""")

# ================================================================= 4
w("## 4. The stop sequence as one run; three waiting messages; a folded message\n")
w("Question: `interrupt` with `cancel_queued` during a tool call, then `stop_task` right after the interrupted turn's result, then stdin close. "
  "An interrupt while THREE messages wait: order in `still_queued`, one `cancelled` line each? A message already folded into the turn when the interrupt comes?\n")
N4 = "r5-stop-sequence"
w("Fixtures: `r5-stop-sequence`, `r5-stop-three-plain`, `r5-stop-three-cancel`, `r5-stop-folded-plain`, `r5-stop-folded-cancel`. Flags: %s. "
  "The CLI is the leader of its own process group, as the app starts it.\n" % flags(N4))
w("### The sequence\n")
w("A background command (`sleep 307; echo late`) runs since turn 1. Turn 2 runs `echo go; sleep 61`; a third message waits. "
  "Then the three steps, every line from the driver's note before the interrupt:\n")
w(block(tl(N4, lambda l: kind(l) in ("sent", "meta", "control_response", "result", "command_lifecycle") or sub(l) in TASK or is_tool_result(l),
           start=lambda l: "_meta" in l and "before_the_stop" in l["_meta"], text=120, maxlist=9)))
m4 = meta(N4, "result_after_interrupt_s")[0]
ex4 = [l for l in fx(N4) if "_meta" in l and "exit_code" in l["_meta"]][0]
cl4 = [l for l in fx(N4) if "_meta" in l and "closing_stdin" in l["_meta"]][0]
check(ex4["_meta"]["exit_code"] == 1 and meta(N4, "after_the_exit")[0]["after_the_exit"] == {"sleep_307_alive": False, "sleep_61_alive": False}, "end of the stop sequence")
w("""What the adapter can rely on (n=1; an earlier recording of the same script, not kept, had the same order of lines,
0.20 s to the result and exit code 1):
- The three steps work in this order with no wait between them other than "read until the `result`":
  1. `interrupt {"cancel_queued":true}`: the waiting message gets `cancelled`, the answer names it in `cancelled`, the
     running tool call is rejected, the turn's error `result` comes (%.2f s after the request by the driver's clock,
     the same time stamp in the stream), then `cancelled` for the turn's own message.
  2. `stop_task` for the background command: `killed` / `stopped` / list `[]` / answer `{}`, answered %.3f s after the
     interrupt. No turn starts.
  3. stdin closed: the driver noted the exit %.1f s later (it notes it 0.3 s after the fact), exit code 1 (the last
     `result` was the interrupted one: as in part 2, scenario 8).
- After the exit neither `sleep 307` nor `sleep 61` was alive. Nothing had to be signalled.
- Not in this run: a background SUBAGENT at shutdown. Section 1 says its stop starts a turn; a shutdown that stops a
  helper has to interrupt or outwait that turn before stdin close (not recorded as one run).
""" % (m4["result_after_interrupt_s"], m4["stop_task_answered_after_s"], ex4["_t"] - cl4["_t"]))
w("### Three messages wait, then the interrupt\n")
w("Turn 1 runs `sleep 15`; three messages (`banana`, `cherry`, `damson`) are written 0.3 s apart and wait. Plain `interrupt`:\n")
PRED4 = lambda l: kind(l) in ("sent", "control_response", "result", "command_lifecycle") or sub(l) == "init" or has_text(l)
w(block(tl("r5-stop-three-plain", PRED4, start=after_request("r5-stop-three-plain", "interrupt"), stop=lambda l: l.get("type") == "result" and l.get("result_index") == 1, text=120, maxlist=9)))
w("With `\"cancel_queued\":true`:\n")
w(block(tl("r5-stop-three-cancel", PRED4, start=lambda l: l.get("type") == "command_lifecycle" and l.get("state") == "cancelled",
           stop=lambda l: "_meta" in l and "results_in_the_12s_after_the_interrupt" in l["_meta"], text=120, maxlist=9)))
sq = [l for l in fx("r5-stop-three-plain") if l.get("type") == "control_response" and "still_queued" in (l["response"].get("response") or {})][0]
check(sq["response"]["response"]["still_queued"] == ["user-msg-2", "user-msg-3", "user-msg-4"], "order of still_queued")
cq = [l for l in fx("r5-stop-three-cancel") if l.get("type") == "control_response" and "cancelled" in (l["response"].get("response") or {})][0]
check(cq["response"]["response"] == {"still_queued": [], "cancelled": ["user-msg-2", "user-msg-3", "user-msg-4"]}, "cancelled list of three")
check(meta("r5-stop-three-plain", "results_in_the_12s_after_the_interrupt")[0]["results_in_the_12s_after_the_interrupt"] == 2, "one turn for the three messages")
w("""What the adapter can rely on (n=1 each):
- `still_queued` lists the waiting messages in the order they were written (`user-msg-2`, `-3`, `-4`).
- Plain interrupt: after the stopped turn's `result` the three messages start TOGETHER, in order (`started` x3), as ONE
  turn (one `system/init`, one answer `banana\\ncherry\\ndamson`, one `result` whose `user_message_uuids` has all three
  and whose `user_message_uuid` is the last), then `completed` x3. So "the next turn" after an interrupt can carry
  several messages; a host that counts one turn per waiting message is wrong.
- `cancel_queued:true`: one `cancelled` line per message, in order, all BEFORE the answer; the answer's `cancelled`
  has the same three uuids in the same order; `still_queued` is `[]`; no turn ran in the 12 s after.
""")
w("### A message that was already folded into the turn\n")
w("Turn 1 runs `sleep 4`, then `echo go; sleep 21`. A second message is written during the first call and is folded in at the tool boundary "
  "(`started` while the turn runs). The interrupt comes 2 s into the second call. Plain `interrupt` (lifecycle lines, tool calls, the interrupt and what follows):\n")
w(block(tl("r5-stop-folded-plain", lambda l: kind(l) in ("sent", "control_response", "result", "command_lifecycle") or has_tool_use(l),
           stop=lambda l: "_meta" in l and "results_in_the_12s_after_the_interrupt" in l["_meta"], text=120, maxlist=9)))
for v in ("plain", "cancel"):
    n = "r5-stop-folded-" + v
    life = meta(n, "results_in_the_12s_after_the_interrupt")[0]
    check(life["results_in_the_12s_after_the_interrupt"] == 1 and life["lifecycle"][-2:] == [["m2", "cancelled"], ["m1", "cancelled"]], "folded message of " + n)
fc = [l for l in fx("r5-stop-folded-cancel") if l.get("type") == "control_response" and "still_queued" in (l["response"].get("response") or {})][0]
w("With `\"cancel_queued\":true` the lines are the same; the answer is:\n")
w(block([j(fc)]))
w("""What the adapter can rely on (n=1 each):
- A folded message is part of the turn. The interrupt answer does not name it (`still_queued:[]`; with
  `cancel_queued`, `cancelled:[]` too). It gets `command_lifecycle` `cancelled` just before the stopped turn's
  `result`, and the turn's own message gets `cancelled` just after. The `result.user_message_uuids` has both.
- It is NOT put back and does not run again: no turn started in the 12 s after the interrupt, and the next message
  (`alive`) was answered without the folded request (`banana` was not said). The bundle's `put_back` /
  `requeue_if_interrupted_within_ms` did not act here (the interrupt came 2.7 s after the message was taken).
- For the host: `cancelled` for a message that had `started` during a running turn means "the turn that took it was
  stopped", not "the CLI dropped it unread". Whether to send it again is the host's choice; the model had seen it.
""")

# ================================================================= 5
w("## 5. Sandbox on with `autoAllowBashIfSandboxed:false`: do `git push` and `curl` ask?\n")
w("Question: in `acceptEdits` and in the app's `manual` mode (the CLI's `default`), do the commands ask as they do with the sandbox off?\n")
N5 = ["r5-commands-off", "r5-commands-noauto", "r5-commands-manual-off", "r5-commands-manual-noauto"]
w("Fixtures: `r5-commands-off` (%s), `r5-commands-noauto` (%s), `r5-commands-manual-off`, `r5-commands-manual-noauto` (the same two with `--permission-mode manual`). "
  "The same eight turns in each run, one command per turn; every request is allowed (a request to run outside the sandbox would be denied; none came). "
  "`git push` goes to a bare repo inside the work folder.\n" % (flags(N5[0]), flags(N5[1])))
ROWS = ["echo hi > note.txt", "git add note.txt && git commit -m note", "git push origin HEAD", "curl -sI https://example.com", "chmod +x hello.txt",
        "rm hello.txt (a tracked file)", "Write tool: .claude/settings.local.json", "echo '{  }' > .claude/settings.local.json"]


def cell(name, turn):
    r = [m for m in meta(name, "can_use_tool_requests") if m.get("turn") == turn][0]["can_use_tool_requests"]
    if not r:
        return "no request"
    return " + ".join("asks" + (" (`%s`)" % x[2] if x[2] else "") if x[0] != "SandboxNetworkAccess" else "network asks (`%s`)" % x[1] for x in r)


w("| # | command | acceptEdits, sandbox off | acceptEdits, sandbox + no auto-allow | manual, sandbox off | manual, sandbox + no auto-allow |")
w("|---|---|---|---|---|---|")
for i, row in enumerate(ROWS, 1):
    w("| %d | `%s` | %s |" % (i, row, " | ".join(cell(n, i) for n in N5)))
w("\n(In brackets: the request's `decision_reason_type`, when it has one. \"network asks\" is the sandbox's own `SandboxNetworkAccess` request, part 2, scenario 14.)\n")
for i in range(1, 9):
    a, b = cell(N5[0], i), cell(N5[1], i)
    check(b == a or b == a + " + network asks (`example.com`)", "sandbox + no auto-allow asks as sandbox off, turn %d" % i)
    check(cell(N5[2], i).startswith("asks") and cell(N5[3], i).startswith("asks"), "manual mode asks, turn %d" % i)
w("The two requests of the `curl` turn with the sandbox on (`r5-commands-noauto`):\n")
w(block([x for x in reqs(N5[1]) if "curl" in x or "SandboxNetworkAccess" in x][:4]))
w("What was true on disk at the end of each run (driver notes, in the order of the table's columns):\n")
w(block([metas(n, "commits_here")[0] for n in N5]))
w("""What the adapter can rely on (n=1 per column):
- With `autoAllowBashIfSandboxed:false` the sandbox changes nothing about WHICH Bash commands ask. In `acceptEdits`
  the same six of eight turns ask with and without the sandbox (`git commit`, `git push`, `curl`, `chmod +x`, both
  writes to the local settings file), with the same `decision_reason_type`; `echo hi > note.txt` and `rm` of a tracked
  file ask in neither. In `manual` all eight ask in both.
- The one difference: with the sandbox on, `curl` asks TWICE: first the Bash command itself (as without the
  sandbox), then, while it runs, the sandbox's `SandboxNetworkAccess` request for the host.
- `git commit` and `git push` to a bare repo inside the work folder WORK inside the sandbox once allowed (2 commits
  in the work repo and in the bare repo in all four runs).
- With the sandbox on, the SHELL write to `.claude/settings.local.json` did not happen although its request was
  allowed: the command failed with `operation not permitted` (the sandbox protects the file). The Write tool is not
  sandboxed: its write went through in all four runs. `settings_local` in the notes shows it: `{ }` (the Write tool's
  content) at the end of the sandbox runs, `{  }` (the shell's) at the end of the others.
""")
for n in N5:
    check(meta(n, "commits_here")[0]["settings_local"].strip() == ("{ }" if "noauto" in n else "{  }"), "who wrote the local settings in " + n)
    check(meta(n, "commits_here")[0]["commits_in_the_bare_repo"] == "2", "the push arrived in " + n)
w("The shell write to the local settings file with the sandbox on (request, answer, tool result):\n")
w(block(tl(N5[1], lambda l: kind(l) in ("control_request", "sent") and "settings.local" in json.dumps(l) and "echo" in json.dumps(l) or (is_tool_result(l) and "not permitted" in json.dumps(l)), text=160)[-6:]))

# ================================================================= 6
w("## 6. Sandbox network: a second command to the same host\n")
w("Question: after the first request for a host was allowed, does a second command to the same host in the same session ask again? What do `network_allowed_domains` and `updatedPermissions` show?\n")
N6 = ["r5-sandbox-network-once", "r5-sandbox-network-rule-local", "r5-sandbox-network-rule-session"]
w("Fixtures: `r5-sandbox-network-once` (the request is answered with a plain allow: what \"allow once\" sends), `r5-sandbox-network-rule-local` "
  "(allow + `updatedPermissions` = the request's own suggestion), `r5-sandbox-network-rule-session` (the same with `\"destination\":\"session\"`). "
  "Flags: %s. Turn 1 and turn 2 run `curl -sI https://example.com`, turn 3 `curl -sI https://example.org`.\n" % flags(N6[0]))
w("`once`: every `can_use_tool` request with its answer, and the driver's note per turn:\n")
w(block(tl(N6[0], lambda l: kind(l) == "control_request" or (kind(l) == "sent" and l["_sent"].get("type") == "control_response") or (kind(l) == "meta" and "turn" in l["_meta"]), maxlist=9)))
for n in N6:
    per_turn = [len(m["can_use_tool_requests"]) for m in meta(n, "can_use_tool_requests")]
    check(per_turn == [1, 0, 1], "requests per turn of " + n)
    for l in fx(n):
        r = (l.get("response", {}).get("response") or {}) if l.get("type") == "control_response" else {}
        if "restrictions" in r:
            check(r["restrictions"]["network_allowed_domains"] == [], "network_allowed_domains stays empty in " + n)
w("The answers with `updatedPermissions`, and what `list_permission_rules` and the local settings file show after turn 1:\n")
for n in N6[1:]:
    ans = [l for l in fx(n) if "_sent" in l and l["_sent"].get("type") == "control_response" and "updatedPermissions" in json.dumps(l)][0]
    rules = [l for l in fx(n) if l.get("type") == "control_response" and "state" in (l["response"].get("response") or {})][0]
    w(block([j(ans["_sent"]), j(rules, keep=("rules",)), metas(n, "settings_local_json", maxlist=9)[0]]))
w("""What the adapter can rely on (n=3 for the first point: the three runs agree):
- A host that was allowed is not asked about again in the same session (process): turn 2 ran the same `curl` with NO
  request in all three runs, also after a plain allow with no `updatedPermissions`. So "allow once" on a network
  request lasts until the session's process ends, not for one command. Another host still asks (turn 3).
- `get_sandbox_dialog.restrictions.network_allowed_domains` stayed `[]` in every reading, after a plain allow and
  after a saved rule: it does not show what was allowed by answering requests.
- `updatedPermissions` with the request's suggestion as sent (`destination:"localSettings"`) writes
  `{"permissions":{"allow":["WebFetch(domain:example.com)"]}}` to the work folder's `.claude/settings.local.json`, and
  `list_permission_rules` shows `{"behavior":"allow","source":"localSettings","rule":"WebFetch(domain:example.com)"}`.
  With `destination:"session"` the rule's source is `session` and no file is written.
- Not recorded: whether a NEW process in the same folder asks again after the plain allow (expected: yes) and after
  the saved rule (the rule is in the file the next process reads with `--setting-sources …local`).
""")

# ================================================================= 7
w("## 7. Effort: `apply_flag_settings` on a process started with `--effort high`\n")
w("Question: does `get_settings` show the new level, does the next turn use it, and what does `effortLevel:null` do on such a process?\n")
N7 = ["r5-effort-started-high", "r5-effort-started-none"]
w("Fixtures: `r5-effort-started-high` (%s), `r5-effort-started-none` (control, no `--effort`). Each level gets two turns (a small counting puzzle); "
  "`get_settings` is read before and after each turn.\n" % flags(N7[0]).replace("/var/folders/xx/yy/T/codly-r5-capture/raw/", "<capture>/raw/"))
w("Started with `--effort high`: the settings requests around the first change, and around `null`:\n")
c7 = control(N7[0])
gs = [i for i in range(0, len(c7), 2) if '"apply_flag_settings"' in c7[i]]
w(block(c7[gs[0] - 2:gs[0] + 4] + c7[gs[1] - 2:gs[1] + 4]))
w("The driver's notes, one per turn (`thinking_tokens` and `output_tokens` are from the turn's `result.usage`):\n")
w("| process started with | step | `applied.effort` before the turn | after the turn | thinking tokens | output tokens |")
w("|---|---|---|---|---|---|")
for n, label in zip(N7, ("`--effort high`", "no `--effort`")):
    for m in meta(n, "level"):
        w("| %s | %s | `%s` | `%s` | %s | %s |" % (label, m["level"], m["applied_effort_before_the_turn"], m["applied_effort_after_the_turn"], m["thinking_tokens"], m["output_tokens"]))
w("")
h7 = meta(N7[0], "level")
check([m["applied_effort_before_the_turn"] for m in h7] == ["high", "high", "low", "low", "medium", "medium", "high", "high"], "applied.effort of the --effort high run")
check(all(m["applied_effort_before_the_turn"] == m["applied_effort_after_the_turn"] for n in N7 for m in meta(n, "level")), "a turn does not change applied.effort")
check(all(meta(n, "debug_log_lines")[0]["debug_log_lines_that_name_an_effort_level"] == 0 for n in N7), "the debug log names no effort level")
w("""What the adapter can rely on (n=1 process per start; 8 turns each):
- On a process started with `--effort high`, `get_settings.applied.effort` is `high` with `effective:{}` and no
  `sources` entry: the flag's level is not a "setting".
- `apply_flag_settings {"settings":{"effortLevel":"low"}}` on that process wins over the start flag:
  `applied.effort` is `low` at once and stays `low` through the next turns.
- `{"effortLevel":null}` on that process does NOT go back to the level of the start flag: `applied.effort` is
  `medium` (the default), the same value as on the process started without `--effort`. After one
  `apply_flag_settings` the `--effort` of the start is gone for good; to get `high` back the host must send
  `effortLevel:"high"` (recorded: `high` again).
- "Does the next turn use it": NOT directly verified. No line of the stream names the effort of a model request
  (`init` has none, `result` has none), and the CLI's debug log (`--debug-file`, %d and %d lines) names no effort
  level either. What is recorded: `applied.effort` has the new level before and after each turn. The thinking tokens
  of the puzzle turns do not separate the levels (16 turns, 34 to 92 tokens, the ranges overlap), so they prove
  nothing either way.
""" % tuple(meta(n, "debug_log_lines")[0]["debug_log_lines"] for n in N7))

# ================================================================= 8
w("## 8. Model: after `set_model` during a turn; `init.model` against `assistant.message.model`\n")
w("Question: the `message.model` of the assistant lines still in flight and of the next turn; a turn after `set_model` \"default\"; `system/init.model` against `assistant.message.model` when the session is started with a dated id and with an alias.\n")
N8 = "r5-model-midturn"
w("Fixtures: `r5-model-midturn` (%s), and four one-turn starts: `r5-model-start-alias` (`--model haiku`), `r5-model-start-full` (`--model claude-haiku-5-5`), "
  "`r5-model-start-dated` (`--model claude-haiku-4-5-20251001`), `r5-model-start-undated` (`--model claude-haiku-4-5`).\n" % flags(N8))
w("`set_model sonnet` while the Bash call of turn 1 runs (started on haiku), then turn 2, `set_model default` and turn 3, `set_model haiku` and turn 4. "
  "Requests, init lines, assistant lines and the driver's note per turn:\n")
w(block(tl(N8, lambda l: (kind(l) == "sent" and l["_sent"].get("request", {}).get("subtype") != "get_settings" and l["_sent"].get("type") != "user")
           or (kind(l) == "control_response" and "model" in (l["response"].get("response") or {})) or sub(l) == "init" or kind(l) == "assistant"
           or (kind(l) == "user" and l.get("isReplay")) or (kind(l) == "meta" and "turn" in l["_meta"]), text=60, maxlist=9)))
t8 = {m["turn"]: m for m in meta(N8, "turn")}
check(t8["turn 1"]["assistant_message_model"] == ["claude-haiku-5-5", "claude-sonnet-5-5"] and t8["turn 1"]["init_model"] == ["claude-haiku-5-5"], "models of turn 1")
check(t8["turn 2"]["init_model"] == ["claude-sonnet-5-5"] and t8["turn 3"]["assistant_message_model"] == ["claude-opus-5-5"] and t8["turn 4"]["init_model"] == ["claude-haiku-5-5"], "models of turns 2 to 4")
w("The four starts: `applied.model` of `get_settings`, the init line's model, the assistant lines' model and the keys of `result.modelUsage`:\n")
w("| `--model` | `get_settings.applied.model` | `system/init.model` | `assistant.message.model` | `result.modelUsage` keys |")
w("|---|---|---|---|---|")
for v, arg in (("alias", "haiku"), ("full", "claude-haiku-5-5"), ("dated", "claude-haiku-4-5-20251001"), ("undated", "claude-haiku-4-5")):
    n = "r5-model-start-" + v
    m = meta(n, "turn")[0]
    ap = [l["response"]["response"]["applied"]["model"] for l in fx(n) if l.get("type") == "control_response" and "applied" in (l["response"].get("response") or {})]
    check(len(set(ap)) == 1 and len(set(m["assistant_message_model"])) == 1, "one model name per column in " + n)
    w("| `%s` | `%s` | `%s` | `%s` | `%s` |" % (arg, ap[0], m["init_model"][0], m["assistant_message_model"][0], "`, `".join(m["result_modelUsage_keys"][0])))
w("")
u8 = meta("r5-model-start-undated", "turn")[0]
check(u8["init_model"] == ["claude-haiku-4-5"] and set(u8["assistant_message_model"]) == {"claude-haiku-4-5-20251001"}, "undated start: init and assistant differ")
w("""What the adapter can rely on (n=1 each):
- `set_model` during a turn takes effect at the NEXT model request of that same turn: the assistant line written
  before the switch says `claude-haiku-5-5`, the one after the tool result says `claude-sonnet-5-5`. The turn's
  `system/init` (written at its start) still says haiku; `result.modelUsage` has both models. `get_settings`
  answered the new model at once, mid-turn.
- Each `set_model` also writes a `user` line with `"isReplay":true` and the text
  `<local-command-stdout>Set model to …</local-command-stdout>`, just before the answer.
- The next turn's `system/init.model` is the new model. After `set_model "default"` the turn ran on
  `claude-opus-5-5` (`init.model`, `message.model` and a new `modelUsage` key all say so); after `set_model haiku`
  on haiku again.
- `init.model` and `message.model` are the same string for `haiku`, for `claude-haiku-5-5` and for a dated id. They
  DIFFER for an undated name of a dated model: started with `--model claude-haiku-4-5`, `init.model`,
  `applied.model` and the `modelUsage` key are `claude-haiku-4-5`, but every assistant line says
  `claude-haiku-4-5-20251001`. A host that compares `message.model` with `init.model` to notice "the turn ran on
  another model" gets a false alarm on such a session; the two names must be compared as "same model" when one is
  the other plus a `-YYYYMMDD` date.
- The alias `haiku` gives `claude-haiku-5-5` everywhere: no line keeps the alias.
""")

# ================================================================= 9
w("## 9. `--resume` with an unknown session id, and `initialize` written at once\n")
w("Question: the order of the `initialize` answer and the exit over 5 runs, and what the last lines say.\n")
N9 = ["r5-resume-missing-%d" % i for i in range(1, 6)]
w("Fixtures: `r5-resume-missing-1` … `-5`. Flags: %s. The host writes `initialize` right after the spawn and nothing else.\n" % flags(N9[0]))
w("Everything of the first run after the first line:\n")
w(block([j(l, maxlist=9) for l in fx(N9[0])[1:] if set(l) != {"_t"}]))
T9 = []
for n in N9:
    m = meta(n, "stdout_lines_in_order")[0]
    check(m["stdout_lines_in_order"] == ["result/error_during_execution"] and m["exited_by_itself"], "lines of " + n)
    check([l["_meta"]["exit_code"] for l in fx(n) if "_meta" in l and "exit_code" in l["_meta"]] == [1], "exit code of " + n)
    T9.append([l["_t"] for l in fx(n) if "_stderr" in l][0])
w("""What the adapter can rely on (n=5, the five runs agree):
- The `initialize` request is NEVER answered: stdout has one line, the error `result`
  (`subtype:"error_during_execution"`, `is_error:true`, `num_turns` 0,
  `errors:["No conversation found with session ID: <id>"]`), stderr has the same sentence, and the process exits by
  itself with code 1. The stderr line came %s s after the spawn.
- So for this failure a start cannot "succeed first": a host that waits for the `initialize` answer sees the
  process end instead. (An `initialize` answer followed by this error was not seen in 5 runs. A host that does not
  wait for the answer, and treats "spawned" as "started", is the one that can see a start succeed and then fail.)
- The `result` line's `session_id` is the id that was asked for.
""" % ", ".join("%.2f" % t for t in T9))

# ================================================================= 10
w("## 10. MCP: a successful `mcp_reconnect`; `mcp_toggle` off and on\n")
w("Question: the answer to a successful `mcp_reconnect`; the answers to `mcp_toggle` and the `mcp_status` after them.\n")
N10 = "r5-mcp-reconnect-toggle"
w("Fixture: `%s`. Flags: %s. The server `ping-flaky` (the drivers folder's `mcp_ping_server.py` with `need=switch.on`) exits at once while the file "
  "`switch.on` is missing; the driver creates the file between the two `mcp_reconnect` requests.\n" % (N10, flags(N10)))
w("Every control request with its answer, the driver's notes, and the init line of the turn at the end:\n")
w(block(tl(N10, lambda l: kind(l) in ("sent", "control_response", "meta") and not (kind(l) == "sent" and l["_sent"].get("type") != "control_request") or sub(l) == "init", maxlist=9)))
c10 = [l["response"] for l in fx(N10) if l.get("type") == "control_response"]
check([r["subtype"] for r in c10 if "response" not in r] == ["error", "success", "success", "success"], "answers to mcp_reconnect and mcp_toggle")
st10 = [r["response"]["mcpServers"][0]["status"] for r in c10 if "mcpServers" in (r.get("response") or {})]
check(st10 == ["failed", "failed", "connected", "disabled", "connected"], "mcp_status sequence")
w("""What the adapter can rely on (n=1):
- A stdio server that exits at start: `mcp_status` says `"status":"failed"`, `"error":"Connection closed"`.
- `mcp_reconnect {"serverName"}` while it still fails: a control ERROR, `"error":"Connection closed"` (the connect
  error text, as in part 2 for the http server).
- `mcp_reconnect` that works: `{"subtype":"success"}` with NO inner `response`. The `mcp_status` after it says
  `"status":"connected"` with `serverInfo` and `tools`. The answer came after the server was up (the status asked
  right after it was already `connected`).
- `mcp_toggle {"serverName","enabled":false}`: success with no inner response, status `disabled` (no `serverInfo`,
  no `tools`). `mcp_toggle` with `enabled:true`: success with no inner response, status `connected` again; the
  server process was started again (the start counter in the notes goes up by one).
- So the answers carry no state: success = "done", error = "the connect failed, here is why". The state is in
  `mcp_status` (and in the next turn's `system/init.mcp_servers`: `connected`, tool in `init.tools`).
- Every start, reconnect and enable spawns the server command again; a failing connect spawned it twice here
  (the counter went 2, 4, 5, 5, 6).
""")

# ================================================================= 11
w("## 11. MCP sources: the `mcp_server` object of `can_use_tool`\n")
w("Question: `request.mcp_server` (`name`, `source`) for a server from `--mcp-config`, from the project's `.mcp.json`, and from user settings; and a name clash between `--mcp-config` and `.mcp.json`.\n")
N11 = ["r5-mcp-source-config", "r5-mcp-source-clash", "r5-mcp-source-user"]
w("Fixtures: `r5-mcp-source-config` (%s), `r5-mcp-source-clash` (%s; `--mcp-config` and `.mcp.json` both define `ping-same`), `r5-mcp-source-user` "
  "(%s, a throw-away HOME whose `.claude.json` defines `ping-user`; not logged in). A server from `.mcp.json` alone is part 2, scenario 19 "
  "(`\"source\":\"project\"`).\n" % tuple(flags(n) for n in N11))
w("`--mcp-config`: the init entry, the request, the result:\n")
w(block(tl(N11[0], lambda l: sub(l) == "init" or kind(l) == "control_request" or is_tool_result(l))))
w("Name clash. The two definitions answer `pong from config` and `pong from project`, and each writes its own start file:\n")
w(block(tl(N11[1], lambda l: sub(l) == "init" or kind(l) == "control_request" or is_tool_result(l) or (kind(l) == "meta" and "server_starts" in l["_meta"]), maxlist=9)))
m11 = meta(N11[1], "server_starts")[0]
check(m11["server_starts"] == {"started-config.txt": 1} and m11["result"] == "pong from config" and m11["can_use_tool_requests"][0]["mcp_server"] == {"name": "ping-same", "source": "dynamic"},
      "the --mcp-config definition wins the clash")
w("A server of the user's own `~/.claude.json` (throw-away HOME, no login, so no tool call):\n")
w(block(tl(N11[2], lambda l: sub(l) == "init" or kind(l) in ("sent", "control_response", "meta", "result") and not (kind(l) == "sent" and l["_sent"].get("type") == "user"), maxlist=9, text=80)))
w("""What the adapter can rely on (n=1 each):
- A server from `--mcp-config`: `"mcp_server":{"name":"ping-config","source":"dynamic"}`; the same word in
  `init.mcp_servers[].source` and in `mcp_status` (`scope` and `source` are both `dynamic`).
- A server from the project's `.mcp.json`: `"source":"project"` (part 2).
- A server from the user's `~/.claude.json`: `init.mcp_servers` and `mcp_status` say `"source":"user"`
  (`"scope":"user"`). The REQUEST for such a server is not recorded (no login in a throw-away HOME). In the two
  recorded cases the request's `source` is the init entry's `source`, so `user` is the expected word, not a recorded one.
- Name clash: the `--mcp-config` definition wins and the project's is not even started (only `started-config.txt`
  exists; the tool answered `pong from config`). There is ONE server of that name, and both the init entry and the
  request say `"source":"dynamic"`. Nothing in the stream says that a project definition was shadowed.
- So `source` tells Codly's own servers and the Connections (`dynamic`: they come from `--mcp-config`) from the
  project's (`project`) and the user's (`user`) by a field, not by a name: a project server cannot pass as `dynamic`
  by taking the name of one.
""")

# ================================================================= 12
w("## 12. A project `.mcp.json` server with the app's own flags\n")
w("Question: with `--setting-sources user,project` (no `--strict-mcp-config`), is an unapproved `.mcp.json` server loaded headless? Do `enableAllProjectMcpServers` / `disabledMcpjsonServers` change it?\n")
V12 = ["plain", "flag-enable-all-false", "flag-enabled-other", "flag-disabled", "user-disabled", "login-flag-disabled"]
N12 = ["r5-project-mcp-" + v for v in V12]
w("Fixtures: `r5-project-mcp-plain` (%s), and the same with one setting each: `-flag-enable-all-false`, `-flag-enabled-other`, `-flag-disabled` "
  "(`--settings`), `-user-disabled` (the key in `~/.claude/settings.json`). These five use a throw-away HOME (no login). `-login-flag-disabled` has the real "
  "login: %s.\n" % (flags(N12[0]), flags(N12[5])))
w("`plain`: the driver's notes, `mcp_status`, the init line:\n")
w(block(tl(N12[0], lambda l: sub(l) == "init" or kind(l) in ("sent", "control_response", "meta") and not (kind(l) == "sent" and l["_sent"].get("type") == "user"), maxlist=9)))
w("| run | the setting | where | server started | init entry |")
w("|---|---|---|---|---|")
SET12 = {"plain": ("none", "-"), "flag-enable-all-false": ('`{"enableAllProjectMcpServers":false}`', "`--settings`"),
         "flag-enabled-other": ('`{"enabledMcpjsonServers":["another-server"]}`', "`--settings`"),
         "flag-disabled": ('`{"disabledMcpjsonServers":["ping-project"]}`', "`--settings`"),
         "user-disabled": ('`{"disabledMcpjsonServers":["ping-project"]}`', "`~/.claude/settings.json`"),
         "login-flag-disabled": ('`{"disabledMcpjsonServers":["ping-project"]}`', "`--settings`, real login, `--setting-sources project`")}
for v, n in zip(V12, N12):
    m = meta(n, "server_starts")[0]
    check((m["server_starts"] > 0) == (v in ("plain", "flag-enable-all-false", "flag-enabled-other")), "server start of " + n)
    check(bool(m["init_entry_for_this_server"]) == (m["server_starts"] > 0) and m["other_servers_in_init"] == 0, "init entry of " + n)
    w("| `%s` | %s | %s | %s | %s |" % (v, SET12[v][0], SET12[v][1], "yes" if m["server_starts"] else "no",
                                    "`%s`" % j(m["init_entry_for_this_server"][0]) if m["init_entry_for_this_server"] else "none"))
w("")
w("""What the adapter can rely on (n=1 per row; five rows without a login):
- Yes: with the app's flags the headless CLI loads a `.mcp.json` server that nobody approved, in a HOME that has never
  seen the folder. It was started within 3 s of the spawn, before any message (`server_started:true`), and is
  `connected` with `"source":"project"`. Part 2 saw the same with `--setting-sources project` and the real HOME.
- `enableAllProjectMcpServers:false` does not keep it out, and neither does an `enabledMcpjsonServers` list that
  does not name it: in `-p` mode there is no approval step for these keys to answer.
- `disabledMcpjsonServers:["<name>"]` DOES keep it out, from `--settings` and from the user's settings file: the
  server was not started and is in neither `init.mcp_servers` nor `init.tools`. The same with the real login
  (`login-flag-disabled`: the model answered `none`).
- So the app can keep a project server out without `--strict-mcp-config`: read the names from the folder's
  `.mcp.json` and pass them as `disabledMcpjsonServers` in its `--settings`.
- Weakness of this section: five of six runs had no login. Loading MCP servers happens before the first model
  request, and the one logged-in run agrees, but "not logged in" is still a difference from a real run.
""")

# ================================================================= 13
w("## 13. `acceptEdits`: `git commit`, `chmod +x`, `rm` of a tracked file, a write to `.claude/settings.local.json`\n")
w("Question: which of these ask?\n")
w("Fixture: `r5-commands-off` (the first column of the table in section 5; the sandbox column next to it gives the same answers). Flags: %s.\n" % flags(N5[0]))
w("Every `can_use_tool` request of the run (the answers are plain allows):\n")
w(block([x for i, x in enumerate(reqs(N5[0])) if i % 2 == 0]))
w("""What the adapter can rely on (n=1; the sandbox run agrees):
- `git add … && git commit …` asks (`decision_reason_type:"subcommandResults"`).
- `git push`, `curl` and `chmod +x` ask (`decision_reason_type:"other"`).
- `rm` of a tracked file and a shell redirect into a new file of the work folder do NOT ask (as `mv`, `rm`, `mkdir`
  in part 2, scenario 9).
- A write to the work folder's `.claude/settings.local.json` asks whether it comes from the Write tool or from a
  shell redirect (`decision_reason_type:"safetyCheck"`; part 2 had the Edit tool).
- The requests carry `permission_suggestions`; for the shell write the first one would add the exact command as an
  allow rule to the local settings (see the fixture).
""")

# ================================================================= 14
w("## 14. `--max-budget-usd` across `--resume`\n")
w("Question: does the budget count the cost of the session before the resume?\n")
N14 = ["r5-budget-resume-" + x for x in "abcd"]
w("Fixtures: `r5-budget-resume-a` (a new session, no budget, five turns), `-b` (`--resume`, no budget, one turn), `-c` (`--resume` with a budget of twice "
  "one turn's cost), `-d` (`--resume` with a budget below one model request). Flags of run c: %s. Every turn asks for one tool call (`echo one`) and a "
  "one-word answer.\n" % flags(N14[2]))
w("The driver's note of each run, and the last `result` of runs c and d (with `total_cost_usd` and `errors`):\n")
w(block([metas(n, "run")[0] for n in N14]))


def res14(n):
    r = results(n)[-1]
    return j(slim(r) | {"total_cost_usd": r.get("total_cost_usd"), "model_usage_since_last_result": {k: {"costUSD": v.get("costUSD")} for k, v in (r.get("model_usage_since_last_result") or {}).items()}},
             keep=("model_usage_since_last_result",))


w(block([res14(N14[2]), res14(N14[3])]))
a14, b14, c14, d14 = [meta(n, "run")[0] for n in N14]
check(b14["total_cost_usd"] > a14["total_cost_usd"] and b14["session_id_is_the_resumed_one"], "the total goes on after --resume")
check(c14["result_subtype"] == "success" and c14["budget_usd"] < b14["total_cost_usd"] and c14["total_cost_usd"] > c14["budget_usd"], "run c passes below the carried total")
check(d14["result_subtype"] == "error_max_budget_usd", "run d hits the limit")
w("""What the adapter can rely on (n=1):
- NO: the budget counts only what THIS process spends. Run c started with a budget of %.6f USD on a session that
  had already cost %.6f USD, and its turn (a tool call and an answer: two model requests, %.6f USD) ended as a
  normal success.
- But `total_cost_usd` (and `modelUsage`) of a resumed process GO ON from the session's earlier total: run b's first
  result says %.6f, run a's last said %.6f. So after a `--resume` the `total_cost_usd` of a `result` can be above
  the budget while the budget is not reached: run c's result says %.6f against a budget of %.6f.
  `model_usage_since_last_result` is the turn alone.
- A host that mirrors the limit must count from the first `total_cost_usd` it sees after a resume, minus that turn,
  not from zero.
- The control (run d, budget 0.00002): `error_max_budget_usd`, `errors:["Reached maximum budget ($0.00002)"]`, after
  the first model request of the turn. That request was a plain text answer, not the tool call: the model wrote that
  "the budget is down to $0.00002" and did not run the command. So the model knows the budget figure (how it is
  told is not in the stream), and the limit is also looked at after a request that ends the turn (in part 1 it hit
  after a `tool_use`).
""" % (c14["budget_usd"], b14["total_cost_usd"], c14["cost_of_the_last_turn_alone"], b14["total_cost_usd"], a14["total_cost_usd"], c14["total_cost_usd"],
       c14["budget_usd"]))
check(c14["cost_of_the_last_turn_alone"] < c14["budget_usd"], "run c's own spend is below its budget")
check(any("budget is down to" in json.dumps(l) for l in fx(N14[3]) if l.get("type") == "assistant"), "run d: the model names the budget")

# ================================================================= 15
w("## 15. Prompt wording: is a background command stopped before the report?\n")
w("Question: does a worker that is told \"Codly waits up to 5 minutes for what you left running in the background and then stops it: stop it yourself before "
  "your report unless the result is needed\" end its turn with its background command stopped?\n")
N15 = ["r5-background-wording-rule-%d" % i for i in (1, 2, 3)] + ["r5-background-wording-plain-%d" % i for i in (1, 2, 3)]
w("Fixtures: `r5-background-wording-rule-1` … `-3` (the sentence is in `--append-system-prompt`), `r5-background-wording-plain-1` … `-3` (control: the same "
  "system prompt without it). Tools: `Bash,Read,TaskStop`. The task: start a script that writes one line per second and never ends, and report the first line of its output.\n")
w(block([j(x) for x in sent(N15[0], "user")[:1]]))
w("| run | tool calls of the turn | background task still listed at the end of the turn | script still runs |")
w("|---|---|---|---|")
for n in N15:
    m = meta(n, "at_the_end_of_the_turn")[0]
    end = m["at_the_end_of_the_turn"]
    check(bool(end["background_tasks_still_listed"]) == end["tick_script_still_runs"] == ("plain" in n), "end of the turn of " + n)
    check(("TaskStop" in m["tool_calls"]) == ("rule" in n), "TaskStop call of " + n)
    w("| `%s` | %s | %s | %s |" % (n[len("r5-background-wording-"):], ", ".join(m["tool_calls"]), "yes" if end["background_tasks_still_listed"] else "no",
                               "yes" if end["tick_script_still_runs"] else "no"))
w("")
w("The stop in the first run with the sentence (the model's `TaskStop` call and the lines it causes):\n")
w(block(tl(N15[0], lambda l: (has_tool_use(l) and "TaskStop" in json.dumps(l)) or sub(l) in ("task_updated", "task_notification", "background_tasks_changed") or kind(l) == "result", text=200)[2:]))
w("""What this shows (n=3 with the sentence, n=3 without; WEAK evidence):
- With the sentence, 3 of 3 turns ended with the background command stopped: the model called `TaskStop` with the
  task id before its report (`task_updated` `killed`, `task_notification` `stopped`, list `[]`), and the script was
  gone. Without the sentence, 0 of 3: the command was still listed and still ran when the turn ended.
- This is one model (haiku), one task, one wording, six tiny runs, on one day. It says the sentence CAN work; it
  does not say how often it works on real tasks, with other models, or when the result is needed later.
""")

# ================================================================= code
w("""## What this means for the code that waited (as of the day of recording)

| # | waits | can it be built now? |
|---|---|---|
| 1 | Stop for helpers in the sheet; the gate's stop of a helper; the stop at shutdown | yes: `stop_task` works on a subagent's task id and the end lines are the usual ones. New fact to build in: a turn starts by itself after the stop |
| 2 | Reading the list for "what still runs" | yes: `failed` is the third status word; background subagents are listed, foreground helpers and commands are not |
| 3 | The watchdog's life sign (`tool_progress` arm) | yes: every recorded line has the running call's id in `parent_tool_use_id`; the arm that needs it drops nothing that was seen. A Bash call inside a helper has no heartbeat of its own: the helper's `Agent` call has |
| 4 | Shutdown: interrupt + cancel, stop tasks, close | yes for commands (recorded as one run). partly for helpers: the turn that follows a helper's stop is not in the one-run recording |
| 5 | "Workers' commands ask as before" with the sandbox | yes: the same commands ask with and without the sandbox when auto-allow is off; `curl` asks a second time for the host |
| 6 | The sentence about how long a network allow lasts | yes: "for this session": a plain allow is not asked again until the process ends |
| 7 | Dropping the restart for an effort-only switch | partly: the setting changes and `null` falls to `medium`, not to the start flag (so the host must always send a level). That the model request uses it is not visible in any line |
| 8 | The model notice | yes: compare model names with the date suffix removed; a mid-turn switch shows on the next assistant line of the same turn |
| 9 | Keeping or dropping the session id after a start that "succeeded" | yes: with `initialize` awaited, a missing session never answers it (5 of 5) |
| 10 | Reconnect and enable/disable buttons | yes: success has no body; read `mcp_status` after it |
| 11 | Asking by source instead of by name | partly: `dynamic` and `project` are recorded in requests; `user` only in `init` / `mcp_status` |
| 12 | Keeping unapproved project servers out | yes: `disabledMcpjsonServers` in `--settings` (the other two keys do nothing headless) |
| 13 | The approval rules text for workers | yes |
| 14 | The spending limit across a restart | yes: the limit is per process, the reported total is per session |
| 15 | The worker rule about background commands | partly: the wording worked 3 of 3 against 0 of 3, on one tiny task |
""")

# ================================================================= table
w("""## Table: question -> answer -> fixture

| # | question | answer | n | fixture |
|---|---|---|---|---|
| 1 | `stop_task` on a background subagent | Works: `killed` / `stopped` / list `[]`, then `{}`. The helper's tool call is rejected, the helper ends, its `sleep` is gone. The CLI then starts a turn by itself. `get_task_output` on its id is an error | 2 | `r5-stop-subagent-1`, `-2` |
| 2 | Background command with exit 1; who is listed | `task_notification.status` `failed` (`patch.status` `failed`), then list `[]`, then a self-started turn. Background subagent: listed (`task_type:"local_agent"`, `subagent_type`, `description`, ids). Foreground helper or command: never listed, unless the host sends `background_tasks` (then listed, `patch.is_backgrounded:true`) | 1 | `r5-background-fail` |
| 3 | `tool_progress` for other tools | Same heartbeat line: `tool_use_id` `<call id>-heartbeat-N`, `parent_tool_use_id` = the call id (never null), `tool_name` = the tool, at 30, 60, 90 s. MCP tool and `Agent` recorded; a Bash call inside a helper has none; WebFetch not recorded | 1 each | `r5-tool-progress-mcp`, `-subagent`, `-webfetch` |
| 4 | Stop sequence; three waiting; folded | Sequence works: 0.2 s to the stopped turn's `result` and the task's stop, exit about 1 s after stdin close, exit code 1, nothing left running. Three waiting: `still_queued` in written order; plain = ONE turn with all three; `cancel_queued` = three `cancelled` lines before the answer. Folded: not in the answer, `cancelled` with the turn, not run again | 1 each | `r5-stop-sequence`, `r5-stop-three-plain`, `-cancel`, `r5-stop-folded-plain`, `-cancel` |
| 5 | Sandbox + no auto-allow: `git push`, `curl` | Both ask, in `acceptEdits` and in `manual`, exactly as with the sandbox off; `curl` then also gets the network request | 1 per mode | `r5-commands-noauto`, `r5-commands-manual-noauto` (against `r5-commands-off`, `r5-commands-manual-off`) |
| 6 | Second command to an allowed host | Not asked again in the same process, also after a plain allow. `network_allowed_domains` stays `[]`. `updatedPermissions` saves `WebFetch(domain:…)` (local file or session) | 3 | `r5-sandbox-network-once`, `-rule-local`, `-rule-session` |
| 7 | Effort on a `--effort high` process | `get_settings` shows the new level at once; `null` gives `medium`, not the start flag's `high`. The turn's use of it: not visible | 1 | `r5-effort-started-high`, `-none` |
| 8 | Model after `set_model`; init vs message | Mid-turn switch: next assistant line of the same turn is on the new model; next `init.model` too. `default` -> opus. `init.model` = `message.model` except for an undated name of a dated model (`claude-haiku-4-5` vs `claude-haiku-4-5-20251001`) | 1 each | `r5-model-midturn`, `r5-model-start-alias`, `-full`, `-dated`, `-undated` |
| 9 | `--resume` unknown id + `initialize` | No `initialize` answer in 5 of 5: one error `result`, the stderr sentence, exit 1 | 5 | `r5-resume-missing-1` … `-5` |
| 10 | `mcp_reconnect` success; `mcp_toggle` | Success = `{"subtype":"success"}` with no body (failure = control error with the connect error). Toggle off -> `disabled`, on -> `connected` (server started again) | 1 | `r5-mcp-reconnect-toggle` |
| 11 | `mcp_server.source` by origin; name clash | `--mcp-config`: `dynamic`. `.mcp.json`: `project` (part 2). User `~/.claude.json`: `user` in `init` / `mcp_status` (request not recorded). Clash: the `--mcp-config` one runs, the project one is not started, `source` `dynamic` | 1 each | `r5-mcp-source-config`, `-clash`, `-user` |
| 12 | Project server with `--setting-sources user,project` | Loaded with no approval. `enableAllProjectMcpServers:false` and a foreign `enabledMcpjsonServers` list: still loaded. `disabledMcpjsonServers`: kept out (flag or user file) | 1 per row | `r5-project-mcp-plain`, `-flag-enable-all-false`, `-flag-enabled-other`, `-flag-disabled`, `-user-disabled`, `-login-flag-disabled` |
| 13 | `acceptEdits`: which commands ask | Ask: `git commit`, `git push`, `curl`, `chmod +x`, any write to `.claude/settings.local.json`. Do not ask: `rm` of a tracked file, a redirect into the work folder | 1 (+1 with the sandbox) | `r5-commands-off` |
| 14 | `--max-budget-usd` across `--resume` | Not counted: the limit is per process. `total_cost_usd` does go on from the earlier session | 1 | `r5-budget-resume-a` … `-d` |
| 15 | Wording: stop background work before the report | With the sentence 3 of 3 stopped it (`TaskStop`); without, 0 of 3. Weak evidence | 3 + 3 | `r5-background-wording-rule-1` … `-3`, `-plain-1` … `-3` |

## Not verified

Still open from the list this part was recorded for:
- The text printed when `failIfUnavailable` stops a start (needs a machine where the sandbox cannot start).
- `mcp_authenticate`, `mcp_clear_auth`, `mcp_oauth_callback_url` (a real sign-in; may write to the login keychain).
- `claude --resume` typed into a terminal on a session Codly started headless, with zsh prompt plugins (needs the
  app's terminal).
- Linux behaviour of process groups.

Open after these recordings:
- 1: `stop_task` on a subagent while the turn that started it still runs (a foreground wait); a subagent that has
  its own background command; a nested subagent. The stop of a helper as part of the shutdown sequence (one run).
- 2: a subagent that FAILS (its status word); a background command killed by a signal from outside.
- 3: `tool_progress` for WebFetch and WebSearch (needs a slow https page with a trusted certificate); the heartbeat
  past 120 s; a line with a null `parent_tool_use_id` (never seen: whether it exists at all).
- 4: the sequence when the interrupt arrives between two tool calls; `interrupt` with `send_now`; a folded message
  when the interrupt comes within the bundle's `requeue_if_interrupted_within_ms`.
- 5: commands other than the eight; a `git push` to a real remote (the network request then names the host).
- 6: a new process in the same folder after a plain allow; a host denied once (is it asked again?).
- 7: whether a model request really carries the level that `get_settings` shows (no line says it; a proxy that sees
  the request would).
- 8: a turn that falls back to another model by itself (`--fallback-model`); the names for sonnet and opus aliases
  with a date (`claude-opus-4-5` and the like were not started).
- 9: the same with a damaged transcript, and with `--resume` from another folder.
- 10: `mcp_reconnect` on a server that is `connected` (a restart?); a server that dies mid-session (does the status
  change by itself, is there a line?).
- 11: the `can_use_tool` request for a `user` server and for a claude.ai connector (both need the real login with
  the user's own setup); a clash between `.mcp.json` and the user's file.
- 12: the same with a login for the four no-login rows; a server from a plugin.
- 13: writes outside the work folder by a shell command; `git` commands that rewrite history.
- 14: a budget that runs out in the middle of a long turn of a resumed process (part 1 has it for a new one); what
  exactly the CLI tells the model about the budget.
- 15: everything beyond six tiny haiku runs.
""")

out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(OUT, "NOTES-part3.md")
with open(out, "w") as f:
    f.write("\n".join(P))
print("wrote", out, sum(len(x) for x in P), "characters")
