# Claude Code CLI stream-json: recorded protocol shapes, part 7

**QUESTION 1 FIRST. A PLUGIN THE USER NEVER INSTALLED, NAMED BY A FILE OF THE FOLDER (`extraKnownMarketplaces` WITH A
FOLDER SOURCE PLUS `enabledPlugins`, IN `.claude/settings.json` AND `.claude/settings.local.json`, OR IN
`.claude/settings.local.json` ALONE), WAS NOT LOADED: NO SERVER, NO HOOK, NOTHING OF IT IN THE HOME, UNDER `--setting-sources
user` AND UNDER `user,local`, AT A FIRST AND AT A SECOND START (8 RUNS, 2 STARTS EACH). THE SAME TWO KEYS IN THE USER'S
OWN SETTINGS FILE DO LOAD IT (CONTROL, 4 RUNS). BUT A PLUGIN THE USER INSTALLED AND TURNED OFF, TURNED ON BY THE
FOLDER'S `.claude/settings.local.json`, RUNS ITS HOOKS (`SessionStart`, `UserPromptSubmit`) AND STARTS ITS SERVER
UNDER `user,local` (2 of 2 runs, every start); UNDER `user` IT DOES NOT (0 of 2 runs). UNDER `--setting-sources user`
ALONE NO FILE OF THE FOLDER STARTED OR LOADED ANYTHING IN ANY OF THE 29 RECORDINGS OF PARTS 4 TO 7 THAT TRIED
(part 4: 2, part 5: 5, part 6: 10, part 7: 12).** So for a folder you have not trusted: `user` alone keeps every
file of the folder out in what was recorded; `user,local` lets the folder's local settings file run a turned-off
plugin's hooks (and part 4 recorded that file's own hooks running too).

- CLI: `claude` 2.1.311 (the same version as parts 1 to 6; checked with `claude --version` before the runs).
- Recorded: 2026-11-23, macOS, `--model haiku` (`claude-haiku-5-5`), subscription login (`"apiKeySource":"none"`).
- Method, base command, fixture line format and sanitizing: as in part 1 ("Base command", "Files", "Sanitizing") with
  the additions of parts 2 to 6. Fixtures of this part are `fixtures/claude-2.1.311/r9-*.ndjson`; the scripts are
  `drivers/claude-stream-json/r9_*.py` (README there says how to record again).
- Cost of the 99 recordings at list price: 0.0299 USD, the sum of each turn's own model requests
  (`model_usage_since_last_result`). `cost.py` prints 0.0655: it sums each session's last `total_cost_usd` per
  recording, and a resumed or forked session carries its earlier cost. The runs in a throw-away HOME have no login
  and cost nothing. With the runs that were recorded again (sections 2, 3, 4 and 7) about 0.12 USD by `cost.py`'s
  count: an estimate, as their raw files were replaced. The cap of this part was 2 USD, with a stop at 1.50 USD.
- Everything below was observed in these recordings. Each answer says how many runs it rests on (n) and says "not
  recorded" for what no recording shows. Statements marked "bundle" were read in the CLI's program file
  (`bgrep.py`), not recorded.
- The sections are numbered as the 8 questions this part was recorded for; the last two tables have them in the same
  order.

Differences from the part 6 method:
- Section 1 ran ONLY with a throw-away HOME (no login): nothing of the owner's own Claude Code setup was read or
  written. The scenario's plugin now brings two hooks (each appends its event's name to a file of the work folder),
  and every run starts the CLI twice in the same HOME and folder (`-a`, `-b`), in case a declared marketplace is
  fetched in the background and loaded only at a later start.
- The sanitizer keeps only the scenario's plugin in a driver note too (a list under a key `plugins` or `*_plugins`):
  part 6's notes copied `init.plugins` past it (`test_sanitize.py` has the test). It keeps the scenario's own slash
  commands and skills of section 5 (`codly-*`) in `init`, and removes every other one, as before.
- Sections 2, 3 and 4 read the session's own transcript file under `~/.claude/projects/` for counts and for the
  CLI's own line types (`r9_lib.transcript_types`: `user`, `assistant`, `attachment/<kind>`, …); nothing of the
  conversation is copied.
- Section 7 runs the interactive `claude` in a pseudo-terminal (`r9_lib.Terminal`). Its screen is read only to wait
  for words and is never written to a fixture; those fixtures (`-a`, `-c`) hold the driver's notes only.
- Section 7 ran under the owner's REAL HOME and accepted the CLI's trust question there ("Yes, I trust this
  folder"): the CLI saved a trust entry for each of the two work folders (`projects["<capture folder>/work/r9-terminal-1"]`
  and `-2` of the real `~/.claude.json`). Part 6's rule asks for a throw-away HOME for every run that writes a
  setting; this run did not keep it, and was not approved in advance. A later recording needs the owner's OK first.
- In quoted lines `"…":"…"` stands for keys left out; `# t=` lines are seconds since the process started.

## 1. A plugin named by a file of the folder, and a plugin's hooks, in a folder you have not trusted

Question: with a throw-away HOME, no `--strict-mcp-config`, `--setting-sources user` and `user,local`: is anything of a plugin the user NEVER installed loaded or started when a file of the folder names it (`extraKnownMarketplaces` plus `enabledPlugins`)? Does the HOOK of a plugin the user turned off run when the folder's local settings file turns it on?

Fixtures: `r9-plugin-from-repo-<form>-<sources>-<label>-a` and `-b` (two starts in the same HOME and folder). Flags of `off-repo-local-user-local-1-a`: `--permission-mode manual --setting-sources user,local --tools Bash`. Every run has a THROW-AWAY HOME (no login: each message ends at once with `Not logged in`; the recordings show what was LOADED and STARTED, and what the hooks did). The plugin `ping-plugin` lives in a local marketplace folder `codly-test-market` under the capture folder; its `.mcp.json` brings `ping-plugged` (one line in `started-plugin.txt` per start) and its `hooks/hooks.json` has a `SessionStart` and a `UserPromptSubmit` hook (each appends its event's name to `hook-ran.txt`). The forms: `repo-both` (the user never installed it; both repo files say `extraKnownMarketplaces` with a `directory` source plus `enabledPlugins`), `repo-local` (the same in `.claude/settings.local.json` only), `user-declared` (control: the same two keys in the HOME's user settings file), `on` (control: installed and on for the user, with `claude plugin marketplace add` and `claude plugin install --scope user`), `off` (control: then `claude plugin disable --scope user`), `off-repo-local` (turned off for the user; the repo's `.claude/settings.local.json` turns it on).

The set-up note and the `init` line of `r9-plugin-from-repo-repo-both-user-local-1-a` (the user never installed the plugin):

```json
{"setting_sources":"user,local","repo_files":{"settings.json":{"extraKnownMarketplaces":{"codly-test-market":{"source":{"source":"directory","path":"/tmp/work/r9-plugin-from-repo-repo-both-user-local-1-market"}}},"enabledPlugins":{"ping-plugin@codly-test-market":true}},"settings.local.json":{"extraKnownMarketplaces":{"codly-test-market":{"source":{"source":"directory","path":"/tmp/work/r9-plugin-from-repo-repo-both-user-local-1-market"}}},"enabledPlugins":{"ping-plugin@codly-test-market":true}}},"user_settings_file":null,"plugin_files_in_the_home":[],"by_the_setup":{"server_starts":0,"hook_lines":[]}}
# t=4.01
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"default","tools":["Bash"],"mcp_servers":[],"…":"…"}
```

`r9-plugin-from-repo-off-repo-local-user-local-1-a` (turned off for the user, on in the repo's local file): the hook lines of the stream, the `init` line and the note after the turn:

```json
# t=0.00
{"type":"system","subtype":"hook_started","hook_id":"hook-1","hook_name":"SessionStart:startup","hook_event":"SessionStart"}
# t=0.00
{"type":"system","subtype":"hook_response","hook_id":"hook-1","hook_name":"SessionStart:startup","hook_event":"SessionStart","output":"","stdout":"","stderr":"","exit_code":0,"outcome":"success"}
# t=4.01
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"default","tools":["Bash","mcp__plugin_ping-plugin_ping-plugged__ping"],"mcp_servers":[{"name":"plugin:ping-plugin:ping-plugged","status":"connected","source":"plugin"}],"…":"…"}
{"after_the_turn":true,"server_starts":1,"hook_lines":["SessionStart","UserPromptSubmit"],"init_mcp_servers":[{"name":"plugin:ping-plugin:ping-plugged","status":"connected","source":"plugin"}],"init_mcp_tools":["mcp__plugin_ping-plugin_ping-plugged__ping"],"init_names_the_plugin":true,"plugin_files_in_the_home":[".claude/plugins/cache/codly-test-market/ping-plugin/0.0.1/.claude-plugin/plugin.json",".claude/plugins/cache/codly-test-market/ping-plugin/0.0.1/.mcp.json","…(2 more)"],"can_use_tool_requests":0,"result":"Not logged in · Please run /login"}
```

The same files under `--setting-sources user` (`r9-plugin-from-repo-off-repo-local-user-1-a`): the `init` line and the note after the turn:

```json
# t=4.01
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"default","tools":["Bash"],"mcp_servers":[],"…":"…"}
{"after_the_turn":true,"server_starts":0,"hook_lines":[],"init_mcp_servers":[],"init_mcp_tools":[],"init_names_the_plugin":false,"plugin_files_in_the_home":[".claude/plugins/cache/codly-test-market/ping-plugin/0.0.1/.claude-plugin/plugin.json",".claude/plugins/cache/codly-test-market/ping-plugin/0.0.1/.mcp.json","…(2 more)"],"can_use_tool_requests":0,"result":"Not logged in · Please run /login"}
```

| form | `--setting-sources` | what the HOME and the repo have | runs | after start a: server starts, hook lines | after start b (counted from a on) | `init` names the plugin (a, b) |
|---|---|---|---|---|---|---|
| `repo-both` | `user` | never installed; both repo files declare and enable it | 2 | 0, none / 0, none | 0, none / 0, none | False, False / False, False |
| `repo-both` | `user,local` | never installed; both repo files declare and enable it | 2 | 0, none / 0, none | 0, none / 0, none | False, False / False, False |
| `repo-local` | `user` | never installed; the repo's local file declares and enables it | 2 | 0, none / 0, none | 0, none / 0, none | False, False / False, False |
| `repo-local` | `user,local` | never installed; the repo's local file declares and enables it | 2 | 0, none / 0, none | 0, none / 0, none | False, False / False, False |
| `user-declared` | `user` | never installed; the user settings file declares and enables it (control) | 2 | 1, SessionStart+UserPromptSubmit / 1, SessionStart+UserPromptSubmit | 2, SessionStart+UserPromptSubmit+SessionStart+UserPromptSubmit / 2, SessionStart+UserPromptSubmit+SessionStart+UserPromptSubmit | True, True / True, True |
| `user-declared` | `user,local` | never installed; the user settings file declares and enables it (control) | 2 | 1, SessionStart+UserPromptSubmit / 1, SessionStart+UserPromptSubmit | 2, SessionStart+UserPromptSubmit+SessionStart+UserPromptSubmit / 2, SessionStart+UserPromptSubmit+SessionStart+UserPromptSubmit | True, True / True, True |
| `off-repo-local` | `user` | installed, off for the user; the repo's local file turns it on | 2 | 0, none / 0, none | 0, none / 0, none | False, False / False, False |
| `off-repo-local` | `user,local` | installed, off for the user; the repo's local file turns it on | 2 | 1, SessionStart+UserPromptSubmit / 1, SessionStart+UserPromptSubmit | 2, SessionStart+UserPromptSubmit+SessionStart+UserPromptSubmit / 2, SessionStart+UserPromptSubmit+SessionStart+UserPromptSubmit | True, True / True, True |
| `on` | `user` | installed and on for the user (control) | 2 | 1, SessionStart+UserPromptSubmit / 1, SessionStart+UserPromptSubmit | 2, SessionStart+UserPromptSubmit+SessionStart+UserPromptSubmit / 2, SessionStart+UserPromptSubmit+SessionStart+UserPromptSubmit | True, True / True, True |
| `on` | `user,local` | installed and on for the user (control) | 2 | 1, SessionStart+UserPromptSubmit / 1, SessionStart+UserPromptSubmit | 2, SessionStart+UserPromptSubmit+SessionStart+UserPromptSubmit / 2, SessionStart+UserPromptSubmit+SessionStart+UserPromptSubmit | True, True / True, True |
| `off` | `user` | installed, off for the user (control) | 2 | 0, none / 0, none | 0, none / 0, none | False, False / False, False |
| `off` | `user,local` | installed, off for the user (control) | 2 | 0, none / 0, none | 0, none / 0, none | False, False / False, False |

What the adapter can rely on:
- A plugin the user never installed is NOT loaded when a file of the folder names it, under `user` or `user,local`
  (`repo-both`, `repo-local`; n=2 per form and sources, 8 runs, each with two starts): no server start, no hook line,
  no plugin in `init`, no file of the plugin in the HOME, no stderr line. The same declaration in the user's own
  settings file loads it at once (control `user-declared`, n=2 per sources): the CLI reads such a declaration, but
  only from the user's side. Bundle (not recorded): a hint of the CLI for a marketplace whose folder is on a network
  drive says to declare it "in user or managed settings (a declaration in project or local settings does not
  count)".
- A plugin the user installed and left on runs its hooks and starts its server under both sources (control `on`, n=2
  each): `SessionStart` at the start, `UserPromptSubmit` at the message, `init.mcp_servers` names
  `plugin:ping-plugin:ping-plugged` with `"source":"plugin"`. Turned off, nothing of it runs (control `off`, n=2 each).
- A plugin the user turned OFF is turned ON by the folder's `.claude/settings.local.json` under `user,local`: its
  `SessionStart` and `UserPromptSubmit` hooks RAN (the stream has its `hook_started` / `hook_response` lines, the file
  got both lines), at both starts, and its server started (n=2 runs). Under `user` it stays off (n=2). So part 6's
  finding holds for the plugin's hooks too: under `user,local` the folder's local file can make code the user turned
  off run before the first model request, and without a login.
- Under `user` alone no file of the folder started or loaded anything in any of the 29 recordings of parts 4 to 7 that
  put such a file there (part 4: `r6-project-mcp-user-only`, `r6-project-mcp-user-only-home`; part 5: `r7-untrusted-mcp-approval-user-1`, `r7-untrusted-mcp-approval-user-2`, `r7-untrusted-mcp-local-scope-user-1`, `r7-untrusted-mcp-local-scope-user-2`, `r7-untrusted-mcp-user-login-1`; part 6: the 10 `r8-untrusted-mcp-*-user-*` recordings but `plugin-user` (the user's own plugin); part 7: the 12 recordings of the forms `repo-both`, `repo-local`, `off-repo-local` under `user`).
- Not recorded: a marketplace a repo file declares with a network source (`github`, `git`, `url`: the CLI would fetch it;
  this part has no network form), an inline `settings` source, and any of this with the real login.

## 2. Rewind a conversation that has tool calls, a helper and an interrupted turn

Question: after `--resume <id> --resume-session-at <uuid>` with the uuid of the last assistant line of a turn with tool calls, of a turn with a helper, and of an interrupted turn: does the next turn start, does it know only the kept turns, does any line say that a tool call has no result? Which `uuid` does the host see on the final assistant line of a turn (with `--include-partial-messages`)? The control: the same uuid resumed before a rewound turn was added. Also: a rewind to a user message's uuid; `--resume-drops-turn`.

Fixtures: `r9-rewind-tools-<label>-a` … `-f` and `r9-rewind-drops-<label>-a` … `-c`, labels 1, 2. Flags of `-a`: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash,Read,Edit,Agent --replay-user-messages`. Process a: four turns, each tells one secret word (`lantern`, `pebble`, `violet`, `copper`) and carries a uuid of the host: turn 1 text only; turn 2 a Read, an Edit and a Bash call; turn 3 a helper (`Agent`) that runs a Bash call; turn 4 a Bash call `echo copper-start; sleep 19.5; echo copper-end`, and an `interrupt` once its `sleep` runs. Then one process each, in this order, asks (no tools) for every secret word: b `--resume-session-at <the last assistant line of turn 4>` (the interrupted call's line), c `… <the end of turn 3>`, d `… <the end of turn 2>`, e `… <the uuid of turn 2's USER message>`, f `… <the end of turn 3>` again (c is its control: the same uuid before d and e added turns on an earlier branch).

Process a's note per turn (`r9-rewind-tools-1-a`):

```json
{"turn":1,"assistant_uuids":["uuid-8"],"assistant_uuids_in_the_transcript":[true],"stream_events_with_a_uuid":6,"stream_event_uuids_in_the_transcript":0,"replayed_user_lines":[{"uuid_is_the_hosts":true,"isReplay":true}],"result_subtype":"success","result_user_message_uuid_is_the_hosts":true,"tools_used":[]}
{"turn":2,"assistant_uuids":["uuid-32","uuid-43","uuid-65","uuid-78","uuid-95"],"assistant_uuids_in_the_transcript":[true,true,true,true,true],"stream_events_with_a_uuid":68,"stream_event_uuids_in_the_transcript":0,"replayed_user_lines":[{"uuid_is_the_hosts":true,"isReplay":true}],"result_subtype":"success","result_user_message_uuid_is_the_hosts":true,"tools_used":["Read","Bash","Edit"]}
{"turn":3,"assistant_uuids":["uuid-112","uuid-133","uuid-149"],"assistant_uuids_in_the_transcript":[true,true,true],"stream_events_with_a_uuid":34,"stream_event_uuids_in_the_transcript":0,"replayed_user_lines":[{"uuid_is_the_hosts":true,"isReplay":true}],"result_subtype":"success","result_user_message_uuid_is_the_hosts":true,"tools_used":["Agent","Bash"]}
{"turn":4,"assistant_uuids":["uuid-170"],"assistant_uuids_in_the_transcript":[true],"stream_events_with_a_uuid":14,"stream_event_uuids_in_the_transcript":0,"replayed_user_lines":[{"uuid_is_the_hosts":true,"isReplay":true}],"result_subtype":"error_during_execution","result_user_message_uuid_is_the_hosts":true,"tools_used":["Bash"]}
```

Process b (rewound to the interrupted call's line): every stdout line but stream events, and the transcript note after its exit:

```json
# t=0.32
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Task","Bash","…(2 more)"],"mcp_servers":[],"…":"…"}
# t=0.32
{"type":"system","subtype":"status","status":"requesting"}
# t=1.07
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"thinking","…":"…"}],"…":"…"},"…":"…"}
# t=1.34
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"lantern, pebble, violet, copper"}],"…":"…"},"…":"…"}
# t=1.34
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"lantern, pebble, violet, copper","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":0,"…":"…"}
{"transcript":{"transcript_found":true,"lines":87,"lines_that_name":{"lantern":5,"pebble":12,"violet":7,"copper":6},"session_files_in_its_folder":2},"transcript_types_added":["queue-operation","queue-operation","user","attachment/total_tokens_reminder","api-request-blob","api-request-blob","api-request","assistant","assistant","last-prompt","mode","cost-state"]}
```

Process e (rewound to turn 2's user message): its note:

```json
{"asked":"e","ok":true,"words_named":["lantern","pebble"],"result":"done\n\nNote: I skipped the edit because hello.txt already contained `hello pebble`. Running it would have produced `hello pebble pebble`.\n\nlantern, pebble","tools_used":["Read","Bash"],"result_subtype":"success","result_errors":null,"init_came":true,"init_session_id_is_the_first_session":true,"result_session_id_is_the_first_session":true,"lines":["system/init","assistant","…(9 more)"]}
```

Process f (the end of turn 3 again, after d and e): the stderr line and the result:

```json
{"_stderr":"No message found with message.uuid of: uuid-1"}
{"type":"result","subtype":"error_during_execution","is_error":true,"num_turns":0,"errors":["No message found with message.uuid of: uuid-1"]}
```

| label | b: end of turn 4 (interrupted) | c: end of turn 3 | d: end of turn 2 | e: turn 2's user message | f: end of turn 3 again |
|---|---|---|---|---|---|
| 1 | lantern, pebble, violet, copper | lantern, pebble, violet | lantern, pebble | lantern, pebble; tools: Read, Bash | `error_during_execution` |
| 2 | lantern, pebble, violet, copper | lantern, pebble, violet | lantern, pebble | lantern, pebble; tools: Read, Bash | `error_during_execution` |

`--resume-drops-turn`: `r9-rewind-drops-<label>-b` (`--resume-session-at <end of turn 1> --resume-drops-turn <U3>`: the dropped part also holds turn 2) and `-c` (`--resume-session-at <end of turn 2> --resume-drops-turn <U3>`). Flags of `r9-rewind-drops-1-b`: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash,Read,Edit,Agent --resume sess-1 --resume-session-at uuid-1 --resume-drops-turn uuid-2`. The result of b:

```json
{"type":"result","subtype":"error_during_execution","is_error":true,"num_turns":0,"errors":["Resume rejected by --resume-drops-turn: resuming at uuid-1 would discard entries not attributable to turn uuid-2: range does not start with the declared turn prompt; first discarded entry 1 [type=user, uuid=uuid-3]"]}
```

What the adapter can rely on (n=2):
- A rewind to the end of a turn with tool calls (d) or with a helper (c) works like part 6's rewind of text turns:
  the next turn starts (`init`, the answer, `result` `success`, the same session id) and knows only the kept turns
  (d: lantern, pebble; c: lantern, pebble, violet).
- A rewind to the last assistant line of an INTERRUPTED turn (b, the line whose `Bash` call was cut) works too: the
  next turn started at once and knew all four words. No line says that the call has no result: the stream of b is
  `init`, the assistant lines and the `result`; no `user` line with a tool result, nothing on stderr.
- A rewind to a USER message's uuid (e: turn 2's, the host's own uuid) keeps that message and drops the answer to
  it: the next turn first did turn 2's request again (it called tools: Read, Bash; the host's question said "without
  using any tool"), then answered. So the point to rewind to is the end of the last turn to keep, not the user
  message of the first turn to drop.
- The control of part 6's finding: the end of turn 3 resumed BEFORE turns were added on an earlier branch worked (c);
  the same uuid after d and e added turns after turn 2 gave no `init`, `error_during_execution` with
  `No message found with message.uuid of: <uuid>`, exit code 1 (f). So a uuid is found only while it lies on the
  branch of the latest turn: a host that rewinds more than once must rewind to points of that branch.
- The uuid the host sees: with `--include-partial-messages` each complete assistant line of the stream (one per
  content block: a turn with tools has several) has a `uuid`, and every one of them is in the session's transcript;
  the `stream_event` lines carry uuids too, and none of those is in the transcript (4 turns per label). So the host
  keeps the `uuid` of the turn's LAST complete assistant line, never one of a stream event.
- `--resume-drops-turn <the user uuid of the one turn to drop>` with `--resume-session-at <the end of the turn before
  it>` works (c: the turn ran and knew the two kept words). When the dropped part holds more than that turn (b: turns 2
  and 3) the start is refused: no `init`, `error_during_execution` with `Resume rejected by --resume-drops-turn: …
  range does not start with the declared turn prompt …` (n=2 each). A host can pass it as a guard that it drops one turn.
- Not recorded: a rewind while a background command or helper of the dropped part still runs; `--resume-drops-turn`
  when queued messages or task notifications are in the dropped part.

## 3. Fork a conversation that has tool calls, and one whose first process is still alive

Question: `--resume <id> --fork-session`, with and without `--resume-session-at`, of a session with tool calls and a helper; and a fork while the first process still runs with a background command: does the first go on untouched (its next turn, its command)?

Fixtures: `r9-fork-tools-<label>-a` … `-d` and `r9-fork-live-<label>-a`, `-b`, labels 1, 2. Flags of `-a`: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash,Read,Edit,Agent`. Process a: turn 1 (`lantern`) a Read, an Edit and a Bash call; turn 2 (`pebble`) a helper that runs a Bash call. b `--resume <a> --fork-session`; c the same with `--resume-session-at <the end of turn 1>`; d a plain `--resume <a>`. Each asks (no tools) for every secret word. `r9-fork-live`: process a starts a new session whose turn 1 (`harbor`) starts `echo bg-start; sleep 263; echo bg-end` in the background and ends; while a still runs, b is `--resume <a> --fork-session` with one turn; then a gets turn 2 (`anchor`) and a question for every word; `stop_task` ends the command.

| label | turn 1, 2 of a: tools | b: words, new session?, `total_cost_usd` = a + own turn | c: words, new session? | d: words, same session? |
|---|---|---|---|---|
| 1 | Read, Edit, Bash; Agent, Bash | lantern, pebble, new, 0.00153209 = 0.00140230 + 0.00012979 | lantern, new | lantern, pebble, the same |
| 2 | Read, Edit, Bash; Agent, Bash | lantern, pebble, new, 0.00128859 = 0.00115960 + 0.00012899 | lantern, new | lantern, pebble, the same |

The fork of the live session (`r9-fork-live-1-b`): its first lines, up to the empty `result` (no model request: `num_turns` 0), the answer and the note:

```json
# t=0.00
{"type":"system","subtype":"task_notification","task_id":"tid-1","tool_use_id":"toolu-1","status":"stopped","output_file":"","summary":"Background shell command didn't finish before the previous session ended"}
# t=0.28
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=0.28
{"type":"result","subtype":"success","is_error":false,"num_turns":0,"result":"","stop_reason":null,"origin":{"kind":"task-notification"},"queued_turn_count":0,"result_index":0,"…":"…"}
# t=0.28
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=0.92
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"harbor"}],"…":"…"},"…":"…"}
# t=0.92
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"harbor","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":1,"…":"…"}
{"asked":"b","ok":true,"results_with_no_model_request":1,"words_named":["harbor"],"init_session_id_is_the_first_session":false,"first_sessions_last_total_cost_usd":0.00023594000000000002,"total_cost_usd":0.00015207,"cost_of_this_turn":0.00015207,"result_session_id_is_the_first_session":false}
```

What the adapter can rely on (n=2):
- A fork of a session with tool calls and a helper is like part 6's fork of text turns: a NEW session id; b knew both
  words, c (with `--resume-session-at <end of turn 1>`) only `lantern`; the old session answers as before (d: both
  words, its own id); the fork's `total_cost_usd` is the old session's total plus its own turn.
- A fork while the first process still runs with a background command: the first process is untouched. No line came
  to it while the fork ran; its command still ran and stayed in its list; its next turn ran in its own session and
  knew its words; `stop_task` then ended the command.
- The FORK, after the host's first message and before its answer, writes a `task_notification` `"status":"stopped"`
  for that command, `"summary":"Background shell command didn't finish before the previous session ended"`, and a
  `result` with `num_turns` 0 and no model request (cost 0); its answer to the host's message is the NEXT `result`.
  In both runs the host's message was the first line sent, before any line of the CLI: whether the fork writes these
  lines by itself when no message comes is not recorded. The command was NOT stopped: it still ran in the first
  process. A host must not end that command's row on this line, and must not take the empty `result` as the answer
  to its message.
- The fork knew `harbor` (the first process's turn before the fork) and nothing after it.
- The live fork's `total_cost_usd` is its own turn only (0.00015207 / 0.00015185), not the first session's total
  (0.00023594 / 0.00023554) plus its turn, unlike the fork of an ended session above: a host that adds up a
  session's cost must not read the fork's total as carrying the first session's.
- Not recorded: a fork while the first process's TURN runs; a fork of a session whose helper still runs in the
  background; a fork that gets no message at once; a fork of a session whose last turn was interrupted in a tool call
  (the forks here are of a new two-turn session, not of section 2's session); a plain `--resume` of a live session
  (the control for whether the `stopped` line belongs to the fork).

## 4. The CLI's own file rewind beyond the Edit tool

Question: with `CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING=1`, `rewind_files` after a turn that wrote a file with Bash, made a new file with Write, and had a helper edit a file: which files go back; is the next turn told anything?

Fixtures: `r9-file-rewind-more-1`, `r9-file-rewind-more-2`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash,Read,Edit,Write,Agent`, with the variable in the CLI's environment. The work folder has `notes.txt` (`notes`) and `README.md` (first line `# demo`). Turn 1 (host uuid U1): Bash writes `notes.txt` (`bash wrote this`), the Write tool makes `new.txt` (`new file`), a helper (foreground) edits `README.md`'s first line to `# demo edited by helper`. Then `rewind_files U1` as a dry run and for real; then turn 2 asks (no tools) whether anything told it that files were put back.

The two requests and answers of `r9-file-rewind-more-1`:

```json
{"type":"control_request","request_id":"host-1","request":{"subtype":"rewind_files","user_message_id":"user-msg-1","dry_run":true}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"canRewind":true,"filesChanged":["/tmp/work/r9-file-rewind-more-1/new.txt"],"insertions":0,"deletions":1}}}
{"type":"control_request","request_id":"host-2","request":{"subtype":"rewind_files","user_message_id":"user-msg-1","dry_run":false}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-2","response":{"canRewind":true,"skippedLinks":0}}}
```

| label | tools of turn 1 (helper's marked) | files after turn 1 | dry run: `filesChanged` | files after the rewind | transcript lines the rewind added | turn 2's answer |
|---|---|---|---|---|---|---|
| 1 | Bash, Write, Agent, Read (helper), Edit (helper) | notes.txt: `bash wrote this`, new.txt: `new file`, README.md: `# demo edited by helper` | `new.txt` | notes.txt: `bash wrote this`, new.txt: gone, README.md: `# demo edited by helper` | 0 | No. Nothing since my last turn said files in this folder were restored to an earlier state. The Bash, Write, and subagent results reported only their own actions. |
| 2 | Bash, Write, Agent, Read (helper), Edit (helper) | notes.txt: `bash wrote this`, new.txt: `new file`, README.md: `# demo edited by helper` | `new.txt` | notes.txt: `bash wrote this`, new.txt: gone, README.md: `# demo edited by helper` | 0 | No. Nothing since my last turn told me any files in this folder were restored to an earlier state. |

What the adapter can rely on (n=2):
- `rewind_files` puts back only what the CLI's own file tools of the MAIN thread changed: the file the Write tool made
  in the turn was DELETED (the dry run listed only it: `insertions` 0, `deletions` 1). The file Bash wrote kept its
  new text, and the file the HELPER changed with its Edit tool kept the helper's change. Neither was in the dry run's
  `filesChanged`. With part 6 (an Edit of the main thread put back, n=2) that is what is recorded.
- The rewind added no line to the session's transcript, the stream had no line about it, and the next turn was told
  nothing: its transcript lines are those of any turn (no attachment about files), and the model answered "no".
- So the CLI's own rewind cannot replace Codly's restore: it misses Bash and helpers, and it tells the agent
  nothing; Codly's restore note (`with_restore_note`) is still needed after a restore.
- Not recorded: a file a helper made new with Write; a file Bash deleted; a background helper's edit.

## 5. Your own slash commands and skills

Question: with a user command, a project command and a skill: what does `init` list; what does a user message "/<command> <words>" do in stream-json (does the command's prompt run, which lines mark it); an unknown "/nope"; a project command in a folder that is not trusted (`--setting-sources user`)?

Fixtures: `r9-own-commands-<variant>-<label>`. The scenario's files: a user command `HOME/.claude/commands/codly-hello.md` and a user skill `HOME/.claude/skills/codly-skill/SKILL.md` (throw-away HOME variants only), a project command `.claude/commands/codly-proj.md` and a project skill `.claude/skills/codly-proj-skill/SKILL.md` in the work folder. Each asks for one fixed reply (`hello says <words>`, `proj says <words>`, `skill ran`, `proj skill ran`). Variants: `home-all` (throw-away HOME, no login, `--setting-sources user,project,local`), `home-user` (the same, `--setting-sources user`), `login` (the real login, `--strict-mcp-config --setting-sources project`: only the work folder's files are a source). Flags of `r9-own-commands-login-1`: `--permission-mode default --strict-mcp-config --setting-sources project --tools Bash --replay-user-messages --settings '{"disableAllHooks": true}'`. Messages, one turn each: `/codly-hello world`, `/codly-proj world`, `/codly-skill`, `/codly-proj-skill`, `/nope world`.

`r9-own-commands-login-1`: the echo of each message (`--replay-user-messages`) and the turn's result:

```json
# t=0.91
{"type":"user","message":{"role":"user","content":"/codly-hello world"},"isReplay":true,"…":"…"}
# t=1.70
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"The `/codly-hello` command isn't installed in this session, so I didn't run it. Treating your message as a plain request: hello, world!\n\nIf you meant to use `/c…","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":0,"…":"…"}
# t=2.83
{"type":"user","message":{"role":"user","content":"<command-message>codly-proj</command-message>\n<command-name>/codly-proj</command-name>\n<command-args>world</command-args>"},"isReplay":true,"…":"…"}
# t=3.18
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"proj says world","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":1,"…":"…"}
# t=4.65
{"type":"user","message":{"role":"user","content":"/codly-skill"},"isReplay":true,"…":"…"}
# t=4.91
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"The `/codly-skill` command isn't installed in this session, so I didn't run it. Your message doesn't include any other request. What would you like me to do?","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":2,"…":"…"}
# t=6.12
{"type":"user","message":{"role":"user","content":"<command-message>codly-proj-skill</command-message>\n<command-name>/codly-proj-skill</command-name>"},"isReplay":true,"…":"…"}
# t=6.38
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"proj skill ran","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":3,"…":"…"}
# t=7.61
{"type":"user","message":{"role":"user","content":"/nope world"},"isReplay":true,"…":"…"}
# t=7.61
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"The `/nope` command isn't installed in this session, so I didn't run it. Treating your message as a plain request: \"nope, world\" — is there something you'd like…","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":4,"…":"…"}
```

| variant | `init.slash_commands` of the scenario | `init.skills` of the scenario | `/codly-hello world` | `/codly-proj world` | `/codly-skill` | `/codly-proj-skill` | `/nope world` |
|---|---|---|---|---|---|---|---|
| `home-all-1` | `codly-hello`, `codly-proj`, `codly-proj-skill`, `codly-skill` | `codly-proj-skill`, `codly-skill` | `Not logged in` | `Not logged in` | `Not logged in` | `Not logged in` | `Not logged in` |
| `home-all-2` | `codly-hello`, `codly-proj`, `codly-proj-skill`, `codly-skill` | `codly-proj-skill`, `codly-skill` | `Not logged in` | `Not logged in` | `Not logged in` | `Not logged in` | `Not logged in` |
| `home-user-1` | `codly-hello`, `codly-skill` | `codly-skill` | `Not logged in` | `Not logged in` | `Not logged in` | `Not logged in` | `Not logged in` |
| `home-user-2` | `codly-hello`, `codly-skill` | `codly-skill` | `Not logged in` | `Not logged in` | `Not logged in` | `Not logged in` | `Not logged in` |
| `login-1` | `codly-proj`, `codly-proj-skill` | `codly-proj-skill` | The `/codly-hello` command isn't installed in this session, … | proj says world | The `/codly-skill` command isn't installed in this session, … | proj skill ran | The `/nope` command isn't installed in this session, so I di… |
| `login-2` | `codly-proj`, `codly-proj-skill` | `codly-proj-skill` | The `/codly-hello` command isn't installed in this session, … | proj says world | The `/codly-skill` command isn't installed in this session, … | proj skill ran | The `/nope` command isn't installed in this session, so it d… |

What the adapter can rely on:
- `init.slash_commands` lists the user's commands and skills and the project's (a skill is listed in both
  `slash_commands` and `skills`) for the sources that are on: `user,project,local` lists all four, `user` only the
  user's two, `project` only the project's two (n=2 each). A project command or skill of a folder you have not
  trusted (`--setting-sources user`) is not listed.
- A user message "/<command> <words>" of a LISTED command runs its prompt: the CLI echoes it (`isReplay`) as
  `<command-message>…</command-message><command-name>/…</command-name><command-args>…</command-args>`, and the model
  answered the command's own reply (`proj says world`, `proj skill ran`; logged in, n=2). No other line marks it:
  the turn's lines are `init`, the echo, the assistant lines and `result` `success`.
- A message "/<name>" that no listed command has (`/nope`, and `/codly-hello`, `/codly-skill` where the user source is
  off) goes to the MODEL as plain text: the echo is the text as sent, the model answered that the command is not
  installed (n=2). No local error line, no refusal. So the composer may send an unknown "/" text, but the user then
  gets a model turn, not an error.
- Without a login the throw-away HOME runs show the lists only: each message ended with `Not logged in` (no prompt
  ran). Not recorded: a USER command's prompt with a login (it needs a command file in the owner's real
  `~/.claude/commands`); `isReplay` lines of a command that has its own `allowed-tools`; a plugin's command.

## 6. A turn with no network

Question: a turn with the API unreachable (a proxy variable that points to a closed local port; a host name that does not resolve): which lines come, how long until the turn ends, the error text and the exit code, and what a turn does when the network is back.

Fixtures: `r9-no-network-proxy-1`, `r9-no-network-proxy-2` (`HTTPS_PROXY`, `HTTP_PROXY` and their lower-case spellings point to a port on 127.0.0.1 that the script holds bound but not listening, so every connection is refused; turn 1 runs; then the script makes that port a CONNECT proxy that only copies bytes, and turn 2 runs in the same process) and `r9-no-network-dns-<label>-a`, `-b` (`ANTHROPIC_BASE_URL=https://api.no-such-host.invalid`, a name that never resolves; turn 1 runs; process b resumes the session with no such variable). Flags of `r9-no-network-proxy-1`: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash`.

`r9-no-network-proxy-1`, turn 1: every line but stream events, from the host's message to the result:

```json
# t=23.39
{"type":"system","subtype":"api_retry","attempt":1,"max_retries":10,"retry_delay_ms":552,"error_status":null,"error":"unknown"}
# t=31.72
{"type":"system","subtype":"api_retry","attempt":2,"max_retries":10,"retry_delay_ms":1108,"error_status":null,"error":"unknown"}
# t=40.63
{"type":"system","subtype":"api_retry","attempt":3,"max_retries":10,"retry_delay_ms":2153,"error_status":null,"error":"unknown"}
# t=50.57
{"type":"system","subtype":"api_retry","attempt":4,"max_retries":10,"retry_delay_ms":4101,"error_status":null,"error":"unknown"}
# t=62.46
{"type":"system","subtype":"api_retry","attempt":5,"max_retries":10,"retry_delay_ms":9839,"error_status":null,"error":"unknown"}
# t=80.08
{"type":"system","subtype":"api_retry","attempt":6,"max_retries":10,"retry_delay_ms":18132,"error_status":null,"error":"unknown"}
# t=106.00
{"type":"system","subtype":"api_retry","attempt":7,"max_retries":10,"retry_delay_ms":39002,"error_status":null,"error":"unknown"}
# t=152.82
{"type":"system","subtype":"api_retry","attempt":8,"max_retries":10,"retry_delay_ms":35116,"error_status":null,"error":"unknown"}
# t=195.71
{"type":"system","subtype":"api_retry","attempt":9,"max_retries":10,"retry_delay_ms":37199,"error_status":null,"error":"unknown"}
# t=240.70
{"type":"system","subtype":"api_retry","attempt":10,"max_retries":10,"retry_delay_ms":34466,"error_status":null,"error":"unknown"}
# t=282.95
{"type":"assistant","message":{"model":"<synthetic>","content":[{"type":"text","text":"Request timed out"}],"…":"…"},"…":"…"}
# t=282.95
{"type":"result","subtype":"success","is_error":true,"num_turns":1,"result":"Request timed out","stop_reason":"stop_sequence","terminal_reason":"api_error","queued_turn_count":0,"result_index":0,"…":"…"}
```

| recording | turn 1: `api_retry` lines (attempt: s after the message, `retry_delay_ms`) | turn 1: result after | its text, `is_error`, `terminal_reason` | turn 2: result after, text |
|---|---|---|---|---|
| `r9-no-network-proxy-1` | 1: 15.6, 552; 2: 23.9, 1108; 3: 32.9, 2153; 4: 42.8, 4101; 5: 54.7, 9839; 6: 72.3, 18132; 7: 98.2, 39002; 8: 145.0, 35116; 9: 187.9, 37199; 10: 232.9, 34466 | 275.2 s | `Request timed out`, true, `api_error` | 0.64 s, `pong` |
| `r9-no-network-proxy-2` | 1: 15.6, 560; 2: 23.9, 1171; 3: 32.9, 2043; 4: 42.7, 4172; 5: 54.7, 9504; 6: 72.0, 17033; 7: 96.8, 38189; 8: 142.8, 37282; 9: 187.9, 39672; 10: 235.3, 39314 | 282.4 s | `Request timed out`, true, `api_error` | 0.66 s, `pong` |
| `r9-no-network-dns-1-a` | 1: 0.3, 510; 2: 0.8, 1141; 3: 2.0, 2101; 4: 4.1, 4708; 5: 8.8, 8152; 6: 16.9, 18607; 7: 35.6, 32917; 8: 68.5, 34243; 9: 102.7, 39895; 10: 142.6, 38383 | 181.0 s | `API Error: Can't reach the API server — check your internet or DNS (ENOTFOUND)`, true, `api_error` | 1.33 s, `pong` |
| `r9-no-network-dns-2-a` | 1: 0.3, 624; 2: 0.9, 1036; 3: 2.0, 2053; 4: 4.0, 4064; 5: 8.1, 8227; 6: 16.3, 17356; 7: 33.7, 38815; 8: 72.5, 37278; 9: 109.8, 39040; 10: 148.8, 38464 | 187.3 s | `API Error: Can't reach the API server — check your internet or DNS (ENOTFOUND)`, true, `api_error` | 1.06 s, `pong` |

What the adapter can rely on (n=2 per way):
- With the service unreachable the turn does not fail at once: the CLI retries. Each try is a line
  `{"type":"system","subtype":"api_retry","attempt":N,"max_retries":10,"retry_delay_ms":…,"error_status":null,"error":"unknown"}`.
  A refused proxy gave all 10 tries, and the turn's `result` came 275.2 / 282.4 s after the message. The waits
  (`retry_delay_ms`, lowest and highest per attempt over the 4 runs of both ways) grow:
  1: 0.5-0.6 s, 2: 1.0-1.2 s, 3: 2.0-2.2 s, 4: 4.1-4.7 s, 5: 8.2-9.8 s, 6: 17.0-18.6 s, 7: 32.9-39.0 s, 8: 34.2-37.3 s, 9: 37.2-39.9 s, 10: 34.5-39.3 s.
  Through the refused proxy each try itself also took some seconds
  (the first retry line came 15.6 / 15.6 s after the message); for the name that does not resolve it failed at once (0.3 / 0.3 s).
- The end of the turn: an assistant line with `"model":"<synthetic>"`, `"error":"server_error"`,
  `"is_api_error_message":true` and the text, then the `result` with `"subtype":"success"` (not an error subtype),
  `"is_error":true`, `"terminal_reason":"api_error"`, `api_error_status` null and `result` = the same text. The text
  is `Request timed out` for the refused proxy (n=2): it does not say "no network". For the name that does not
  resolve: `API Error: Can't reach the API server — check your internet or DNS (ENOTFOUND)`, after 10 tries, 181.0 / 187.3 s after the message. Nothing on stderr. The process stays up and takes the next
  message; when stdin was closed after the failed turn (the `dns` runs' process a) it exited with code 1.
- When the network is back the next turn works at once with no retry line: in the same process (the proxy up again:
  `pong` in under a second, n=2), and in a new process that resumes the session (n=2).
- So the adapter can tell "cannot reach the service" by `terminal_reason` `api_error` with `api_error_status` null
  after `api_retry` lines whose `error_status` is null; the words alone (`Request timed out`) are not enough. A host
  that shows each `api_retry` line can say "trying again (N of 10)" for up to about 5 minutes.
- The two ways gave two different texts and durations, so a third way may give a third: the adapter should not
  rely on the text.
- Not recorded: a machine with no network at all (no route, Wi-Fi off), the case a user meets most; a failed name
  lookup of the DEFAULT API host (the `dns` runs set `ANTHROPIC_BASE_URL`); a network that comes back during the
  retries; `CLAUDE_CODE_MAX_RETRIES` or another limit on them; a proxy that answers with an error; a TLS failure.

## 7. A conversation started in the terminal

Question: where are a folder's sessions kept (names and counts only)? Does `--resume <id>` in stream-json of a session that an interactive `claude` started work (`init`, the first turn, what it knows)? What does `claude --resume` with an unknown id say?

Fixtures: `r9-terminal-<label>-a` (notes only: the interactive `claude --model haiku --strict-mcp-config --setting-sources local` in a pseudo-terminal, one message "remember the word meadow", then `/exit`), `-b` (stream-json `--resume <that session>`: one turn asks for the word; flags of `r9-terminal-1-b`: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --resume sess-1`) and `-c` (notes only: the interactive `claude --resume <a uuid no session has>`), labels 1, 2. Label 1 ran in a folder where an earlier try of the same script had made one session and had trusted the folder (so its "before" list has one file and it was not asked again); label 2 ran in a new folder.

The note of `r9-terminal-1-a`:

```json
{"asked_whether_to_trust_the_folder":false,"exit_code":0,"sessions_folder":".claude/projects/-tmp-work-r9-terminal-1","session_files_before":["uuid-1.jsonl"],"session_files_after":["uuid-1.jsonl","uuid-2.jsonl"],"session_found":true,"transcript_types":["mode","permission-mode","atis-latch","file-history-snapshot","user","attachment/environment","attachment/model","attachment/deferred_tools_delta","attachment/agent_listing_delta","attachment/skill_listing","attachment/total_tokens_reminder","attachment/session_context","attachment/date","attachment/credential_org","attachment/remote_session_change","attachment/prompt_snapshot","api-request-shape","api-request-blob","api-request-blob","api-request","last-prompt","mode","permission-mode","atis-latch","ai-title","assistant","attachment/prompt_snapshot","system/turn_duration","last-prompt","ai-title","mode","permission-mode","atis-latch","cost-state","file-history-snapshot","cost-state"],"entrypoint":"cli","other_entries_in_the_folder":["memory"]}
```

The note of `r9-terminal-2-a`:

```json
{"asked_whether_to_trust_the_folder":true,"exit_code":0,"sessions_folder":".claude/projects/-tmp-work-r9-terminal-2","session_files_before":[],"session_files_after":["uuid-1.jsonl"],"session_found":true,"transcript_types":["mode","permission-mode","atis-latch","file-history-snapshot","user","attachment/environment","attachment/model","attachment/deferred_tools_delta","attachment/agent_listing_delta","attachment/skill_listing","attachment/total_tokens_reminder","attachment/session_context","attachment/date","attachment/credential_org","attachment/remote_session_change","attachment/prompt_snapshot","api-request-shape","api-request-blob","api-request-blob","api-request","last-prompt","mode","permission-mode","atis-latch","ai-title","assistant","attachment/prompt_snapshot","system/turn_duration","last-prompt","ai-title","mode","permission-mode","atis-latch","cost-state","file-history-snapshot","cost-state"],"entrypoint":"cli","other_entries_in_the_folder":["memory"]}
```

| label | asked to trust the folder | sessions folder (under the HOME) | session files before / after | `entrypoint` | b: init and result in that session, words | c: exit code, text |
|---|---|---|---|---|---|---|
| 1 | false | `~/.claude/projects/-tmp-work-r9-terminal-1` | 1 / 2 | `cli` | yes, yes, meadow | 1, `No conversation found with session ID: sess-1` |
| 2 | true | `~/.claude/projects/-tmp-work-r9-terminal-2` | 0 / 1 | `cli` | yes, yes, meadow | 1, `No conversation found with session ID: sess-1` |

What the adapter can rely on (n=2):
- A folder's sessions are kept in `~/.claude/projects/<the folder's path, each character that is not a letter or a
  digit made '-'>/`, one file `<session id>.jsonl` per session, whether the terminal or stream-json started it (a
  `memory` folder sits next to them). The transcript's lines carry `"entrypoint":"cli"` for a terminal session
  (part 1's stream-json sessions: `sdk-cli`, not checked here). A list of an agent's earlier conversations can be
  made from that folder: the names are the session ids.
- `--resume <id>` in stream-json of a session the interactive `claude` started works: `init` and the `result` carry
  that session's id, the first turn ran and knew the word told in the terminal.
- The interactive `claude --resume <unknown id>` prints `No conversation found with session ID: <id>` and exits with
  code 1 at once (no screen of its own): the same words as stream-json's error for a missing session (part 3,
  `r5-resume-missing-*`, and part 1, `resume-missing`).
- The first interactive start in a folder asks whether you trust it (choices "No, exit" and "Yes, I trust this
  folder"; the first is the default); the script answered yes, which the CLI saved for that folder, and the second
  start in the same folder did not ask.
- Not recorded: a resume while the terminal `claude` still runs the session; the terminal resuming a session Codly
  started headless; sessions of a git worktree of the folder.

## 8. A slow tool of a streaming HTTP MCP server that sends nothing for more than 5 minutes

Question: a tool that sends nothing for 330 s on an HTTP server that answers with an event stream: when and how does the call end, do the 30 s heartbeat lines go on until then? One run is enough for this question (n=1).

Fixture: `r9-http-silent-1`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --mcp-config /tmp/work/r9-http-silent-1.json`. The server `ping-http` (`mcp_http_ping_server.py`, answer kind `sse`) sends the event stream's headers at once and then nothing until 330 s.

The tool's result line and the note:

```json
# t=300.99
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"MCP server \"ping-http\" tool \"ping\" sent no response or progress for 300s; aborting. If this server is configured in your MCP settings, set a per-server \"timeout\" (ms) to allow longer silent runs for just this server; otherwise set CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT (ms) globally (0 disables).","is_error":true,"tool_use_id":"toolu-1"}]},"…":"…"}
{"tool_progress_count":10,"tool_progress_first_and_last":[{"type":"tool_progress","tool_use_id":"toolu-1-heartbeat-0","tool_name":"mcp__ping-http__ping","parent_tool_use_id":"toolu-1","elapsed_time_seconds":30,"heartbeat":true,"_t":30.977},{"type":"tool_progress","tool_use_id":"toolu-1-heartbeat-9","tool_name":"mcp__ping-http__ping","parent_tool_use_id":"toolu-1","elapsed_time_seconds":300,"heartbeat":true,"_t":300.987}],"server_requests":[{"at":0.227,"http":"POST","jsonrpc_method":"server/discover","answered_at":0.227},{"at":0.242,"http":"POST","jsonrpc_method":"initialize","answered_at":0.242},{"at":0.244,"http":"POST","jsonrpc_method":"notifications/initialized","answered_at":0.244},{"at":0.245,"http":"GET","answered_at":0.245},{"at":0.246,"http":"POST","jsonrpc_method":"tools/list","answered_at":0.246},{"at":0.978,"http":"POST","jsonrpc_method":"tools/call","progress_token_in_request":true,"headers_sent_at":0.978}],"tool_results":[{"is_error":true,"content":"MCP server \"ping-http\" tool \"ping\" sent no response or progress for 300s; aborting. If this server is configured in your MCP settings, set a per-server \"timeout\" (ms) to allow longer silent runs for j"}],"can_use_tool_requests":[["mcp__ping-http__ping","",null]],"seconds_from_tool_use_to_tool_result":300.019,"seconds_from_tool_use_to_each_tool_progress":[30.005,60.007,90.007,120.007,150.008,180.009,210.01,240.011,270.012,300.015]}
```

What the adapter can rely on (n=1):
- The CLI ends such a call itself at 300 s of silence (300.0 s after the `tool_use` line): the tool result is an error,
  `MCP server "ping-http" tool "ping" sent no response or progress for 300s; aborting. …`, and the text names the two
  ways to change it: a per-server `"timeout"` (ms) in the MCP settings, or `CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT` (ms,
  `0` turns the limit off). The turn went on (the model answered the error; `result` `success`).
- The `tool_progress` heartbeat lines went on until then: one each 30 s, `elapsed_time_seconds` 30 to 300 (10 lines),
  the last one at the moment of the error.
- The server still held the request (its entry has neither an answer time nor a failed write at the end of the
  run): what the CLI did with the connection is not visible to the server until it writes.
- So the watchdog may treat an HTTP tool with heartbeats as alive up to 300 s and expect the CLI's own error then;
  the user's sentence can say "the server sent nothing for 5 minutes" and name the `timeout` key.
- Not recorded: a server that sends progress notifications only (part 6: they add no line), a stdio server's silence,
  the two settings themselves.

## Notes written late

Each note of this part that lists lines is compared with the lines in the fixture between the step the script marked just before it (named in the last column) and the note:

| fixture | note | lines in its list | lines in the file before it | counted from |
|---|---|---|---|---|
| `r9-file-rewind-more-1` | `lines` | 7 | 7 | the host's last message |
| `r9-file-rewind-more-2` | `lines` | 7 | 7 | the host's last message |
| `r9-fork-live-1-a` | `lines_while_the_fork_ran` | 0 | 0 | the note with `background_task_id` |
| `r9-fork-live-2-a` | `lines_while_the_fork_ran` | 0 | 0 | the note with `background_task_id` |
| `r9-no-network-dns-1-a` | `lines` | 13 | 13 | the host's last message |
| `r9-no-network-dns-1-b` | `lines` | 3 | 3 | the host's last message |
| `r9-no-network-dns-2-a` | `lines` | 13 | 13 | the host's last message |
| `r9-no-network-dns-2-b` | `lines` | 3 | 3 | the host's last message |
| `r9-no-network-proxy-1` | `lines` | 13 | 13 | the host's last message |
| `r9-no-network-proxy-1` | `lines` | 3 | 3 | the host's last message |
| `r9-no-network-proxy-2` | `lines` | 13 | 13 | the host's last message |
| `r9-no-network-proxy-2` | `lines` | 3 | 3 | the host's last message |
| `r9-own-commands-home-all-1` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-home-all-1` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-home-all-1` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-home-all-1` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-home-all-1` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-home-all-2` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-home-all-2` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-home-all-2` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-home-all-2` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-home-all-2` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-home-user-1` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-home-user-1` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-home-user-1` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-home-user-1` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-home-user-1` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-home-user-2` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-home-user-2` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-home-user-2` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-home-user-2` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-home-user-2` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-login-1` | `lines` | 5 | 5 | the host's last message |
| `r9-own-commands-login-1` | `lines` | 5 | 5 | the host's last message |
| `r9-own-commands-login-1` | `lines` | 5 | 5 | the host's last message |
| `r9-own-commands-login-1` | `lines` | 5 | 5 | the host's last message |
| `r9-own-commands-login-1` | `lines` | 4 | 4 | the host's last message |
| `r9-own-commands-login-2` | `lines` | 5 | 5 | the host's last message |
| `r9-own-commands-login-2` | `lines` | 5 | 5 | the host's last message |
| `r9-own-commands-login-2` | `lines` | 5 | 5 | the host's last message |
| `r9-own-commands-login-2` | `lines` | 5 | 5 | the host's last message |
| `r9-own-commands-login-2` | `lines` | 4 | 4 | the host's last message |
| `r9-rewind-drops-1-b` | `lines` | 1 | 1 | the process's start |
| `r9-rewind-drops-1-c` | `lines` | 4 | 4 | the process's start |
| `r9-rewind-drops-2-b` | `lines` | 1 | 1 | the process's start |
| `r9-rewind-drops-2-c` | `lines` | 3 | 3 | the process's start |
| `r9-rewind-tools-1-b` | `lines` | 4 | 4 | the process's start |
| `r9-rewind-tools-1-c` | `lines` | 3 | 3 | the process's start |
| `r9-rewind-tools-1-d` | `lines` | 3 | 3 | the process's start |
| `r9-rewind-tools-1-e` | `lines` | 11 | 11 | the process's start |
| `r9-rewind-tools-1-f` | `lines` | 1 | 1 | the process's start |
| `r9-rewind-tools-2-b` | `lines` | 4 | 4 | the process's start |
| `r9-rewind-tools-2-c` | `lines` | 4 | 4 | the process's start |
| `r9-rewind-tools-2-d` | `lines` | 3 | 3 | the process's start |
| `r9-rewind-tools-2-e` | `lines` | 10 | 10 | the process's start |
| `r9-rewind-tools-2-f` | `lines` | 1 | 1 | the process's start |

All 56 notes of this part that list lines list exactly the lines in their window: none is late.

## Can the waiting code be built now? (as of the day of recording)

| # | the code that waits | can it be built now? |
|---|---|---|
| 1 | `claude-part5-truth` item 1 (`claude.rs` `build_args`, `orchestrator/trust.rs`): drop `--strict-mcp-config` in a folder you have not trusted when "Use my Claude setup" is on | yes for `--setting-sources user` alone: no file of the folder started or loaded anything in 29 recordings of parts 4 to 7, and a plugin a repo file names but the user never installed is not loaded (n=8). NO for `user,local`: the folder's `.claude/settings.local.json` runs the hooks and the server of a plugin the user turned off (n=2), and part 4 saw that file's own hooks run. Never pass `local` in an untrusted folder |
| 2 | "Undo a turn" for the conversation (`checkpoints.rs`, `src/home/rewind.ts`) | yes: `--resume <id> --resume-session-at <uuid of the LAST complete assistant line of the last turn to keep>` works after tool calls, a helper and an interrupted turn (n=2 each), with no line about a cut call. Never a stream event's uuid; not a user message's uuid (the next turn redoes that message). Rewind only to points of the latest branch; `--resume-drops-turn <U>` can guard that exactly one turn goes |
| 3 | "Try again from here" / "branch this agent" | yes: as part 6, also with tool calls and a helper (n=2). A fork of a LIVE session leaves the first process untouched, but the fork itself reports the first's background command as `stopped` and sends an empty `result` before its answer to the host's first message (n=2): the adapter must ignore both. Not recorded: a fork of a session whose last turn was interrupted |
| 4 | Codly's restore note (`with_restore_note`, `checkpoints.rs`) vs the CLI's own rewind | no: `rewind_files` puts back only the main thread's file tools (a Write's new file deleted; part 6: an Edit), not Bash's writes and not a helper's edit, and tells the agent nothing (n=2). Keep Codly's restore and its note |
| 5 | The composer's "/" menu (`composerMenu.ts`) and `slash_commands` / `skills` of `init` | yes: `init.slash_commands` lists the user's and the project's commands and skills for the sources on (n=2 per variant); a listed command's prompt runs, echoed as `<command-message>…`; an unknown "/" text goes to the model as plain text (n=2): it gives a model turn, not an error. The menu can offer what `init` lists |
| 6 | The failure words for "cannot reach the service" (`claude.rs`) | yes for these two forms (a machine with no network at all, and a failed lookup of the default host, are not recorded): up to 10 `api_retry` lines with `error_status` null, then `terminal_reason` `api_error`, `is_error` true, `api_error_status` null, text `Request timed out` (refused proxy, n=2, about 4.6 minutes) or `API Error: Can't reach the API server — check your internet or DNS (ENOTFOUND)` (name that does not resolve, n=2, about 3 minutes); the next turn works when the network is back (n=2 per way) |
| 7 | A list of an agent's earlier conversations; "pick up what I started in my terminal" | yes: `~/.claude/projects/<folder path with '-'>/<session id>.jsonl`; stream-json `--resume <id>` of a terminal session works and knows its turns (n=2) |
| 8 | The watchdog's limits for HTTP tools (`watchdog.rs`) | yes (n=1): a silent HTTP tool ends at 300 s with the CLI's own error naming `timeout` and `CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT`; the 30 s heartbeats go on until then |

## Table: question -> answer -> fixture

| # | question | answer | n | fixture |
|---|---|---|---|---|
| 1 | A plugin named by a repo file; a plugin's hooks; `user` vs `user,local` | Never installed: not loaded under either; turned off for the user, on in the repo's local file: hooks and server RUN under `user,local`, not under `user`; under `user` alone nothing of the folder ran in parts 4 to 7 | 2 per form and sources (two starts each) | `r9-plugin-from-repo-*` |
| 2 | Rewind with tool calls, a helper, an interrupted turn; user message uuid; `--resume-drops-turn` | Each works and knows only the kept turns; no line about a cut call; a user message's uuid redoes it; a uuid off the latest branch is not found (control: found before); drops-turn refuses a range of two turns | 2 | `r9-rewind-tools-*`, `r9-rewind-drops-*` |
| 3 | Fork with tool calls; fork of a live session | As part 6; the first process untouched; the fork says the first's command `stopped` and sends an empty `result` first | 2 | `r9-fork-tools-*`, `r9-fork-live-*` |
| 4 | File rewind beyond Edit | A Write's new file deleted; Bash's write and a helper's edit kept; nothing tells the agent | 2 | `r9-file-rewind-more-*` |
| 5 | Own slash commands and skills | Listed in `init` per source; a listed one's prompt runs (`<command-message>` echo); an unknown one goes to the model as text | 2 per variant | `r9-own-commands-*` |
| 6 | A turn with no network | 10 `api_retry` lines, then `api_error` with a synthetic text (`Request timed out`; for DNS `Can't reach the API server …`), 3 to 5 minutes; the next turn works when the network is back | 2 per way | `r9-no-network-*` |
| 7 | A conversation started in the terminal | Kept as `~/.claude/projects/<folder>/<id>.jsonl`; stream-json `--resume` of it works; unknown id: `No conversation found with session ID: <id>`, exit 1 | 2 | `r9-terminal-*` |
| 8 | An HTTP tool silent for more than 5 minutes | Ended by the CLI at 300 s with an error naming the settings; heartbeats every 30 s until then | 1 | `r9-http-silent-1` |

## Not verified

Not possible here (named limits of this part):
- An MCP switch (`mcp_toggle`) while a turn runs: a switch is saved in the HOME's `.claude.json`, so it needs a
  throw-away HOME, and a turn there needs a login in it (the owner's decision).
- A real sign-in to an MCP server (`mcp_authenticate`; it may write to the login keychain).
- Linux process groups (this machine is macOS).
- The plugin forms of section 1 with a real login (section 1 ran with a throw-away HOME only, on purpose).

Not recorded in this part (still open):
- 1: a marketplace a repo file declares with a network source (`github`, `git`, `url`) or an inline `settings` source;
  a plugin's skills and commands; the plugin forms with `project` among the sources.
- 2: a rewind while a background command or helper of the dropped part still runs; `--resume-drops-turn` with queued
  messages or task notifications in the dropped part.
- 3: a fork while the first process's turn runs; a fork of a session whose helper runs in the background; a fork
  that gets no message at once; a fork of a session whose last turn was interrupted; a plain `--resume` of a live
  session (control for whether the `stopped` line is the fork's).
- 4: a file a helper made new; a file Bash deleted; a background helper's edit.
- 5: a user command's prompt with a login; a command with its own `allowed-tools`; a plugin's command.
- 6: a machine with no network at all (no route), and a failed name lookup of the default API host; a network that
  comes back during the retries; a limit on the retries; a proxy that answers with an error; a TLS failure.
- 7: a resume while the terminal `claude` still runs the session; the terminal resuming a session Codly started.
- 8: a stdio server's silence; the per-server `timeout` and `CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT` themselves (named by
  the error text, not recorded).
- From part 6, still open: the text printed when `failIfUnavailable` stops a start; a `tool_progress` line for
  WebFetch; a network allow that lasts hours; whether a model request carries the effort level `get_settings`
  shows; the `can_use_tool` request of a user-scope server; a turn that fails by an API error while a helper works.

