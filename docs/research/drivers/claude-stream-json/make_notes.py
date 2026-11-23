"""Part 1: builds NOTES.md from the sanitized fixtures, so every JSON line quoted there is copied from a fixture.

python3 make_notes.py [output file]     (default: $CAPTURE_DIR/NOTES.md; reads $CAPTURE_DIR/fixtures)

Shrinking rule for quoted lines (the fixture always has the full line):
  - an array with more than 3 items shows its first 2 items and "…(N more)"
  - a string longer than 320 characters is cut and ends with "…"
  - the keys in HEAVY are replaced by "…" unless a section asks for them
"""
import collections, glob, json, os

OUT = os.path.abspath(os.environ.get("CAPTURE_DIR") or os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
HEAVY = ("usage", "modelUsage", "model_usage_since_last_result", "subagent_stats")


def fx(name):
    return [json.loads(l) for l in open(os.path.join(OUT, "fixtures", name + ".ndjson"))]


def shrink(v, keep=(), maxstr=320, maxlist=3, key=None):
    if isinstance(v, dict):
        return {k: ("…" if (k in HEAVY and k not in keep) else shrink(x, keep, maxstr, maxlist, k)) for k, x in v.items()}
    if isinstance(v, list):
        if len(v) > maxlist and key not in keep:
            return [shrink(x, keep, maxstr, maxlist) for x in v[:2]] + ["…(%d more)" % (len(v) - 2)]
        return [shrink(x, keep, maxstr, maxlist) for x in v]
    if isinstance(v, str) and len(v) > maxstr:
        return v[:maxstr] + "…"
    return v


def j(o, **kw):
    return json.dumps(shrink(o, **kw), ensure_ascii=False, separators=(",", ":"))


def jd(o):
    return json.dumps(o, ensure_ascii=False, separators=(",", ":"))


def block(lines):
    return "```json\n" + "\n".join(lines) + "\n```\n"


def is_cli(l):
    return not any(k.startswith("_") for k in l)


def pick(name, pred, n=None, **kw):
    out = [j(l, **kw) for l in fx(name) if pred(l)]
    return out if n is None else out[:n]


def sent(name):
    return [l for l in fx(name) if "_sent" in l]


def timeline(name, pred, **kw):
    """Lines with their time: '# t=…' comment lines are not JSON; they give the seconds since the process started."""
    out, t = [], 0.0
    for l in fx(name):
        if set(l) == {"_t"}:
            t = l["_t"]
            continue
        if "_t" in l:
            t = l["_t"]
        if pred(l):
            out.append("# t=%.2f" % t)
            out.append(j(l, **kw))
    return out


def not_stream(l):
    return l.get("type") != "stream_event" and l.get("subtype") not in ("thinking_tokens",) and "_note" not in l


def results(name, **kw):
    return pick(name, lambda l: l.get("type") == "result", **kw)


P = []
w = P.append

w("""# Claude Code CLI stream-json: recorded protocol shapes

- CLI: `claude` 2.1.311 (`/opt/homebrew/bin/claude`, build time `2026-10-26T18:31:42Z` as reported by `get_binary_version`).
- Recorded: 2026-11-22, macOS, model alias `haiku` (`claude-haiku-5-5`), subscription login (init says `"apiKeySource":"none"`).
- Everything below was observed in these recordings. Where a statement comes from reading the CLI bundle and not from a
  recording, it says "bundle, not recorded".

## Base command

```
env -u <every CLAUDE* and ANTHROPIC_BASE_URL variable of the parent session> \\
  claude -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages \\
    --permission-prompt-tool stdio --permission-mode acceptEdits --session-id <uuid v4> \\
    --model haiku --strict-mcp-config --setting-sources local  [scenario flags]
```

- cwd: a throw-away git repo (`README.md`, `hello.txt`, one commit), shown as `/tmp/work/<scenario>`.
- `--setting-sources local` keeps the user's own skills, plugins, hooks and MCP servers out. The app uses `user,local`,
  so its init line has longer lists.
- Scenario flags are in the first line (`_argv`) of each fixture.
- The driver answers every `can_use_tool` with `{"behavior":"allow","updatedInput":<the request's input>}` unless a
  section says otherwise.

## Files

- `fixtures/<scenario>.ndjson`: sanitized recordings. Line kinds:
  - `{"_note":…,"_argv":[…],"_cwd":…}` first line.
  - `{"_t":12.3}` time marker (seconds since the process started), written when the time moved on by 0.25 s or more.
  - `{"_sent":{…},"_t":…}` a line the host wrote to stdin.
  - `{"_stderr":"…","_t":…}` a stderr line. `{"_meta":{…},"_t":…}` a driver note (stdin closed, exit code, files on disk).
  - every other line: one stdout line of the CLI.
- `raw/<scenario>.ndjson`: unsanitized, `{"t":…,"dir":"recv|send|stderr|meta","msg":…}`. Not for the repo.
- `drivers/`: `drv.py` (driver), `sNN_*.py` (one per scenario), `sanitize.py`, `make_notes.py` (builds this file), `tl.py` (timeline viewer).

## Sanitizing

- Paths: the work dir is `/tmp/work/<scenario>`, the CLI's temp dir is `/tmp/claude-tmp/…`, the home dir is `/Users/me`.
- Ids are stable placeholders per fixture: `sess-N` (session), `req-N` (CLI request id), `host-N` (my request ids, unchanged),
  `msg-N`, `toolu-N`, `apireq-N`, `tid-N` (task and agent ids), `run-N`, `hook-N`, `user-msg-N` (uuid the host put on a user
  message), `conv-N` (`new_conversation_id`), `uuid-N` (every other uuid), `sig-N` (thinking signatures). Real ids are uuid v4, `msg_…`, `toolu_…`, `req_…`.
- Shortened in every `system/init` line: `slash_commands` (kept `clear`, `compact`), `terminal_slash_commands` (1 kept),
  `skills` (emptied), `plugins` (emptied), `agents` (kept `general-purpose`). `memory_paths` keeps its one key `auto`
  with a placeholder path. `mcp_servers` is real (empty, except in the broken-mcp scenario).
- Shortened in control responses: `initialize.commands` (2 kept), `initialize.agents` (1 kept), `models` (kept `default`,
  `sonnet`, `haiku`), `available_output_styles` (1 kept), `account` values, `pid`, `get_status` login rows,
  `get_usage.rate_limits` (kept `five_hour`), `get_usage.behaviors`, `gridRows` of the context usage,
  `list_permission_rules` rules of source `toolsNarrowing` (2 kept).
- `commands_changed.commands` keeps 2 entries. The `/cost` text keeps its first lines only.
- Streamed deltas that carried a piece of a path were joined, cleaned and cut again into the same number of deltas.
- In this file only: arrays longer than 3 items show 2 items and `"…(N more)"`, strings longer than 320 characters end
  with `…`, and `usage`, `modelUsage`, `model_usage_since_last_result`, `subagent_stats` show as `"…"` unless the section
  is about them. Lines starting with `# t=` are my time stamps, not CLI output.
""")

# ---------------------------------------------------------------- 1
w("## 1. mid-turn-user-message\n")
w("Fixtures: `mid-turn-user-message` (no uuid, no priority), `-uuid`, `-uuid-next`, `-uuid-now`, `-uuid-later`. Flags: `--tools Bash`.\n")
w("Sent (turn start, then the second message 1 s after the first `tool_use` was complete, while `sleep 4` ran):\n")
w(block([j(l["_sent"]) for l in sent("mid-turn-user-message")]))
w("Variants add `\"uuid\":\"<uuid v4>\"` to both messages and `\"priority\":\"next\"|\"now\"|\"later\"` to the second one:\n")
w(block([j(l["_sent"]) for l in sent("mid-turn-user-message-uuid-now")][1:]))
w("### No uuid, no priority (what the adapter sends today)\n")
w("Every non-stream line from the second message to the end (assistant lines without `usage`):\n")
started = {"on": False}


def after_second(l):
    if "_sent" in l and "banana" in json.dumps(l):
        started["on"] = True
    return started["on"] and not_stream(l) and l.get("type") != "rate_limit_event" and not (l.get("subtype") == "status")


w(block(timeline("mid-turn-user-message", after_second)))
w("### With a uuid on each message: `command_lifecycle` lines\n")
w("In the four blocks below the sent lines, the tool results, the assistant lines and the results are abbreviated (`\"…\":\"…\"` stands for the keys left out); the `command_lifecycle` lines are complete.\n")
for v, title in (("uuid", "no priority"), ("uuid-next", "`priority:\"next\"`"), ("uuid-now", "`priority:\"now\"`"), ("uuid-later", "`priority:\"later\"`")):
    w("%s (lifecycle lines, tool results, final text, results):\n" % title)

    def keyline(l):
        if l.get("type") == "command_lifecycle" or l.get("type") == "result":
            return True
        if "_sent" in l:
            return True
        if l.get("type") == "user" and "tool_use_result" in l:
            return True
        if l.get("type") == "assistant" and any(b.get("type") == "text" for b in l["message"]["content"]):
            return True
        return False

    def slim(l):
        if l.get("type") == "user":
            return {"type": "user", "message": l["message"], "…": "…"}
        if l.get("type") == "assistant":
            return {"type": "assistant", "message": {"content": l["message"]["content"], "…": "…"}, "…": "…"}
        if l.get("type") == "result":
            return {k: l[k] for k in ("type", "subtype", "is_error", "num_turns", "result", "stop_reason", "terminal_reason", "queued_turn_count", "result_index", "user_message_uuid", "user_message_uuids") if k in l} | {"…": "…"}
        if "_sent" in l:
            s = l["_sent"]
            return {"_sent": {"type": "user", "message": {"content": s["message"]["content"][:40] + "…"}, **{k: s[k] for k in ("uuid", "priority") if k in s}}}
        return l

    out, t = [], 0.0
    for l in fx("mid-turn-user-message-" + v):
        if set(l) == {"_t"}:
            t = l["_t"]
            continue
        if "_t" in l:
            t = l["_t"]
        if keyline(l):
            out.append("# t=%.2f" % t)
            out.append(json.dumps(slim(l), ensure_ascii=False, separators=(",", ":")))
    w(block(out))
w("""Verified facts:
- A user message written mid-turn is accepted at once and held in a queue. With no `priority` (and the same with
  `"next"`) it is folded into the running turn at the next tool boundary: after tool 1's `tool_result`, before the next
  model request. The turn ends with ONE `result` (`"done banana"`, `num_turns` 4, `queued_turn_count` 0).
- Without a `uuid` on the message, nothing on stdout marks the queued message or the moment it is taken in (no `user`
  line, no system line). With a `uuid`, the CLI writes `command_lifecycle` lines: `queued` when it arrives, `started`
  when it is taken into the turn, `completed` when that turn ends. `command_uuid` is the uuid the host sent. The
  `result` then also has `user_message_uuid` (the message that started the turn) and `user_message_uuids` (every message
  the turn took in: `["user-msg-1","user-msg-2"]` for the folded case).
- `priority:"now"` stops the running turn at the tool boundary (the running `sleep 4` was not killed: its normal
  `tool_result` came first). The stopped turn gets a `result` with `subtype:"success"`, `is_error:false`, `result:""`,
  `stop_reason:"tool_use"`, `terminal_reason:"aborted_tools"`, and its message gets `command_lifecycle` `cancelled`.
  Then the queued message runs as its own turn (new `system/init`, second `result`, `result_index` 1). In that second
  turn the model ran the two remaining sleeps (two more tool results) and answered `done banana`.
- `priority:"later"` waits for the turn to end: first `result` (`"done"`), then a new turn for the queued message
  (second `result`, `"done banana"`).
- `system/init` is written at the start of EVERY turn, not once per process: in every recording the number of
  `system/init` lines equals the number of `result` lines (71 in total), slash-command and self-started turns included.

Not verified:
- Two or more messages queued at once, and how they merge.
- `cancel_async_message` on a really queued message (only the "nothing to cancel" answer was recorded, section 2).
""")

# ---------------------------------------------------------------- 2
w("## 2. control-requests\n")
w("Fixture: `control-requests`. Flags: `--tools Bash,Read`. Request shape: `{\"type\":\"control_request\",\"request_id\":\"host-N\",\"request\":{\"subtype\":…}}`.\n")
w("### Idle session, before the first user message (no `system/init` yet)\n")
ls = fx("control-requests")
resp = {l["response"]["request_id"]: l for l in ls if l.get("type") == "control_response"}
phase_b_start = None
rows_a, rows_b = [], []
seen_turn3 = False
for l in ls:
    if "_sent" in l and l["_sent"].get("type") == "user" and "sleep 20" in json.dumps(l):
        seen_turn3 = True
    if "_sent" in l and l["_sent"].get("type") == "control_request":
        rid = l["_sent"]["request_id"]
        pair = [j(l["_sent"]), j(resp[rid], maxstr=420) if rid in resp else "(no response)"]
        (rows_b if seen_turn3 else rows_a).append(pair)
for a, b in rows_a[:8]:
    w(block([a, b]))
w("`set_permission_mode` also writes a status line (after the response, or just before it):\n")
w(block(pick("control-requests", lambda l: l.get("subtype") == "status" and "permissionMode" in l, 2)))
w("`set_model` on a session that already had a turn also writes a replayed local-command line (the first `set_model`, sent before any turn, wrote none). The next turn uses the new model (`assistant.message.model`, and `system/init.model`):\n")
w(block(pick("control-requests", lambda l: l.get("type") == "user" and l.get("isReplay"), 1) +
        pick("control-requests", lambda l: l.get("type") == "assistant", 2, maxstr=200)))
w("### Other requests on the idle session (after the first turns)\n")
for a, b in rows_a[8:]:
    w(block([a, b]))
w("`rename_session` also writes:\n")
w(block(pick("control-requests", lambda l: l.get("subtype") == "session_title_changed")))
w("### During a turn (`sleep 20` running), then `interrupt`\n")
for a, b in rows_b[:-1]:
    w(block([a, b]))
w("Lines after the `interrupt` (the tool result, the interruption notice, the result):\n")
flag = {"on": False}


def after_interrupt(l):
    if "_sent" in l and l["_sent"].get("type") == "control_request" and l["_sent"]["request"]["subtype"] == "interrupt" and l["_sent"]["request_id"] != "host-2":
        flag["on"] = True
        return False
    if flag["on"] and "_sent" in l:
        flag["on"] = False
    return flag["on"] and not_stream(l) and l.get("type") != "control_response"


w(block(timeline("control-requests", after_interrupt)))
w("`end_session` (sent last, idle):\n")
w(block(rows_b[-1] + pick("control-requests", lambda l: "_meta" in l)))
w("""Verified facts:
- Control requests are answered at once, idle or mid-turn, and before any user message (no `system/init` is needed).
  Success: `{"type":"control_response","response":{"subtype":"success","request_id":…,"response":{…}}}` (some have no
  inner `response`). Error: `{"subtype":"error","request_id":…,"error":"<text>"}`, sometimes with `error_code`.
- Unknown subtype: `"error":"Unsupported control request subtype: <name>"`. `list_directory` exists in the bundle but
  answers the same error in this mode.
- `set_permission_mode`: `plan`, `acceptEdits`, `default` work at any time and answer `{"mode":…}` plus a
  `system/status` line with `permissionMode`. `bypassPermissions` is refused unless the process was started with
  `--dangerously-skip-permissions` (`error_code:"bypass_not_launched"`). A bad mode gives `error_code:"invalid_mode"` and
  the list of valid modes (`acceptEdits, auto, bypassPermissions, default, dontAsk, plan`).
- `set_model`: answers `{"model":"<resolved id>"}`; the next turn runs on that model (`claude-sonnet-5-5`, then back to
  `claude-haiku-5-5`). It works mid-turn too. Unknown model: `error_code:"not_found"` plus the current `model`.
- `interrupt` mid-turn: answer `{"still_queued":[]}`, then a `user` line with the rejected `tool_result`
  (`tool_use_result:"User rejected tool use"`), a `user` text line `[Request interrupted by user for tool use]`, and a
  `result` with `subtype:"error_during_execution"`, `is_error:true`, `terminal_reason:"aborted_tools"`, an `errors`
  array and NO `result` text key. All of it came within 0.01 s of the request (the `sleep 20` had run for 1.5 s). The process
  stays alive (the next turn was answered).
  `interrupt` on an idle session answers the same `{"still_queued":[]}` and writes nothing else.
- `initialize` returns `commands`, `agents`, `models`, `account` (`email`, `organization`, `subscriptionType`,
  `apiProvider`), `pid`, `claude_code_version`, `current_permission_mode`, `session_state`, `capabilities`,
  `output_style`, `available_output_styles` and more, without spending tokens.
- Read-only subtypes that answered: `get_context_usage`, `mcp_status`, `get_session_cost`, `get_usage` (slow: 4.9 s, it
  calls the usage service), `list_models`, `get_status`, `get_settings`, `list_permission_rules`, `get_plan`,
  `get_hooks_listing`, `get_binary_version`, `file_suggestions`, `get_workspace_diff`, `read_file`,
  `get_sandbox_dialog` (section 9). Others that answered: `set_max_thinking_tokens`, `rename_session`, `stop_task`,
  `cancel_async_message`, `background_tasks`, `rewind_files` (dry run: "File rewinding is not enabled."),
  `mcp_reconnect`, `mcp_toggle` (section 8), `end_session`.
- `end_session` answers success and the process exits by itself with code 0 (stdin still open).

Not verified:
- `stop_task` and `get_task_output` on a real task id, `background_tasks` on a running foreground task.
- The other subtypes found in the bundle were not sent (they write state, need the network or an account, or drive a
  UI): `mcp_set_servers`, `mcp_call`, `mcp_authenticate`, `mcp_clear_auth`, `apply_flag_settings`, `update_settings`,
  `add_directory`, `set_cwd`, `reload_plugins`, `reload_skills`, `export_conversation`, `fork_conversation`,
  `rewind_conversation`, `side_question`, `set_proactivity_level`, `seed_read_state`, `stage_file`, `remote_control`,
  `channel_enable`, `claim_session`, the `ui_*` family, the cloud-session and feedback ones.
""")

# ---------------------------------------------------------------- 3
w("## 3. image-input\n")
w("Fixture: `image-input`. Flags: `--tools \"\"`. The PNG is a 16x16 solid red square made with `zlib`/`struct` (second turn: a blue one).\n")
s3 = sent("image-input")
w("Sent (1: red PNG, 2: blue PNG, 3: base64 of the text `hello world` labelled `image/png`, 4: plain text):\n")
w(block([j(l["_sent"], maxstr=400) for l in s3]))
w("The `result` line of each turn:\n")
w(block(pick("image-input", lambda l: l.get("type") == "result", maxstr=200, keep=())))
w("""Verified facts:
- A user message whose `content` is an array with an `image` block (`source.type:"base64"`, `media_type:"image/png"`)
  and a `text` block is accepted. The answers were `Red` for the red square and `Blue` for the blue one, so the model
  saw the pixels.
- An image block whose data is not an image (base64 of plain text) does not fail the turn: `result` is `success` and
  the model says it could not see the image. No error line is written.
- The image turn has the same line sequence as a text turn (`system/init`, `status`, stream events, `assistant`, `result`).

Not verified:
- Other media types, large images, `source.type:"url"`, and images combined with tool use.
""")

# ---------------------------------------------------------------- 4
w("## 4. subagent\n")
w("Fixtures: `subagent` (default), `subagent-forward` (`--forward-subagent-text`), `subagent-foreground` (prompt asks `run_in_background:false`), `subagent-foreground-forward`, `subagent-stdin-closed`. Flags: `--tools Bash,Read,Agent`.\n")
w("Sent:\n")
w(block([j(l["_sent"]) for l in sent("subagent")]))
w("### Default: the Agent tool starts the subagent in the background\n")
w("All non-stream lines (assistant lines without `usage`; the two `result` lines are shown in full further down):\n")


def sub_pred(l):
    return not_stream(l) and l.get("type") not in ("rate_limit_event", "result") and l.get("subtype") not in ("status", "init") and "_meta" not in l and "_sent" not in l


w(block(timeline("subagent", sub_pred, maxstr=260)))
w("The two `result` lines (first: the turn that started the subagent; second: the turn the CLI started by itself when the subagent finished). `subagent_stats` and `origin` in full:\n")
w(block(results("subagent", keep=("subagent_stats",), maxstr=200)))
w("### Foreground (`run_in_background:false` in the Agent input)\n")
w(block(timeline("subagent-foreground", sub_pred, maxstr=260)))
w(block(results("subagent-foreground", keep=("subagent_stats", "modelUsage"), maxstr=200)))
w("With `--forward-subagent-text` the foreground run also forwards the subagent's final text (absent above):\n")
w(block(pick("subagent-foreground-forward", lambda l: l.get("type") == "assistant" and l.get("parent_tool_use_id") and l["message"]["content"][0]["type"] == "text", maxstr=200)))
bg = [l for l in fx("subagent") if l.get("type") == "result"]
fg = [l for l in fx("subagent-foreground") if l.get("type") == "result"]
fgmu = list(fg[0]["modelUsage"].values())[0]
bgmu = list(bg[1]["modelUsage"].values())[0]
bgd = list(bg[1]["model_usage_since_last_result"].values())[0]
USAGE4 = ("- Usage (numbers from these fixtures). Foreground run, one result: `usage.output_tokens` %d but "
          "`modelUsage.<model>.outputTokens` %d: `modelUsage` (and `total_cost_usd`, %s) counts output tokens that `usage` "
          "does not, and the only other model calls in that process were the subagent's. Background run: first result `total_cost_usd` %s (subagent still running), second result %s; the second "
          "result's `usage.output_tokens` is %d (that turn only), `modelUsage` is the running total for the process (%d output "
          "tokens) and `model_usage_since_last_result` is the part since the first result (%d output tokens, the subagent "
          "plus the self-started turn). `subagent_stats` gives `spawned`, `completed`, `by_type`.\n"
          % (fg[0]["usage"]["output_tokens"], fgmu["outputTokens"], fg[0]["total_cost_usd"], bg[0]["total_cost_usd"], bg[1]["total_cost_usd"],
             bg[1]["usage"]["output_tokens"], bgmu["outputTokens"], bgd["outputTokens"]))
w("""Verified facts:
- By default (the model did not set `run_in_background`) the Agent call returns at once: its `tool_result` says
  "Async agent launched successfully", `tool_use_result` is `{"isAsync":true,"status":"async_launched","agentId":…}`.
  The turn ends with a first `result` while the subagent still runs. When it finishes, the CLI starts a turn BY ITSELF
  (`task_notification`, new `system/init`, assistant text, second `result`). That second `result` has
  `"origin":{"kind":"task-notification","producer":"session-task","runId":…}`; a user-started turn has no `origin` key.
- Subagent messages are `assistant` and `user` lines with `parent_tool_use_id` = the id of the Agent `tool_use`, plus
  `agent_id`, `subagent_type`, `task_description`. No `stream_event` line carries a `parent_tool_use_id` (subagent
  output is not streamed as partial messages).
- Task lines: `system/task_started` (`task_type:"local_agent"`, `subagent_type`, `prompt`, `is_backgrounded`,
  `spawn_depth`), `system/task_progress` (`usage.total_tokens`, `tool_uses`, `duration_ms`, `last_tool_name`),
  `system/task_updated` (`patch.status`), `system/task_notification` (`status`, `summary` = the subagent's final text,
  `usage`, `output_file`), and `system/background_tasks_changed` (the list of running background tasks) for
  background runs only.
- Naming: `--tools` takes `Agent`, the `tool_use` block's `name` is `Agent`, but `system/init.tools` lists it as `Task`.
- Foreground run: one `result`. The subagent's prompt arrives as a `user` line with `parent_tool_use_id`. The subagent's
  final text is NOT forwarded as an `assistant` line unless `--forward-subagent-text` is passed; in the background run
  it is forwarded without that flag.
USAGE4_PLACEHOLDER
- Closing stdin right after the first `result` does not stop the work: the subagent finished, the self-started turn
  ran, the second `result` was written, then the process exited with code 0 (fixture `subagent-stdin-closed`).

Not verified:
- Nested subagents, a failing subagent, `stop_task` on a running subagent.
""".replace("USAGE4_PLACEHOLDER\n", USAGE4))

# ---------------------------------------------------------------- 5
w("## 5. background-wakeup\n")
w("Fixture: `background-wakeup`. Flags: `--tools Bash`.\n")
w("Sent:\n")
w(block([j(l["_sent"]) for l in sent("background-wakeup")]))
w("The background start in the first turn:\n")
w(block(pick("background-wakeup", lambda l: (l.get("type") == "assistant" and l["message"]["content"][0]["type"] == "tool_use") or l.get("subtype") in ("task_started",) or (l.get("type") == "user" and "tool_use_result" in l), 3, maxstr=260) +
        pick("background-wakeup", lambda l: l.get("subtype") == "background_tasks_changed", 1)))
w("Every line after the first `result` (stream events left out; the fixture has them), with times:\n")
st = {"n": 0}


def after_first_result(l):
    if l.get("type") == "result":
        st["n"] += 1
        return True
    return st["n"] >= 1 and not_stream(l)


w(block(timeline("background-wakeup", after_first_result, maxstr=260)))
w("""Verified facts:
- Yes: the CLI starts a turn by itself. 5.5 s after the first `result` (when `sleep 6; echo woke` ended) it wrote
  `task_updated`, `task_notification` (`status:"completed"`, `summary`, `output_file`), `background_tasks_changed` with
  an empty list, a new `system/init`, then a normal model turn and a second `result`.
- The self-started turn's `result` carries `"origin":{"kind":"task-notification","producer":"session-task","runId":…}`
  and `result_index` 1. No `user` line announces it; the only markers are the task lines and `origin`.
- The process stayed alive and silent for the 23 s after that until stdin was closed; exit code 0.
- A background Bash start is marked by `task_started` with `is_backgrounded:true` and by `background_tasks_changed`.
  The tool result text is "Command running in background with ID: <task id>. Output is being written to: <file>".

Not verified:
- A background task that fails, several tasks ending together, and a task that ends while a user turn runs.
""")

# ---------------------------------------------------------------- 6
w("## 6. limits\n")
w("Fixtures: `limits-max-turns` (`--tools Bash --max-turns 1`, then a second message), `limits-max-turns-then-close` (no second message), `limits-max-budget` (`--tools Bash --max-budget-usd 0.0001`).\n")
w("Sent:\n")
w(block([j(l["_sent"]) for l in sent("limits-max-turns")]))
w("(a) `--max-turns 1`: the `result` lines (limit hit, then the second message answered) and the end of the process:\n")
w(block(results("limits-max-turns", maxstr=200) + pick("limits-max-turns", lambda l: "_meta" in l)))
w("Same run without a second message (`limits-max-turns-then-close`):\n")
w(block(pick("limits-max-turns-then-close", lambda l: "_meta" in l)))
w("(b) `--max-budget-usd 0.0001`: the `result` lines (limit hit after the first model reply; the second message is refused at once) and the end of the process:\n")
w(block(results("limits-max-budget", maxstr=200) + pick("limits-max-budget", lambda l: "_meta" in l)))
w("""Verified facts:
- `--max-turns 1` (the flag is not in `--help` but is accepted): `result` with `subtype:"error_max_turns"`,
  `is_error:true`, `terminal_reason:"max_turns"`, `errors:["Reached maximum number of turns (1)"]`, `num_turns` 2, no
  `result` text key. The first tool ran (its `tool_result` came before the result). The limit is per turn: the process
  stays alive and the next user message is answered normally.
- `--max-budget-usd 0.0001`: `result` with `subtype:"error_max_budget_usd"`, `is_error:true`,
  `terminal_reason:"budget_exhausted"`, `errors:["Reached maximum budget ($0.0001)"]`. It came right after the first
  model reply (a `tool_use` that was not run). The budget is per process: the process stays alive, but the next user
  message gets the same error `result` at once, with no model call (`stop_reason:null`, same `total_cost_usd`).
- Exit code after stdin is closed: 1 when the last `result` was an error (both limits, also an interrupted turn in
  `misc-events`), 0 when the last `result` was a success (max-turns followed by an answered message).

Not verified:
- `--max-turns` with a value above 1, and whether `--max-budget-usd` counts cost from before a `--resume`.
""")

# ---------------------------------------------------------------- 7
w("## 7. resume-missing\n")
w("Fixtures: `resume-missing` (`--resume <random uuid>`, no `--session-id`), `session-id-in-use` (extra: `--session-id` of a session that already has a transcript).\n")
w("Nothing was sent. Everything the process wrote, `resume-missing`:\n")
w(block(pick("resume-missing", lambda l: "_note" not in l, keep=HEAVY)))
w("`session-id-in-use`:\n")
w(block(pick("session-id-in-use", lambda l: "_note" not in l)))
w("""Verified facts:
- `--resume` with an unknown id: the process exits by itself 0.25 s after start, with no input sent, with code 1.
  stdout has ONE line: a `result` with `subtype:"error_during_execution"`, `is_error:true`, `num_turns` 0,
  `errors:["No conversation found with session ID: <id>"]`, `session_id` = the id that was asked for. stderr has the
  same sentence. No `system/init` line.
- `--session-id` with an id that already has a transcript: exit code 1, stdout EMPTY, stderr
  `Error: Session ID <id> is already in use.` So this failure has no `result` line.

Not verified:
- `--resume` of a session whose transcript file is damaged, and `--resume` from another cwd.
""")

# ---------------------------------------------------------------- 8
w("## 8. broken-mcp\n")
w("Fixture: `broken-mcp`. Flags: `--tools Bash --mcp-config <file>` with this file:\n")
w(block(['{"mcpServers":{"exits-at-once":{"type":"stdio","command":"/usr/bin/false","args":[]},"no-such-command":{"type":"stdio","command":"/nonexistent/bin/mcp-server-xyz","args":[]}}}']))
w("`mcp_status` before the first user message (0.2 s after start), then the `system/init` line's `mcp_servers`, then `mcp_status` after the turn:\n")
init8 = [l for l in fx("broken-mcp") if l.get("subtype") == "init"][0]
mcp8 = pick("broken-mcp", lambda l: l.get("type") == "control_response" and "mcpServers" in json.dumps(l), maxstr=400)
w(block([mcp8[0], jd({"type": "system", "subtype": "init", "mcp_servers": init8["mcp_servers"], "…": "…"}), mcp8[1]]))
w("`mcp_reconnect` and `mcp_toggle`:\n")
ls8 = fx("broken-mcp")
resp8 = {l["response"]["request_id"]: l for l in ls8 if l.get("type") == "control_response"}
for l in ls8:
    if "_sent" in l and l["_sent"].get("type") == "control_request" and l["_sent"]["request"]["subtype"] in ("mcp_reconnect", "mcp_toggle"):
        w(block([j(l["_sent"]), j(resp8[l["_sent"]["request_id"]])]))
w(block([mcp8[-1]]))
w("""Verified facts:
- Broken MCP servers do not stop the session: the turn ran, `result` was `success`, exit code 0, and nothing about the
  failure was written to stdout besides the init line (stderr was empty).
- `system/init.mcp_servers` entries are `{"name","status","source"}`; both broken servers show `"status":"failed"`
  with no error text.
- `mcp_status` gives more: `status` (`pending` while connecting, `failed`, `disabled` after `mcp_toggle` off), `error`
  ("Connection closed" for the server that exits at once, "ENOENT: no such file or directory, posix_spawn 'stdio'" for
  the missing command), `config`, `transport`, `scope`, `source`.
- `mcp_reconnect` on a failed server answers a control error with the connection error text. `mcp_toggle` answers
  success with no inner response.
- With no MCP config, `mcp_status` answers `{"mcpServers":[]}` (section 2).

Not verified:
- A working server (its status value), a server that dies mid-session, and a slow server at the first turn (`pending`
  in the init line).
""")

# ---------------------------------------------------------------- 9
w("## 9. sandbox\n")
w("Fixtures: `sandbox-allow`, `sandbox-deny-unsandboxed`, `sandbox-strict`, `sandbox-off`. Flags: `--tools Bash --settings '{\"sandbox\":{\"enabled\":true}}'` (strict: `'{\"sandbox\":{\"enabled\":true,\"allowUnsandboxedCommands\":false}}'`; off: no `--settings`). Mode `acceptEdits`.\n")
w("Settings key names come from the CLI bundle's settings schema (`sandbox.enabled`, `sandbox.allowUnsandboxedCommands`, `sandbox.autoAllowBashIfSandboxed`, `sandbox.failIfUnavailable`, `sandbox.network`, `sandbox.filesystem`); only the first two were used.\n")
w("The outside probe was `/tmp/sandbox-probe-outside-<variant>.txt` (another name than the one in the task, so that the fixtures carry no project name). It was removed after each run.\n")
w("Sent (`sandbox-allow`):\n")
w(block([j(l["_sent"]) for l in sent("sandbox-allow") if l["_sent"].get("type") != "control_response"]))
w("`get_sandbox_dialog` with the sandbox on, and with it off:\n")
w(block(pick("sandbox-allow", lambda l: l.get("type") == "control_response" and "supported" in json.dumps(l), 1, maxstr=600) +
        pick("sandbox-off", lambda l: l.get("type") == "control_response" and "supported" in json.dumps(l), 1, maxstr=600)))
w("(a) `echo hi > inside.txt` and (b) `touch /tmp/…` with the sandbox on: the tool calls and results (no `control_request` was written for either):\n")
w(block(pick("sandbox-allow", lambda l: (l.get("type") == "user" and "tool_use_result" in l) or (l.get("type") == "assistant" and l["message"]["content"][0]["type"] == "tool_use"), 4, maxstr=260)))
w("Driver notes (files on disk) of `sandbox-allow`:\n")
w(block(pick("sandbox-allow", lambda l: "_meta" in l)))
w("Third message: the same `touch` with `dangerouslyDisableSandbox:true`. The request, my allow, and the result (`sandbox-allow`):\n")
w(block(pick("sandbox-allow", lambda l: l.get("type") == "control_request", maxstr=400) +
        [j(l["_sent"]) for l in sent("sandbox-allow") if l["_sent"].get("type") == "control_response"] +
        pick("sandbox-allow", lambda l: l.get("type") == "user" and "tool_use_result" in l, maxstr=260)[-1:]))
w("The same request denied (`sandbox-deny-unsandboxed`):\n")
w(block([j(l["_sent"]) for l in sent("sandbox-deny-unsandboxed") if l["_sent"].get("type") == "control_response"] +
        pick("sandbox-deny-unsandboxed", lambda l: l.get("type") == "user" and "tool_use_result" in l, maxstr=260)[-1:] +
        pick("sandbox-deny-unsandboxed", lambda l: "_meta" in l and "inside_exists" in l["_meta"])))
w("Control run with no sandbox (`sandbox-off`): the outside `touch` asks for permission:\n")
w(block(pick("sandbox-off", lambda l: l.get("type") == "control_request", maxstr=400) + pick("sandbox-off", lambda l: "_meta" in l and "inside_exists" in l["_meta"])))
w("""Verified facts:
- The `sandbox.enabled` setting passed with `--settings` works in `-p` stream-json mode on macOS:
  `get_sandbox_dialog` reports `"enabled":true,"mode":"auto-allow"`.
- With the sandbox on, NO `can_use_tool` request is sent for sandboxed Bash commands, inside or outside the cwd
  (`tool_result_meta.permission_decision` is `{"decision":"accept","source":"config","reason_type":"other"}`).
- (a) The write inside the cwd ran (`inside.txt` exists). (b) The write outside the cwd failed (the same command worked in
  the run without the sandbox): `tool_result` with `is_error:true` and content
  `Exit code 1\\ntouch: <path>: Operation not permitted`;
  `tool_use_result` is the string `Error: Exit code 1\\n…`. The file was not made. There is no sandbox-specific field: a
  blocked write looks like any failed command.
- A command with `dangerouslyDisableSandbox:true` in its input DOES send `can_use_tool`, with
  `"decision_reason_type":"sandboxOverride"` and no `permission_suggestions`. Allow: the command runs unsandboxed and
  the file is made. Deny: `tool_result` `is_error:true` with the deny message, file not made.
- With `allowUnsandboxedCommands:false` the model refused to retry unsandboxed (no tool call, no request);
  `get_sandbox_dialog` reports `"unsandboxed_fallback":false`.
- Without the sandbox, mode `acceptEdits`: the inside write runs with no request, the outside `touch` sends
  `can_use_tool` with `blocked_path` and `permission_suggestions`.

Not verified:
- Network rules, `sandbox.filesystem` allow lists, and whether a command the sandbox cannot run falls back by itself.
- What the CLI does with a `dangerouslyDisableSandbox:true` call when `allowUnsandboxedCommands` is false (the model
  made no such call).
- The setting given in a settings file (only the `--settings` JSON string was used), and Linux.
""")

# ---------------------------------------------------------------- 10
w("## 10. permission-request-shapes\n")
w("Fixtures: `permission-request-shapes` and `permission-updated-permissions` (`--permission-mode default --tools Bash,Read,Edit`), `permission-plan-mode` (`--permission-mode plan --tools Bash,Read,Edit,Write,ExitPlanMode`), `permission-askuserquestion` (`--permission-mode default --tools Read,AskUserQuestion`).\n")
w("`--permission-mode default` is accepted on the command line (`--help` lists `manual`; the init line says `\"permissionMode\":\"default\"`).\n")
w("### Requests in mode `default`\n")
w("User messages sent (`permission-request-shapes`):\n")
w(block([j(l["_sent"]) for l in sent("permission-request-shapes") if l["_sent"].get("type") == "user"]))
w("`git status` sent no request (read-only command). Every `can_use_tool` request of that session, in order (Bash write in cwd, the same again, Edit, Bash outside cwd, Read outside cwd, Bash `rm`):\n")
w(block(pick("permission-request-shapes", lambda l: l.get("type") == "control_request", maxstr=500, keep=("permission_suggestions",))))
w("A Bash command that writes no file (`permission-updated-permissions`, `git add -A`) carries `decision_reason`:\n")
w(block(pick("permission-updated-permissions", lambda l: l.get("type") == "control_request", 1, maxstr=500)))
w("Deny (`rm note.txt`): my response, the tool result, and the result's `permission_denials`:\n")
deny_sent = [l for l in sent("permission-request-shapes") if l["_sent"].get("type") == "control_response" and l["_sent"]["response"]["response"]["behavior"] == "deny"]
w(block([j(l["_sent"]) for l in deny_sent] +
        pick("permission-request-shapes", lambda l: l.get("type") == "user" and "The user denied this action." in json.dumps(l)) +
        [json.dumps({"type": "result", "subtype": r["subtype"], "permission_denials": r["permission_denials"], "…": "…"}, ensure_ascii=False, separators=(",", ":")) for r in fx("permission-request-shapes") if r.get("type") == "result" and r.get("permission_denials")]))
w("### Allow with `updatedPermissions` (`permission-updated-permissions`)\n")
w("Six user messages: `git add -A` twice, `python3 -c 'print(42)'` twice, `touch newfile.txt`, then an Edit. The three requests that were sent, each followed by my response:\n")
up = fx("permission-updated-permissions")
for l in up:
    if l.get("type") == "control_request":
        w(block([j(l, maxstr=500, keep=("permission_suggestions",))] + [j(s["_sent"], maxstr=500, keep=("updatedPermissions",)) for s in up if "_sent" in s and s["_sent"].get("type") == "control_response" and s["_sent"]["response"]["request_id"] == l["request_id"]]))
w("After the third response (it echoed a `setMode` suggestion) the CLI wrote:\n")
w(block(pick("permission-updated-permissions", lambda l: l.get("subtype") == "status" and "permissionMode" in l)))
w("`list_permission_rules` at the end, and the driver notes (content of `.claude/settings.local.json` in the repo after turn 4 and at the end):\n")
w(block(pick("permission-updated-permissions", lambda l: l.get("type") == "control_response", maxstr=400, keep=("rules",)) +
        pick("permission-updated-permissions", lambda l: "_meta" in l and "settings_local_json" in l["_meta"])))
w("In `permission-request-shapes` the rule from the suggestion was stored for the session, but the same command was asked again:\n")
w(block(pick("permission-request-shapes", lambda l: l.get("type") == "control_response", maxstr=400, keep=("rules",))))
w("### Plan mode: `ExitPlanMode`\n")
w("Sent:\n")
w(block([j(l["_sent"]) for l in sent("permission-plan-mode") if l["_sent"].get("type") == "user"]))
pm = fx("permission-plan-mode")
w("The model wrote the plan file (under `~/.claude/plans/`) with the Write tool; no `can_use_tool` was sent for that write. First `ExitPlanMode` request, my deny, the tool result:\n")
reqs = [l for l in pm if l.get("type") == "control_request"]
resps = [l for l in pm if "_sent" in l and l["_sent"].get("type") == "control_response"]
w(block([j(reqs[0], maxstr=400), j(resps[0]["_sent"]), ] + pick("permission-plan-mode", lambda l: l.get("type") == "user" and "Not yet" in json.dumps(l), 1, maxstr=300)))
w("Second `ExitPlanMode` request, my allow, the status line, the tool result:\n")
w(block([j(reqs[1], maxstr=200), j(resps[1]["_sent"], maxstr=200)] +
        pick("permission-plan-mode", lambda l: l.get("subtype") == "status" and "permissionMode" in l) +
        pick("permission-plan-mode", lambda l: l.get("type") == "user" and "User has approved your plan" in json.dumps(l), 1, maxstr=300)))
w("After the approval the next write asks again (mode is `default` now):\n")
w(block([j(reqs[2], maxstr=300)] + pick("permission-plan-mode", lambda l: l.get("type") == "control_response" and "exists" in json.dumps(l), maxstr=200)))
w("### `AskUserQuestion`\n")
aq = fx("permission-askuserquestion")
w("Sent, the request, my response, the tool result, the final text:\n")
w(block([j(l["_sent"]) for l in aq if "_sent" in l and l["_sent"].get("type") == "user"] +
        [j(l, maxstr=500) for l in aq if l.get("type") == "control_request"] +
        [j(l["_sent"], maxstr=500) for l in aq if "_sent" in l and l["_sent"].get("type") == "control_response"] +
        [j(l, maxstr=500) for l in aq if l.get("type") == "user" and "tool_use_result" in l] +
        [json.dumps({"type": "result", "subtype": l["subtype"], "result": l["result"], "…": "…"}, ensure_ascii=False, separators=(",", ":")) for l in aq if l.get("type") == "result"]))
w("""Verified facts:
- Request keys seen: `subtype`, `tool_name`, `display_name`, `input`, `tool_use_id`, and when they apply `description`,
  `permission_suggestions`, `blocked_path`, `decision_reason`, `decision_reason_type`, `requires_user_interaction`.
  `request_id` is a uuid on the outer object.
  - Bash that writes a file (inside or outside the cwd): `blocked_path` = the absolute path, no `decision_reason`.
  - Bash that writes no file (`git add -A`, `python3 -c …`): `decision_reason:"This command requires approval"`,
    `decision_reason_type:"other"`, no `blocked_path`.
  - Edit: `description` = the file name, one suggestion `{"type":"setMode","mode":"acceptEdits","destination":"session"}`.
  - Read outside the cwd: `decision_reason:"Path is outside allowed working directories"`,
    `decision_reason_type:"workingDir"`, suggestions are `addRules` for `Read` with `//etc/**` style rules.
  - Sandbox override: `decision_reason_type:"sandboxOverride"` (section 9).
- `permission_suggestions` item types seen: `addRules` (`rules:[{toolName,ruleContent}]`, `behavior:"allow"`,
  `destination:"localSettings"` or `"session"`), `addDirectories` (`directories`, `destination:"session"`), `setMode`
  (`mode:"acceptEdits"`, `destination:"session"`).
- Allow with `updatedPermissions` = the suggestions works: the same command was NOT asked again in the session
  (`git add -A` with `destination` changed to `"session"`; `python3 -c 'print(42)'` with the suggestion unchanged).
  `destination:"localSettings"` writes `<cwd>/.claude/settings.local.json` at once (`permissions.allow`).
- Echoing ALL suggestions is not neutral: a `setMode` item switches the whole session to `acceptEdits`
  (`system/status` with `permissionMode:"acceptEdits"`), and the following Edit was not asked.
- A stored Bash rule does not cover a command that writes a file in mode `default`: `echo probe > note.txt` was asked
  again although `Bash(echo probe *)` was in the session rules; the ask is for the written path (`blocked_path`).
- Deny: the `tool_result` is `is_error:true` with the deny `message` as content, `tool_use_result` is
  `"Error: <message>"`, and the turn's `result` lists the call in `permission_denials` (`tool_name`, `tool_use_id`,
  `tool_input`). No `system/permission_denied` line was written.
- Plan mode: `ExitPlanMode` arrives as `can_use_tool` with `input:{"plan":"<markdown>","planFilePath":"<path under
  ~/.claude/plans>"}` and `requires_user_interaction:true`. Deny: the message goes back to the model as an error tool
  result, the session stays in plan mode, and the model revised the plan and asked again. Allow: the CLI writes
  `system/status` with `permissionMode:"default"` (not `acceptEdits`), the tool result starts with "User has approved
  your plan", and later writes send `can_use_tool` again. `get_plan` returns `{"exists":true,"content":…,"path":…}`.
- `AskUserQuestion` arrives as `can_use_tool` with `input.questions[]` (`question`, `header`, `options[{label,
  description}]`, `multiSelect`) and `requires_user_interaction:true`. Allow with
  `updatedInput:{questions, answers:{"<question text>":"<label>"}}`; the tool result is "Your questions have been
  answered: …" and the model used the answer.
- `tool_result_meta[].permission_decision` on the tool result line tells who decided: `source:"config"` (no request),
  `"user_temporary"` (allowed by the host), `"user_reject"` (denied by the host).

Not verified:
- `multiSelect` answers and several questions in one call, allow of `ExitPlanMode` with a changed `updatedInput.plan`,
  `destination:"userSettings"`/`"projectSettings"`, MCP tool requests, and the `title`, `agent_id` keys.
""")

# ---------------------------------------------------------------- 11
w("## 11. compaction-and-slash\n")
w("Fixture: `compaction-and-slash`. Flags: `--tools Bash`. Each slash command is a normal user message whose content is the command text.\n")
w("Sent, in order:\n")
w(block([j(l["_sent"]) for l in sent("compaction-and-slash")]))
cs = fx("compaction-and-slash")
segs, cur = [], None
for l in cs:
    if "_sent" in l:
        cur = [l["_sent"]["message"]["content"], []]
        segs.append(cur)
    elif cur is not None and not_stream(l) and set(l) != {"_t"} and "_meta" not in l:
        cur[1].append(l)
seg = {k: v for k, v in segs}


def seg_lines(key, **kw):
    out = []
    for l in seg[key]:
        if l.get("subtype") == "init":
            out.append(jd({"type": "system", "subtype": "init", "session_id": l["session_id"], "…": "…"}))
        else:
            out.append(j(l, **kw))
    return out


w("`/context` (no model call; `num_turns` 0):\n")
w(block(seg_lines("/context", maxstr=260)))
w("`/cost` (answered after 4.8 s; the text is cut by the sanitizer):\n")
w(block(seg_lines("/cost", maxstr=260)))
w("`/compact` (8 s). After the boundary the CLI replays the summary and the kept messages:\n")
w(block(seg_lines("/compact", maxstr=200)))
w("`/compact keep only the numbers` (arguments are the compaction instructions):\n")
w(block(seg_lines("/compact keep only the numbers", maxstr=200)))
w("An unknown slash command is sent to the model as plain text:\n")
w(block([x for x in seg_lines("/this-command-does-not-exist", maxstr=260) if '"type":"result"' in x]))
w("`/clear`, then the next turn (note the new `session_id`):\n")
w(block(seg_lines("/clear", maxstr=200) + [x for x in seg_lines("Reply with the single word: four", maxstr=120) if '"type":"result"' in x or '"init"' in x]))
w("""Verified facts:
- `/context`, `/cost`, `/compact`, `/clear` work as user messages in stream-json mode. Each ends with a `result`
  (`subtype:"success"`, `num_turns` 0, no `terminal_reason`, and a `local_command` key: `"context"`, `"cost"`,
  `"compact"`, `"clear"`); `/context` and `/cost` put their text in `result.result`
  and in an `assistant` line with `"model":"<synthetic>"` plus `local_command_source`, `local_command_run`, and a
  structured copy (`context_usage` or `usage_report`).
- `/compact` writes `system/status` `{"status":"compacting"}`, then `{"status":null,"compact_result":"success"}`,
  a `system/init`, `system/compact_boundary` with `compact_metadata` (`trigger:"manual"`, `pre_tokens`, `post_tokens`,
  `cumulative_dropped_tokens`, `duration_ms`, `preserved_segment`, `preserved_messages`), a `user` line with the summary
  text, replays of kept messages, a replayed `user` line `<local-command-stdout>Compacted </local-command-stdout>`
  (`isReplay:true`), and a `result` with empty `result` text. The session id does not change.
- `/clear` writes a top-level `conversation_reset` line (`trigger:"clear"`, `user_message_uuid`, and
  `new_conversation_id`, which is also that line's own `uuid`). Every later line carries a NEW `session_id` (`sess-2`),
  which is a different id from `new_conversation_id` (`conv-1`). `total_cost_usd` starts again from 0. The process
  keeps running.
- An unknown `/name` is not an error: the model receives it as text and a normal turn runs.

Not verified:
- Automatic compaction (`trigger:"auto"`), `/compact` on a session too short to compact, and a failed compaction.
- Whether the session id after `/clear` can be given to `--resume`, and what `new_conversation_id` names.
""")

# ---------------------------------------------------------------- 12
w("## 12. misc-events\n")
w("Fixture: `misc-events`. Flags: `--permission-mode default --tools Bash --include-hook-events --prompt-suggestions true --replay-user-messages`, with a `PreToolUse` (matcher `Bash`) and a `Stop` hook in the repo's `.claude/settings.local.json` (`echo pre-tool-hook-ran`, `echo stop-hook-ran`).\n")
w("Hook lines (`--include-hook-events`):\n")
w(block(pick("misc-events", lambda l: l.get("subtype") in ("hook_started", "hook_response"), 4)))
w("`--replay-user-messages`: the host's user message comes back on stdout with `isReplay:true`:\n")
w(block(pick("misc-events", lambda l: l.get("type") == "user" and l.get("isReplay"), 1)))
w("`interrupt` while a `can_use_tool` request waits for an answer (request left unanswered for 1 s):\n")
fl = {"on": False}


def cancel_window(l):
    if l.get("type") == "control_request" and not fl.get("done"):
        fl["on"] = True
    if fl["on"] and l.get("type") == "result":
        fl["on"] = False
        fl["done"] = True
        return True
    return fl["on"] and not_stream(l) and set(l) != {"_t"}


w(block(timeline("misc-events", cancel_window, maxstr=260)))
w("Deny with `interrupt:true` (third turn): my response, and what followed. With `--replay-user-messages` the CLI also echoes the host's `control_response` on stdout:\n")
ms = fx("misc-events")
idx = [i for i, l in enumerate(ms) if "_sent" in l and l["_sent"].get("type") == "control_response"][-1]
tail = [l for l in ms[idx:] if not_stream(l) and set(l) != {"_t"}]
w(block([j(l["_sent"]) if "_sent" in l else j(l, maxstr=260) for l in tail]))
w("Other lines seen in every recording:\n")
w(block(pick("mid-turn-user-message", lambda l: l.get("subtype") == "status", 1) +
        pick("mid-turn-user-message", lambda l: l.get("subtype") == "thinking_tokens", 1) +
        pick("mid-turn-user-message", lambda l: l.get("type") == "rate_limit_event", 1) +
        pick("mid-turn-user-message", lambda l: l.get("subtype") == "task_started", 1) +
        pick("mid-turn-user-message", lambda l: l.get("subtype") == "task_notification", 1) +
        pick("smoke", lambda l: l.get("subtype") == "commands_changed", 1, maxstr=120) +
        pick("mid-turn-user-message", lambda l: l.get("type") == "stream_event" and l["event"]["type"] == "message_start", 1) +
        pick("mid-turn-user-message", lambda l: l.get("type") == "stream_event" and l["event"]["type"] == "content_block_delta", 1)))
w("A `system/init` line (lists shortened) and a full `result` line with `usage` (from `smoke`):\n")
w(block(pick("smoke", lambda l: l.get("subtype") == "init", 1, keep=("tools", "capabilities")) + pick("smoke", lambda l: l.get("type") == "result", 1, keep=HEAVY)))
w("""Verified facts:
- With `--include-hook-events` and hooks configured, each hook run writes `system/hook_started` and
  `system/hook_response` (`hook_id`, `hook_name`, `hook_event`, `output`, `stdout`, `stderr`, `exit_code`, `outcome`).
  The other recordings had no hook configured, so they do not show what is written without the flag.
- An `interrupt` while a permission request waits: the CLI writes `control_cancel_request` with the request's
  `request_id`, then the interrupt's `control_response`, the rejected tool result, the interruption notice and an
  `error_during_execution` result that also lists the call in `permission_denials`.
- Deny with `interrupt:true` ends the turn the same way as an interrupt: rejected tool result
  (`non_execution_kind:"user-rejected"`), `[Request interrupted by user for tool use]`, `result`
  `error_during_execution` / `terminal_reason:"aborted_tools"`.
- A foreground Bash call that runs for some seconds gets `system/task_started` (about 4 s after the `tool_use`, with
  `is_backgrounded:false`, `task_type:"local_bash"`) and `system/task_notification` when it ends; a short command gets
  neither.
- `system/status` `{"status":"requesting"}` is written before each model request; `system/thinking_tokens` gives a
  running estimate while the model thinks; `rate_limit_event` came once per process, after the first model reply
  (27 recordings; none in `limits-max-budget`).
- `stream_event` lines carry `event` (the API streaming event), `parent_tool_use_id`, `api_message_id`, and on
  `message_start` also `ttft_ms` and `thinking_display`. Thinking text was always empty (`"thinking":""`, only a
  signature), with the default thinking display.

Not verified (not seen in any recording):
- `tool_progress` (a 12 s `sleep` did not produce one), `system/api_retry`, `prompt_suggestion` (not written even with
  `--prompt-suggestions true`), `tool_use_summary`, `auth_status`, `keep_alive`, `system/hook_progress`,
  `system/permission_denied`, `system/model_fallback`, `system/notification`. Their names are in the CLI bundle only.
""")

# ---------------------------------------------------------------- table
w("## Table: every (type, subtype) pair seen\n")
pairs = collections.OrderedDict()
for p in sorted(glob.glob(os.path.join(OUT, "fixtures", "*.ndjson"))):
    name = os.path.basename(p)[:-7]
    for l in fx(name):
        if not is_cli(l):
            continue
        t, st = l.get("type"), l.get("subtype")
        if t == "control_request":
            st = l["request"]["subtype"]
        if t == "control_response":
            st = l["response"]["subtype"]
        e = pairs.setdefault((t, st), [0, []])
        e[0] += 1
        if name not in e[1]:
            e[1].append(name)
where = {
    ("assistant", None): "all", ("user", None): "all (tool results, replays, subagent prompts)", ("stream_event", None): "all",
    ("system", "init"): "all; once per turn", ("system", "status"): "all", ("system", "thinking_tokens"): "all with thinking",
    ("rate_limit_event", None): "all", ("result", "success"): "all",
}
w("| type | subtype | lines | first fixtures | section |\n|---|---|---|---|---|")
sec = {
    "command_lifecycle": "1", "control_cancel_request": "12", "can_use_tool": "9, 10", "conversation_reset": "11",
    "error_during_execution": "2, 7, 12", "error_max_budget_usd": "6", "error_max_turns": "6", "background_tasks_changed": "4, 5",
    "commands_changed": "12", "compact_boundary": "11", "hook_response": "12", "hook_started": "12", "init": "1, 12",
    "session_title_changed": "2", "status": "2, 10, 11, 12", "task_notification": "4, 5, 12", "task_progress": "4",
    "task_started": "4, 5, 12", "task_updated": "4, 5", "thinking_tokens": "12", "rate_limit_event": "12", "stream_event": "12",
    "assistant": "4, 11", "user": "2, 4, 10, 11", "error": "2, 8", "success": "2",
}
for (t, st), (n, names) in sorted(pairs.items(), key=lambda x: (str(x[0][0]), str(x[0][1]))):
    k = st if st in sec and t in ("system", "result", "control_request") else (st if t == "control_response" else t)
    if t == "result" and st == "success":
        s = "all"
    else:
        s = sec.get(k, "")
    w("| `%s` | %s | %d | %s | %s |" % (t, ("`%s`" % st) if st else "(none)", n, ", ".join("`%s`" % x for x in names[:2]) + (" …" if len(names) > 2 else ""), s))
w("")
w("For `control_request` and `control_response` the subtype column is `request.subtype` / `response.subtype`.\n")
inner = collections.Counter()
for p in sorted(glob.glob(os.path.join(OUT, "fixtures", "*.ndjson"))):
    for l in fx(os.path.basename(p)[:-7]):
        if l.get("type") == "stream_event":
            ev = l["event"]
            inner[(ev["type"], (ev.get("delta") or ev.get("content_block") or {}).get("type"))] += 1
w("`stream_event.event.type` values seen (with the block or delta type): " + ", ".join("`%s`%s (%d)" % (a, ("/`%s`" % b) if b else "", n) for (a, b), n in sorted(inner.items(), key=lambda x: str(x[0]))) + ".\n")
w("Top-level keys of `result` (success): " + ", ".join("`%s`" % k for k in [l for l in fx("smoke") if l.get("type") == "result"][0].keys()) + ". Error results have `errors` and no `result`, `api_error_status` or `ttft_*` keys; a self-started turn adds `origin`; a slash command adds `local_command` and has no `terminal_reason`; a turn started by a user message with a `uuid` adds `user_message_uuid`, `user_message_uuids`, `first_request_input_tokens`, `request_sent_wall_ms`.\n")

import sys
DEST = sys.argv[1] if len(sys.argv) > 1 else os.path.join(OUT, "NOTES.md")
open(DEST, "w").write("\n".join(P))
print(os.path.basename(DEST), "written:", sum(len(x) for x in P), "chars")
