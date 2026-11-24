# Claude Code CLI stream-json: recorded protocol shapes, part 6

**QUESTION 1 FIRST. UNDER `--setting-sources user,local`, WITHOUT `--strict-mcp-config`, A FILE OF THE REPO
(`.claude/settings.local.json` WITH `"enabledPlugins": {"<plugin>": true}`) SWITCHES ON A PLUGIN THAT THE USER HAD
INSTALLED AND TURNED OFF, AND THE PLUGIN'S MCP SERVER STARTS (4 of 4 recordings of that form, `r8-untrusted-mcp-plugin-repo-local-user-local-1`, `r8-untrusted-mcp-plugin-repo-local-user-local-2`, `r8-untrusted-mcp-plugin-repo-user-local-1`, `r8-untrusted-mcp-plugin-repo-user-local-2`). UNDER `user` ALONE
IT DOES NOT. THE REPO'S OWN `.mcp.json` SERVER STARTED IN NONE OF THE 24 RECORDINGS OF SECTION 1, ALSO WHEN THE USER
SETTINGS FILE SAYS `enableAllProjectMcpServers` OR NAMES THE SERVER IN `enabledMcpjsonServers`.** So the item of package
`claude-part5-truth` (drop `--strict-mcp-config` in a folder you have not trusted when "Use my Claude setup" is on)
holds for `--setting-sources user` and must be taken back for `user,local`: there the repo can turn on code (a
plugin's server; a plugin can also bring hooks, not recorded) that the user had turned off.

- CLI: `claude` 2.1.311 (the same version as parts 1 to 5; checked with `claude --version` before the runs).
- Recorded: 2026-11-23, macOS, `--model haiku` (`claude-haiku-5-5`), subscription login (`"apiKeySource":"none"`).
- Method, base command, fixture line format and sanitizing: as in part 1 ("Base command", "Files", "Sanitizing") with
  the additions of parts 2 to 5. Fixtures of this part are `fixtures/claude-2.1.311/r8-*.ndjson`; the scripts are
  `drivers/claude-stream-json/r8_*.py` (README there says how to record again).
- Cost of the 101 recordings at list price: 0.1586 USD, the sum of each turn's own model requests
  (`model_usage_since_last_result`). `cost.py` prints 0.1681: it sums each session's last `total_cost_usd` per
  recording, and a resumed or forked session carries its earlier cost (sections 2 and 3). The runs in a throw-away HOME
  have no login and cost nothing.
- Everything below was observed in these recordings. Each answer says how many runs it rests on (n) and says no more
  than the recordings show. Statements marked "bundle" were read in the CLI's program file (`bgrep.py`), not recorded.
- The sections are numbered as the 12 questions this part was recorded for; the last two tables have them in the same
  order.

Differences from the part 5 method:
- `r5_lib.Procs` now matches the whole command text (`Procs("sleep 41")` no longer matches `sleep 410` or
  `sleep 41.5`), and every script uses it that way. Part 6 scripts give each scenario's `sleep` a duration no other
  script uses. Two runs that look for the same text must not run at the same time: at its end a run kills every new
  match, also the other run's. One recording of section 5 was hurt that way and is not kept (section 5).
- Plugins: section 1 installs a plugin of the scenario (`ping-plugin`, from a local marketplace folder) with the CLI's
  own `claude plugin` commands in a throw-away HOME. The sanitizer keeps that plugin (and its server
  `plugin:ping-plugin:ping-plugged`) in `init`, and removes every other plugin.
- New sanitizing: file names the CLI makes from a process id, a hash or the clock (`.claude/sessions/<pid>.<hash>.key`,
  `.claude/sessions/<pid>.json`, `.claude.json.backup.<time>`, `mcp-logs-<server>/<time>.jsonl`) are replaced. The six
  `r7-mcp-toggle-saved-*` fixtures of part 5 were sanitized again with that rule (`sanitize.py --refresh`): only their
  `files_changed` notes changed; nothing was recorded again.
- Sections 2 and 3 count, in the session's own transcript file under `~/.claude/projects/`, its lines and the lines
  that name each secret word, and the session files of its folder (`r8_lib.transcript`: counts only, the transcript
  itself is never copied).
- In quoted lines `"…":"…"` stands for keys left out; `# t=` lines are seconds since the process started. A quoted
  line of a subagent keeps `parent_tool_use_id` and `agent_id`.

## 1. Your own setup in a folder you have not trusted: the user settings that load project servers, and plugins

Question: in a throw-away HOME whose USER settings file says to load the project's servers, with no `--strict-mcp-config` and `--setting-sources user` or `user,local`: is the repo's `.mcp.json` server started? A plugin installed for the user that brings an MCP server: is it loaded, and can a file of the repo switch such a plugin on?

Fixtures: `r8-untrusted-mcp-<form>-<sources>-<label>`. The work folder's `.mcp.json` defines `ping-project` (one line in `started.txt` per start). Every run has a THROW-AWAY HOME (no login: its one message ends at once with `Not logged in`; the recordings show what was LOADED and STARTED). Flags of `plugin-repo-local-user-local-1`: `--permission-mode manual --setting-sources user,local --tools Bash`. The plugin `ping-plugin` comes from a local marketplace folder (`claude plugin marketplace add`, then `claude plugin install --scope user`, both with the throw-away HOME); its `.mcp.json` brings `ping-plugged` (one line in `started-plugin.txt` per start). For the `plugin-repo-*` forms the script then ran `claude plugin disable --scope user` and wrote `{"enabledPlugins":{"ping-plugin@codly-test-market":true}}` into the work folder's `.claude/settings.json` and `.claude/settings.local.json` (`plugin-repo`), only the local file (`plugin-repo-local`) or only `settings.json` (`plugin-repo-project`).

The set-up note and the `init` line of `r8-untrusted-mcp-plugin-repo-local-user-local-1`:

```json
{"setting_sources":"user,local","plugin_commands":[{"command":"claude plugin marketplace add <market folder>","exit_code":0,"first_line_of_its_output":"Adding marketplace…✔ Successfully added marketplace: codly-test-market (declared in user settings)"},{"command":"claude plugin install ping-plugin@codly-test-market --scope user","exit_code":0,"first_line_of_its_output":"Installing plugin \"ping-plugin@codly-test-market\"...✔ Successfully installed plugin: ping-plugin@codly-test-market (scope: user)"},{"command":"claude plugin disable ping-plugin@codly-test-market --scope user","exit_code":0,"first_line_of_its_output":"✔ Successfully disabled plugin: ping-plugin (scope: user)"}],"repo_files":{"settings.local.json":{"enabledPlugins":{"ping-plugin@codly-test-market":true}}},"user_settings_file":{"enabledPlugins":{"ping-plugin@codly-test-market":false}},"servers_started_by_the_setup":{"project":0,"plugin":0}}
# t=3.01
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"default","tools":["Bash","mcp__plugin_ping-plugin_ping-plugged__ping"],"mcp_servers":[{"name":"plugin:ping-plugin:ping-plugged","status":"connected","source":"plugin"}],"…":"…"}
```

The `init` line of `r8-untrusted-mcp-plugin-repo-local-user-1` (the same files, `--setting-sources user`):

```json
# t=3.00
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"default","tools":["Bash"],"mcp_servers":[],"…":"…"}
```

| form | `--setting-sources` | what the HOME and the repo have | recordings | `ping-project` started | `ping-plugged` started | scenario servers in `init` |
|---|---|---|---|---|---|---|
| `enable-all` | `user` | user settings: `"enableAllProjectMcpServers": true` | 2 | 0 / 0 | 0 / 0 | none |
| `enable-all` | `user,local` | user settings: `"enableAllProjectMcpServers": true` | 2 | 0 / 0 | 0 / 0 | none |
| `enabled-named` | `user` | user settings: `"enabledMcpjsonServers": ["ping-project"]` | 2 | 0 / 0 | 0 / 0 | none |
| `enabled-named` | `user,local` | user settings: `"enabledMcpjsonServers": ["ping-project"]` | 2 | 0 / 0 | 0 / 0 | none |
| `plugin` | `user` | the plugin installed and on for the user | 2 | 0 / 0 | 1 / 1 | `plugin:ping-plugin:ping-plugged (connected)` |
| `plugin` | `user,local` | the plugin installed and on for the user | 2 | 0 / 0 | 1 / 1 | `plugin:ping-plugin:ping-plugged (connected)` |
| `plugin-repo` | `user` | the plugin off for the user; both repo files turn it on | 2 | 0 / 0 | 0 / 0 | none |
| `plugin-repo` | `user,local` | the plugin off for the user; both repo files turn it on | 2 | 0 / 0 | 1 / 1 | `plugin:ping-plugin:ping-plugged (connected)` |
| `plugin-repo-local` | `user` | the plugin off for the user; the repo's `settings.local.json` turns it on | 2 | 0 / 0 | 0 / 0 | none |
| `plugin-repo-local` | `user,local` | the plugin off for the user; the repo's `settings.local.json` turns it on | 2 | 0 / 0 | 1 / 1 | `plugin:ping-plugin:ping-plugged (connected)` |
| `plugin-repo-project` | `user` | the plugin off for the user; the repo's `settings.json` turns it on | 2 | 0 / 0 | 0 / 0 | none |
| `plugin-repo-project` | `user,local` | the plugin off for the user; the repo's `settings.json` turns it on | 2 | 0 / 0 | 0 / 0 | none |

What the adapter can rely on:
- The repo's `.mcp.json` server was started in NONE of these 24 recordings (no start, no `init` entry, no tool): not
  with `"enableAllProjectMcpServers": true` in the user settings file, not with the server named in
  `enabledMcpjsonServers` there, under `user` or `user,local` (n=2 each). With parts 4 and 5 (16 recordings) that is
  every form recorded so far.
- A plugin the user installed and left on is loaded under `user` and under `user,local` (n=2 each): its server starts,
  `init.mcp_servers` names it `plugin:ping-plugin:ping-plugged` with `"source":"plugin"`, its tool is
  `mcp__plugin_ping-plugin_ping-plugged__ping`, and `init.plugins` lists the plugin (its `path` is the marketplace
  folder it came from).
- A plugin the user turned OFF (`enabledPlugins` false in the user settings file) is switched ON by the repo's
  `.claude/settings.local.json` under `user,local` (n=4: the local file alone, and both files). It stays off under
  `user` (n=6) and with only the repo's `.claude/settings.json` under `user,local` (n=2): the project file is not
  read without `project` among the sources.
- Not recorded: a plugin the user never installed (a repo file can also name a marketplace: `extraKnownMarketplaces`);
  the plugin's hooks and skills (only its MCP server was looked at); the same with the real login.

## 2. Rewind a conversation: `--resume <id> --resume-session-at <uuid>`

Question: does the next turn know only what came before that message? What do `init` and the stream say, which session id follows, is the old transcript kept, can the dropped part be resumed again? Where does a host get the uuid?

Fixtures: `r8-rewind-<label>-a` … `-d`, labels 1, 2. Process a: a new session with `--replay-user-messages`, three turns, each tells one secret word (`lantern`, `pebble`, `violet`) and asks for `noted`; each user message carries a uuid of the host. Process b: `--resume <a's session> --resume-session-at <uuid of the LAST assistant line of turn 1>`, one turn: "List every secret word I asked you to remember". Process c: a plain `--resume` of the same session, the same question. Process d: `--resume-session-at <uuid of the last assistant line of turn 3>`, the same question. Flags of b: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --resume sess-1 --resume-session-at uuid-1` (the uuid is the placeholder of b's own fixture).

Process a, turn 1: the host's line, the CLI's echo of it, the assistant line and the result (`uuid` and `user_message_uuid` kept):

```json
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Remember this secret word: lantern. Reply with the single word: noted"},"parent_tool_use_id":null,"uuid":"user-msg-1"}}
{"type":"user","message":{"role":"user","content":"Remember this secret word: lantern. Reply with the single word: noted"},"uuid":"user-msg-1","isReplay":true,"…":"…"}
{"type":"assistant","message":{"content":[{"type":"text","text":"noted"}],"…":"…"},"uuid":"uuid-11","…":"…"}
{"type":"result","subtype":"success","result":"noted","session_id":"sess-1","user_message_uuid":"user-msg-1","…":"…"}
```

Process b: the host's line, the `init` line, the answer and the result (`session_id` kept):

```json
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"List every secret word I asked you to remember in this conversation, in one line. If there is none, reply with the single word: none"},"parent_tool_use_id":null}}
{"type":"system","subtype":"init","session_id":"sess-1","model":"claude-haiku-5-5","…":"…"}
{"type":"assistant","message":{"content":[{"type":"text","text":"lantern"}],"…":"…"},"uuid":"uuid-7","…":"…"}
{"type":"result","subtype":"success","result":"lantern","session_id":"sess-1","user_message_uuid":null,"…":"…"}
```

Process d: the stderr line and the result (no `init` line came):

```json
{"_stderr":"No message found with message.uuid of: uuid-1"}
{"type":"result","subtype":"error_during_execution","is_error":true,"num_turns":0,"errors":["No message found with message.uuid of: uuid-1"],"session_id":"sess-1"}
```

| label | b: words named | c: words named | d: result | transcript lines after a / b / c / d | lines naming `pebble`, `violet` after a / after d |
|---|---|---|---|---|---|
| 1 | lantern | lantern | `error_during_execution` | 35 / 46 / 54 / 54 | 2, 3 / 2, 3 |
| 2 | lantern | lantern | `error_during_execution` | 35 / 46 / 54 / 54 | 2, 3 / 2, 3 |

What the adapter can rely on (n=2):
- The rewind works: after `--resume-session-at <uuid of turn 1's last assistant line>` the next turn knew only turn 1
  (`lantern`), not the words of turns 2 and 3.
- The session id stays the same (in `init` and in the `result`): a rewind is no new session. The stream says nothing
  about the rewind: the turn's lines are those of any resumed turn.
- The old transcript is kept: it is the same file (one session file in the folder), it grows with each process, and
  its lines that name the dropped words are still there.
- A plain `--resume` after the rewind goes on from the REWOUND branch (c knew only `lantern`).
- The dropped part cannot be resumed by its uuid once a rewound turn was added: `--resume-session-at <uuid of turn 3's
  last assistant line>` gave no `init`, one `result` `error_during_execution` with
  `errors:["No message found with message.uuid of: <uuid>"]` (the same words on stderr), and exit code 1.
- Where the host gets the uuid: the `uuid` key of each assistant line of the stream (the last one of a turn is the
  point to keep). With `--replay-user-messages` the CLI echoes each user line with the host's own `uuid` and
  `"isReplay":true`, and the turn's `result` has `user_message_uuid` = that uuid. Bundle: the help text of the hidden
  flag says `--resume-session-at` takes "any chain-entry UUID, typically the kept turn's last entry", and a second
  hidden flag `--resume-drops-turn <message id>` names the one turn the rewind means to drop, and the CLI then refuses
  it when the dropped part holds anything else (queued messages, task notifications, other turns; not recorded).
- Not recorded: a rewind to a user message's uuid; a rewind in a session with tool calls or helpers.

## 3. Fork a conversation: `--resume <id> --fork-session`

Question: the new session id, what the first `result` says about cost (`total_cost_usd`: the old session's or zero), and does the old session answer as before? Alone and with `--resume-session-at`.

Fixtures: `r8-fork-<label>-a` … `-d`, labels 1, 2. Process a: a new session, two turns (`lantern`, `pebble`). Process b: `--resume <a's session> --fork-session`; c: the same with `--resume-session-at <uuid of turn 1's last assistant line>`; d: a plain `--resume` of a's session. Each of b, c, d asks for every secret word. Flags of c: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --resume sess-1 --fork-session --resume-session-at uuid-1`.

The `init` line and the result of b (first label), then b's note:

```json
{"type":"system","subtype":"init","session_id":"sess-2","model":"claude-haiku-5-5","…":"…"}
{"type":"result","subtype":"success","result":"lantern, pebble","session_id":"sess-2","total_cost_usd":0.0008813}
{"asked":"b","ok":true,"words_named":["lantern","pebble"],"init_session_id_is_the_first_session":false,"result_session_id_is_the_first_session":false,"total_cost_usd":0.0008813,"first_sessions_last_total_cost_usd":0.00080338,"cost_of_this_turn":7.792e-05}
```

| label | a's last `total_cost_usd` | b: words, new session?, `total_cost_usd` = a + own turn | c: words, new session?, total = a + own turn | d: words, same session?, total = a + own turn | session files after a / b / c / d |
|---|---|---|---|---|---|
| 1 | 0.00080338 | lantern, pebble, new, 0.00088130 = 0.00080338 + 0.00007792 | lantern, new, 0.00094414 = 0.00080338 + 0.00014076 | lantern, pebble, the same, 0.00086420 = 0.00080338 + 0.00006082 | 1 / 2 / 3 / 3 |
| 2 | 0.00081913 | lantern, pebble, new, 0.00089780 = 0.00081913 + 0.00007867 | lantern, new, 0.00096064 = 0.00081913 + 0.00014151 | lantern, pebble, the same, 0.00088070 = 0.00081913 + 0.00006157 | 1 / 2 / 3 / 3 |

What the adapter can rely on (n=2):
- `--fork-session` gives a NEW session id: in `init`, in every line of the stream and in the `result` (one id per
  process). Its transcript is a new file in the same folder (one more session file per fork).
- The fork knows the old conversation (b named both words); with `--resume-session-at` it knows only up to that
  message (c named `lantern` only). So "try again from here" is `--resume <id> --fork-session --resume-session-at
  <uuid>`, and the old session stays as it was.
- COST: the fork's first `total_cost_usd` is the OLD session's total PLUS the fork's own turn (to the last digit), not
  zero. A host that sums `total_cost_usd` per session counts the old part twice; the turn's own cost is in
  `model_usage_since_last_result` (`costUSD`).
- The old session answers as before after the forks: a plain `--resume` (d) keeps its id and knows both words; its
  total goes on from a's.

## 4. Claude Code's own file rewind (`rewind_files`)

Question: is there a flag, a setting or a variable that turns it on in stream-json mode? With it on: the answer to a dry run and to a real one, and which files change.

How it was looked for: `claude --help` lists no switch for it, only `--rewind-files <user-message-id>` ("Restore files to state at the specified user message and exit (requires --resume)"). Bundle: the gate of `rewind_files` (`bgrep.py "File rewinding is not enabled"`, then the function it calls) is: in a run that is not interactive, ONLY the variable `CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING` turns it on (and `CLAUDE_CODE_DISABLE_FILE_CHECKPOINTING` off); the setting `fileCheckpointingEnabled` ("Snapshot files before edits so /rewind can restore them") counts only in the terminal UI. The recordings test the three ways.

Fixtures: `r8-file-rewind-env-*` (the variable `CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING=1`), `-setting-*` (`--settings '{"fileCheckpointingEnabled": true}'`), `-off-*` (neither). Flags of `env-1`: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Read,Edit`. Turn 1 (user message uuid U1) edits `hello.txt` from `hello` to `hello one`, turn 2 (U2) to `hello two`. Then `rewind_files`: U2 dry run, U2, U1 dry run, U1, a uuid that no message has (dry run).

The five requests and answers of `r8-file-rewind-env-1`:

```json
{"type":"control_request","request_id":"host-1","request":{"subtype":"rewind_files","user_message_id":"user-msg-2","dry_run":true}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"canRewind":true,"filesChanged":["/tmp/work/r8-file-rewind-env-1/hello.txt"],"insertions":1,"deletions":1}}}
{"type":"control_request","request_id":"host-2","request":{"subtype":"rewind_files","user_message_id":"user-msg-2","dry_run":false}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-2","response":{"canRewind":true,"skippedLinks":0}}}
{"type":"control_request","request_id":"host-3","request":{"subtype":"rewind_files","user_message_id":"user-msg-1","dry_run":true}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-3","response":{"canRewind":true,"filesChanged":["/tmp/work/r8-file-rewind-env-1/hello.txt"],"insertions":1,"deletions":1}}}
{"type":"control_request","request_id":"host-4","request":{"subtype":"rewind_files","user_message_id":"user-msg-1","dry_run":false}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-4","response":{"canRewind":true,"skippedLinks":0}}}
{"type":"control_request","request_id":"host-5","request":{"subtype":"rewind_files","user_message_id":"uuid-121","dry_run":true}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-5","response":{"canRewind":false,"error":"No file checkpoint found for this message."}}}
```

The same requests of `r8-file-rewind-setting-1` (no variable), the first two:

```json
{"type":"control_request","request_id":"host-1","request":{"subtype":"rewind_files","user_message_id":"user-msg-2","dry_run":true}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"canRewind":false,"error":"File rewinding is not enabled."}}}
{"type":"control_request","request_id":"host-2","request":{"subtype":"rewind_files","user_message_id":"user-msg-2","dry_run":false}}
{"type":"control_response","response":{"subtype":"error","request_id":"host-2","error":"File rewinding is not enabled."}}
```

| recording | answers (U2 dry, U2, U1 dry, U1, unknown dry) | `hello.txt` after each |
|---|---|---|
| `r8-file-rewind-env-1` | canRewind true, filesChanged 1; canRewind true; canRewind true, filesChanged 1; canRewind true; canRewind false, No file checkpoint found for this message. | `hello two`, `hello one`, `hello one`, `hello`, `hello` |
| `r8-file-rewind-env-2` | canRewind true, filesChanged 1; canRewind true; canRewind true, filesChanged 1; canRewind true; canRewind false, No file checkpoint found for this message. | `hello two`, `hello one`, `hello one`, `hello`, `hello` |
| `r8-file-rewind-setting-1` | canRewind false, File rewinding is not enabled.; error: File rewinding is not enabled.; canRewind false, File rewinding is not enabled.; error: File rewinding is not enabled.; canRewind false, File rewinding is not enabled. | `hello two`, `hello two`, `hello two`, `hello two`, `hello two` |
| `r8-file-rewind-setting-2` | canRewind false, File rewinding is not enabled.; error: File rewinding is not enabled.; canRewind false, File rewinding is not enabled.; error: File rewinding is not enabled.; canRewind false, File rewinding is not enabled. | `hello two`, `hello two`, `hello two`, `hello two`, `hello two` |
| `r8-file-rewind-off-1` | canRewind false, File rewinding is not enabled.; error: File rewinding is not enabled.; canRewind false, File rewinding is not enabled.; error: File rewinding is not enabled.; canRewind false, File rewinding is not enabled. | `hello two`, `hello two`, `hello two`, `hello two`, `hello two` |
| `r8-file-rewind-off-2` | canRewind false, File rewinding is not enabled.; error: File rewinding is not enabled.; canRewind false, File rewinding is not enabled.; error: File rewinding is not enabled.; canRewind false, File rewinding is not enabled. | `hello two`, `hello two`, `hello two`, `hello two`, `hello two` |

After the exit of each `env` run: `claude -p --strict-mcp-config --setting-sources local --resume <session>
--rewind-files <U2>` with the same variable answered
`Files rewound to state at message <U2>` (exit code 0) and `hello.txt` was `hello one` again (n=2).

What the adapter can rely on (n=2 per way):
- Only the VARIABLE turns it on: `CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING=1` in the CLI's environment. The setting
  `fileCheckpointingEnabled` in `--settings` does not, nor nothing (the same answers).
- When it is off, a dry run answers success with `{"canRewind":false,"error":"File rewinding is not enabled."}`, and a
  real one answers a control ERROR with the same words. No file changes.
- When it is on, `rewind_files {"user_message_id": U}` puts the files back to their state BEFORE the turn of U (U2 ->
  `hello one`, U1 -> `hello`). `user_message_id` is the uuid the host put on its user message. A dry run answers
  `{"canRewind":true,"filesChanged":[<absolute paths>],"insertions":N,"deletions":N}` and changes nothing; the real
  one answers `{"canRewind":true,"skippedLinks":0}`. A uuid no message has: `{"canRewind":false,"error":"No file
  checkpoint found for this message."}`.
- So Codly's restore could be the CLI's own: start the agent with the variable, send a uuid with each user message,
  and `rewind_files` that uuid. Not recorded: a file the agent wrote with Bash (the CLI snapshots files before ITS
  edits: Edit here), a file made new in the turn, helpers' edits, and whether the conversation is told.

## 5. `stop_task` in the last moment of a turn; `stop_task` on a background command while a turn runs

Question: `stop_task` on a background helper during the turn's LAST model request: does a turn start by itself after the `result`? And `stop_task` on a background COMMAND while a turn runs: the lines, and whether a turn follows.

Fixtures: `r8-stop-helper-last-request-*` (turn 1 starts a helper in the background, its Bash call `echo begin; sleep 211; echo end`; turn 2 asks for a poem with no tool, one model request; `stop_task` with the helper's id at turn 2's first streamed text delta), `r8-stop-command-last-request-*` (turn 1 starts `echo bg; sleep 227` in the background; the same turn 2 and moment), `r8-stop-command-mid-turn-*` (turn 1 starts `echo bg; sleep 223` in the background; turn 2 runs `echo go; sleep 22.5` in the foreground; `stop_task` with the command's id 2 s into that call). Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash,Agent`. The host reads 25 s after turn 2's result.

From the note before `stop_task` to the end (`r8-stop-helper-last-request-1`):

```json
# t=4.49
{"_meta":{"before_stop_task":{"when":"at the first streamed text delta of turn 2","list":[{"task_id":"tid-1","run_id":"run-1","task_type":"local_agent","subagent_type":"general-purpose","description":"Run sleep echo test"}],"a_turn_runs":true,"bg_sleep_alive":true,"fg_sleep_alive":false}},"_t":4.491}
# t=4.49
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"stop_task","task_id":"tid-1"}},"_t":4.491}
# t=4.49
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"killed","end_time":1795483867003}}
# t=4.49
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r8-stop-helper-last-request-1/sess-1/tasks/tid-1.output","summary":"Run sleep echo test"}
# t=4.49
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=4.49
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{}}}
# t=4.49
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-2"}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=4.49
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=7.10
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"The Sea Keeps Its Own Counsel\n\nI came down to the water at the hour\nwhen the sky forgets which color it should wear,\nand the sea, unhurried, folded its long shawl\nof silver over stones that had waited…"}],"…":"…"},"…":"…"}
# t=7.10
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"The Sea Keeps Its Own Counsel\n\nI came down to the water at the hour\nwhen the sky forgets which color it should wear,\nand the sea, unhurried, folded its long shawl\nof silver over stones that had waited…","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":1,"…":"…"}
# t=7.10
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Task","Bash"],"mcp_servers":[],"…":"…"}
# t=7.92
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"The background subagent running `echo begin; sleep 211; echo end` was stopped before it finished, so I have no output to report. I didn't read its transcript file. If you want the result, I can rerun …"}],"…":"…"},"…":"…"}
# t=8.17
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"The background subagent running `echo begin; sleep 211; echo end` was stopped before it finished, so I have no output to report. I didn't read its transcript file. If you want the result, I can rerun …","stop_reason":"end_turn","terminal_reason":"completed","origin":{"kind":"task-notification","producer":"session-task","runId":"run-1"},"queued_turn_count":0,"result_index":2,"…":"…"}
# t=32.41
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":32.406}
# t=33.10
{"_meta":{"exit_code":0},"_t":33.102}
# t=33.10
{"_meta":{"transcript":{"transcript_found":true,"enqueued":1,"attached_to_a_running_turn":0,"removed":0,"dequeued":0}},"_t":33.105}
```

The same moment with a background command (`r8-stop-command-last-request-1`):

```json
# t=3.13
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"stop_task","task_id":"tid-1"}},"_t":3.134}
# t=3.13
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"killed","end_time":1795483911543}}
# t=3.13
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r8-stop-command-last-request-1/sess-1/tasks/tid-1.output","summary":"Run echo and sleep in background"}
# t=3.13
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=3.13
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{}}}
# t=6.13
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"Salt-wind walks the shingle every dawn,\nand the sea unfolds its slow, grey hands,\ngathering the stones it has not yet drawn\nback into the deep where the old tide stands.\n\nIt keeps no ledger of the shi…"}],"…":"…"},"…":"…"}
# t=6.13
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"Salt-wind walks the shingle every dawn,\nand the sea unfolds its slow, grey hands,\ngathering the stones it has not yet drawn\nback into the deep where the old tide stands.\n\nIt keeps no ledger of the shi…","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":1,"…":"…"}
# t=31.26
{"_meta":{"after_stop_task":{"lines":["system/task_updated \"killed\"","system/task_notification \"stopped\"","…(4 more)"],"results_since_the_request":1,"result_origins":[null],"list":[],"bg_sleep_alive":false,"fg_sleep_alive":false},"seconds_from_stop_task":{"answer":0.002,"task_notification":0.002,"result":3.054,"init_after_it":null}},"_t":31.263}
# t=31.26
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":31.263}
# t=31.96
{"_meta":{"exit_code":0},"_t":31.957}
# t=31.96
{"_meta":{"transcript":{"transcript_found":true,"enqueued":1,"attached_to_a_running_turn":0,"removed":0,"dequeued":0}},"_t":31.959}
```

| recording | lines from `stop_task` on | results after it (origin of the 2nd) | `init` after it (s) | queue (enqueued / handed to a running turn / removed) |
|---|---|---|---|---|
| `r8-stop-helper-last-request-1` | `system/task_updated "killed"`, `system/task_notification "stopped"`, `system/background_tasks_changed 0 listed`, `control_response success`, `user (helper)`, `user (helper)`, `assistant`, `result/success`, `system/init`, `assistant`, `assistant`, `result/success` | 2 (task-notification) | 2.73 | 1 / 0 / 0 |
| `r8-stop-helper-last-request-2` | `system/task_updated "killed"`, `system/task_notification "stopped"`, `system/background_tasks_changed 0 listed`, `control_response success`, `user (helper)`, `user (helper)`, `assistant`, `result/success`, `system/init`, `assistant`, `assistant`, `result/success` | 2 (task-notification) | 3.37 | 1 / 0 / 0 |
| `r8-stop-command-last-request-1` | `system/task_updated "killed"`, `system/task_notification "stopped"`, `system/background_tasks_changed 0 listed`, `control_response success`, `assistant`, `result/success` | 1 (-) | none | 1 / 0 / 0 |
| `r8-stop-command-last-request-2` | `system/task_updated "killed"`, `system/task_notification "stopped"`, `system/background_tasks_changed 0 listed`, `control_response success`, `assistant`, `result/success` | 1 (-) | none | 1 / 0 / 0 |
| `r8-stop-command-mid-turn-1` | `system/task_updated "killed"`, `system/task_notification "stopped"`, `system/background_tasks_changed 0 listed`, `control_response success`, `system/task_started`, `system/task_notification "completed"`, `user`, `assistant`, `assistant`, `result/success` | 1 (-) | none | 1 / 1 / 1 |
| `r8-stop-command-mid-turn-3` | `system/task_updated "killed"`, `system/task_notification "stopped"`, `system/background_tasks_changed 0 listed`, `control_response success`, `system/task_started`, `system/task_notification "completed"`, `user`, `assistant`, `assistant`, `result/success` | 1 (-) | none | 1 / 1 / 1 |
| `r8-stop-command-mid-turn-4` | `system/task_updated "killed"`, `system/task_notification "stopped"`, `system/background_tasks_changed 0 listed`, `control_response success`, `system/task_started`, `system/task_notification "completed"`, `user`, `assistant`, `assistant`, `result/success` | 1 (-) | none | 1 / 1 / 1 |

What the adapter can rely on:
- A helper stopped during the turn's LAST model request (n=2): the stop itself as always (`killed`, `stopped`, the
  list `[]`, the answer `{}`, the helper's two `user` lines), the running turn ends normally, and then the CLI STARTS A
  TURN BY ITSELF: an `init` 2.73, 3.37 s after the request, and a second `result` with
  `"origin":{"kind":"task-notification",…}`. The transcript shows the notification put in the queue and handed to no
  running turn. So part 5's "partly" is settled: a stop with no later model request of the running turn is told in a
  turn of its own after the result; a stop while a tool call of the turn runs is told inside the turn (part 5,
  section 11).
- A background COMMAND stopped while a turn runs: the same four lines (`killed`, `stopped`, the list `[]`, the answer
  `{}`), and NO turn by itself, both during the turn's last model request (n=2) and during its tool call (n=3). Its
  notification is put in the queue too: during a tool call it was handed to the turn's next model request (and taken
  out of the queue); during the last request it stayed in the queue until the exit (enqueued, not handed, not
  removed) and started nothing in 25 s. In every `command-mid-turn` run the turn's own foreground call ran to its end
  (`completed`).
- A fourth `command-mid-turn` recording was not kept: the driver itself ended its foreground `sleep 22.5` (exit code
  144). A `command-last-request` run that ran at the same time killed, at its end, every new process that matched
  `sleep 22.5`, its own or not. The script now looks for that `sleep` only in the `command-mid-turn` variant.

## 6. After a budget stop: the call that runs on, and a helper with requests left

Question: with a long Bash call still running when `error_max_budget_usd` ends the turn: does an `interrupt` end that call (stdin still open)? Does `stop_task` with the call's task id? And a helper that still has model requests to make when the limit is passed: does it go on, what does it cost?

Fixtures: `r8-after-budget-<variant>-<label>-calib`, `-t1` …. As in part 5 (section 10): run `calib` has no budget; each try gets `--max-budget-usd` = the cost when the Bash call began + a share of the helper's cost, both from the run before; the script stops at the first try that stops as the scenario needs. Tries that passed: `r8-after-budget-interrupt-1-t2`, `r8-after-budget-interrupt-2-t2`, `r8-after-budget-stop-task-1-t1`, `r8-after-budget-stop-task-2-t2`, `r8-after-budget-helper-goes-on-1-t3`, `r8-after-budget-helper-goes-on-2-t2`. Flags of `r8-after-budget-interrupt-1-t2`: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash,Agent --max-budget-usd 0.000785`.

- `interrupt`: a helper writes a story of about 600 words with no tool; the main thread runs `echo go; sleep 53.5`. 2 s after the budget stop the host sends `interrupt` and reads 5 s, then closes stdin.
- `stop-task`: the same (`sleep 57.5`); the host sends `stop_task` with the task id of the call (its `task_started` line).
- `helper-goes-on`: the helper runs four Bash calls one after the other (`echo h1` … `echo h4`); the main thread runs `echo go; sleep 47.5`; the host reads 40 s after the stop.

`r8-after-budget-interrupt-1-t2`, from the try's note to the end:

```json
# t=11.84
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"interrupt"}},"_t":11.841}
# t=11.84
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"still_queued":[]}}}
# t=16.90
{"_meta":{"after_the_request":{"lines":["control_response success"],"list":[],"sleep_alive":true},"seconds_from_the_request":{"answer":0.002,"call_notification":null}},"_t":16.899}
# t=16.90
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":16.899}
# t=16.90
{"type":"system","subtype":"task_updated","task_id":"tid-2","run_id":"run-2","patch":{"status":"killed","end_time":1795484374277}}
# t=16.90
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r8-after-budget-interrupt-1/sess-1/tasks/tid-2.output","summary":"Print go and sleep 53.5 seconds"}
# t=17.66
{"_meta":{"exit_code":1},"_t":17.658}
# t=17.71
{"_meta":{"after_the_exit":{"lines_after_stdin_was_closed":["system/task_updated \"killed\"","system/task_notification \"stopped\""],"sleep_alive":false}},"_t":17.714}
```

`r8-after-budget-stop-task-1-t1`, from the try's note to the end:

```json
# t=11.51
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Task","Bash"],"mcp_servers":[],"…":"…"}
# t=11.51
{"type":"result","subtype":"error_max_budget_usd","is_error":true,"num_turns":1,"stop_reason":null,"terminal_reason":"budget_exhausted","errors":["Reached maximum budget ($0.001448)"],"origin":{"kind":"task-notification","producer":"session-task","runId":"run-1"},"queued_turn_count":0,"result_index":1,"…":"…"}
# t=13.51
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"stop_task","task_id":"tid-2"}},"_t":13.513}
# t=13.51
{"type":"system","subtype":"task_updated","task_id":"tid-2","run_id":"run-2","patch":{"status":"killed","end_time":1795484315820}}
# t=13.51
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r8-after-budget-stop-task-1/sess-1/tasks/tid-2.output","summary":"Echo go then sleep 57.5 seconds"}
# t=13.51
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{}}}
# t=18.56
{"_meta":{"after_the_request":{"lines":["system/task_updated \"killed\"","system/task_notification \"stopped\"","control_response success"],"list":[],"sleep_alive":false},"seconds_from_the_request":{"answer":0.004,"call_notification":0.004}},"_t":18.559}
# t=18.56
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":18.559}
# t=19.31
{"_meta":{"exit_code":1},"_t":19.31}
# t=19.35
{"_meta":{"after_the_exit":{"lines_after_stdin_was_closed":[],"sleep_alive":false}},"_t":19.354}
```

| try | budget (USD) | `result` (s) | the call's `sleep` alive at the result | the request | lines in the 5 s after it | `sleep` alive 5 s after | after stdin close |
|---|---|---|---|---|---|---|---|
| `r8-after-budget-interrupt-1-t2` | 0.000785 | 9.72 | yes | `interrupt` | `control_response success` | yes | `system/task_updated "killed"`, `system/task_notification "stopped"` |
| `r8-after-budget-interrupt-2-t2` | 0.000877 | 13.83 | yes | `interrupt` | `control_response success` | yes | `system/task_updated "killed"`, `system/task_notification "stopped"` |
| `r8-after-budget-stop-task-1-t1` | 0.001448 | 11.39 | yes | `stop_task` | `system/task_updated "killed"`, `system/task_notification "stopped"`, `control_response success` | no | nothing |
| `r8-after-budget-stop-task-2-t2` | 0.000811 | 11.87 | yes | `stop_task` | `system/task_updated "killed"`, `system/task_notification "stopped"`, `control_response success` | no | nothing |

`r8-after-budget-helper-goes-on-1-t3`: the helper's lines, the results and stderr:

```json
# t=2.80
{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"tid-1","run_id":"run-1","task_type":"local_agent","subagent_type":"general-purpose","description":"Run four sequential echo commands"}]}
# t=2.80
{"type":"system","subtype":"task_started","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","description":"Run four sequential echo commands","subagent_type":"general-purpose","is_backgrounded":true,"spawn_depth":1,"task_type":"local_agent","prompt":"Using the Bash tool, run these four commands one after another, as four separate Bash tool calls. Wait for each to return before making the next call: 1) `echo h1` 2) `echo h2` 3) `echo h4` is wrong; the order is: `echo h1`, then `echo h2`, then `echo h3`, then `echo h4`. Do not combine them into one command. When all …"}
# t=3.88
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-3","name":"Bash","input":{"command":"echo h1","description":"Print h1"}}],"…":"…"},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=5.22
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-4","name":"Bash","input":{"command":"echo h2","description":"Print h2"}}],"…":"…"},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=6.86
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"I ran the first two commands but stopped before the last two because the budget for this run is used up.\n\n- `echo h1` returned `h1`\n- `echo h2` returned `h2`\n- `echo h3` was not run\n- `echo h4` was not run\n\nThe budget was already exceeded after the second call, so I did not start the remaining two. They can be run in a…"}],"…":"…"},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=6.86
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"completed","end_time":1795484442878}}
# t=6.86
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"completed","output_file":"/tmp/claude-tmp/-tmp-work-r8-after-budget-helper-goes-on-1/sess-1/tasks/tid-1.output","summary":"I ran the first two commands but stopped before the last two because the budget for this run is used up.\n\n- `echo h1` returned `h1`\n- `echo h2` returned `h2`\n- `echo h3` was not run\n- `echo h4` was not run\n\nThe budget was already exceeded after the second call, so I did not start the remaining two. They can be run in a…","usage":"…"}
# t=6.86
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=7.52
{"type":"system","subtype":"task_started","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","description":"Print go and sleep 47.5 seconds","is_backgrounded":false,"task_type":"local_bash"}
# t=7.52
{"type":"result","subtype":"error_max_budget_usd","is_error":true,"num_turns":2,"stop_reason":"tool_use","terminal_reason":"budget_exhausted","errors":["Reached maximum budget ($0.000678)"],"queued_turn_count":0,"result_index":0,"…":"…"}
# t=7.52
{"type":"result","subtype":"error_max_budget_usd","is_error":true,"num_turns":1,"stop_reason":null,"terminal_reason":"budget_exhausted","errors":["Reached maximum budget ($0.000678)"],"origin":{"kind":"task-notification","producer":"session-task","runId":"run-1"},"queued_turn_count":0,"result_index":1,"…":"…"}
# t=47.66
{"_meta":{"forty_s_after_the_stop":{"lines":[],"helper_bash_calls_after_the_stop":[],"task_notification_statuses":[],"results":[],"list":[],"sleep_alive":true}},"_t":47.659}
# t=47.66
{"type":"system","subtype":"task_updated","task_id":"tid-2","run_id":"run-2","patch":{"status":"killed","end_time":1795484483654}}
# t=47.66
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r8-after-budget-helper-goes-on-1/sess-1/tasks/tid-2.output","summary":"Print go and sleep 47.5 seconds"}
# t=48.45
{"_meta":{"after_the_exit":{"lines_after_stdin_was_closed":["system/task_updated \"killed\"","system/task_notification \"stopped\""],"sleep_alive":false}},"_t":48.446}
```


`r8-after-budget-helper-goes-on-2-t2`: the helper's lines, the results and stderr:

```json
# t=2.31
{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"tid-1","run_id":"run-1","task_type":"local_agent","subagent_type":"general-purpose","description":"Run four echo commands sequentially"}]}
# t=2.31
{"type":"system","subtype":"task_started","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","description":"Run four echo commands sequentially","subagent_type":"general-purpose","is_backgrounded":true,"spawn_depth":1,"task_type":"local_agent","prompt":"Run these four Bash commands one after another, as four separate Bash tool calls, each only after the previous one has returned: `echo h1`, `echo h2`, `echo h3`, `echo h4`. Then report their outputs."}
# t=3.15
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-3","name":"Bash","input":{"command":"echo h1","description":"Print h1"}}],"…":"…"},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=4.72
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-4","name":"Bash","input":{"command":"echo h2","description":"Print h2"}}],"…":"…"},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=6.89
{"type":"system","subtype":"task_started","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","description":"Print go and sleep 47.5 seconds","is_backgrounded":false,"task_type":"local_bash"}
# t=6.89
{"type":"result","subtype":"error_max_budget_usd","is_error":true,"num_turns":2,"stop_reason":"tool_use","terminal_reason":"budget_exhausted","errors":["Reached maximum budget ($0.000701)"],"queued_turn_count":0,"result_index":0,"…":"…"}
# t=7.01
{"_stderr":"Budget limit reached ($0.00 of $0.000701); stopping background agents.","_t":7.013}
# t=7.01
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"killed","end_time":1795484641293}}
# t=7.01
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r8-after-budget-helper-goes-on-2/sess-1/tasks/tid-1.output","summary":"Run four echo commands sequentially"}
# t=7.01
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=7.01
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=47.13
{"_meta":{"forty_s_after_the_stop":{"lines":[],"helper_bash_calls_after_the_stop":[],"task_notification_statuses":[],"results":[],"list":[],"sleep_alive":true}},"_t":47.132}
# t=47.13
{"type":"system","subtype":"task_updated","task_id":"tid-2","run_id":"run-2","patch":{"status":"killed","end_time":1795484681417}}
# t=47.13
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r8-after-budget-helper-goes-on-2/sess-1/tasks/tid-2.output","summary":"Print go and sleep 47.5 seconds"}
# t=47.87
{"_meta":{"after_the_exit":{"lines_after_stdin_was_closed":["system/task_updated \"killed\"","system/task_notification \"stopped\""],"sleep_alive":false}},"_t":47.865}
```

| try | budget (USD) | the helper's Bash calls before the stop | requests it began after its 2nd Bash result | from that result to the turn's `result` (s) | the helper's end | stderr | results (subtype) | `total_cost_usd` at the end (over the budget) | lines in the 40 s after the `result` |
|---|---|---|---|---|---|---|---|---|---|
| `r8-after-budget-helper-goes-on-1-t3` | 0.000678 | `echo h1`, `echo h2` | 1 | 2.30 | `completed` | none | `error_max_budget_usd`, `error_max_budget_usd` | 0.000987 (+45%) | none |
| `r8-after-budget-helper-goes-on-2-t2` | 0.000701 | `echo h1`, `echo h2` | 1 | 2.18 | `stopped` | `Budget limit reached ($0.00 of $0.000701); stopping background agents.` | `error_max_budget_usd` | 0.000837 (+19%) | none |

What the adapter can rely on (n=2 each, unless it says otherwise):
- An `interrupt` after the budget stop does NOT end the call that runs on: the answer `{"still_queued":[]}` and nothing
  else; the `sleep` was still alive 5 s later. Only the close of stdin ended it (`killed`, `stopped`), as in part 5.
- `stop_task` with the call's task id (the `task_id` of its `task_started` line, which a foreground call gets after
  about 3 s) ends it at once: `task_updated` `killed`, `task_notification` `stopped`, the answer `{}`; the `sleep` was
  gone. So after a budget stop the host can end such a call without ending the process: `stop_task` for every
  foreground call that has a `task_started` and no end yet.
- A helper that still had calls to make DID GO ON for one model request after the limit was passed (n=2): it had
  made two of its four Bash calls; the turn's `result` came 2.30 / 2.18 s after the second one's result, and in that time the
  helper began a third model request. That the helper's second request is the one that passed the budget is inferred
  from the token counts of its requests, not said by the CLI. In `r8-after-budget-helper-goes-on-1-t3` the third request came back whole and was paid,
  its text says `The budget was already exceeded after the second call`, and the helper ended `completed` 0.66 s before the `result`. In `r8-after-budget-helper-goes-on-2-t2` the CLI STOPPED the helper at
  the turn's stop (stderr `Budget limit reached ($0.00 of $0.000701); stopping background agents.`, then `task_updated` `killed`, `task_notification` `stopped`, the helper's line
  `[Request interrupted by user]`). So "the CLI stops a helper that still works at the budget stop" rests on n=1.
- After the `result` no line came in 40 s (n=2): no model request, no Bash call, no other `result`.
- `--max-budget-usd` is not a hard cap: the total ended 19% and 45% above the budget, by the requests that were under way when
  the limit passed.
- So `run_limit_turn_end` (ending the run after a budget stop) was not needed to stop spending AFTER the `result` in
  these runs (nothing came in 40 s; a helper that still worked was stopped by the CLI, n=1). It is needed for the call
  that runs on, unless the host sends `stop_task` for it. Any cap the host shows must allow for the overshoot.

## 7. A foreground Bash call that becomes a task and FAILS, and one cut by an `interrupt`

Question: the status word and the keys of its `task_notification`.

Fixtures: `r8-bash-task-fails-1`, `r8-bash-task-fails-2`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash`. Turn 1: `echo q7; sleep 4; exit 3` in the foreground. Turn 2: `echo q7b; sleep 37` in the foreground; 1 s after its `task_started` and its `sleep`, the host sends `interrupt`.

The task lines and the tool results of both turns (`r8-bash-task-fails-1`):

```json
# t=5.07
{"type":"system","subtype":"task_started","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","description":"Echo q7, sleep 4 seconds, then exit with code 3","is_backgrounded":false,"task_type":"local_bash"}
# t=6.09
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"failed","output_file":"","summary":"Echo q7, sleep 4 seconds, then exit with code 3"}
# t=6.09
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"Exit code 3\nq7","is_error":true,"tool_use_id":"toolu-1"}]},"tool_use_result":"Error: Exit code 3\nq7","tool_result_meta":[{"id":"toolu-1","permission_decision":{"decision":"accept","source":"user_temporary"}}],"…":"…"}
# t=7.08
{"type":"result","subtype":"success","is_error":false,"num_turns":2,"result":"done","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":0,"…":"…"}
# t=11.20
{"type":"system","subtype":"task_started","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","description":"Echo q7b then sleep 37 seconds","is_backgrounded":false,"task_type":"local_bash"}
# t=12.24
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"interrupt"}},"_t":12.237}
# t=12.24
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","status":"stopped","output_file":"","summary":"Echo q7b then sleep 37 seconds"}
# t=12.24
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-2"}]},"tool_use_result":"User rejected tool use","tool_result_meta":[{"id":"toolu-2","non_execution_kind":"user-rejected"}],"…":"…"}
# t=12.24
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]},"…":"…"}
# t=12.24
{"type":"result","subtype":"error_during_execution","is_error":true,"num_turns":3,"stop_reason":"tool_use","terminal_reason":"aborted_tools","errors":["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=tool_use"],"queued_turn_count":0,"result_index":1,"…":"…"}
```


What the adapter can rely on (n=2 each):
- A call that ends by itself with an exit code other than 0: ONE `task_notification` with `"status":"failed"`,
  `"output_file":""`, the call's `tool_use_id` and its description in `summary`, just before the tool result
  (`is_error:true`, `Exit code 3` and the output). No `task_updated`, no list line.
- A call cut by an `interrupt`: ONE `task_notification` with `"status":"stopped"` and the same keys, right after the
  interrupt's answer, then the rejected tool result, `[Request interrupted by user for tool use]` and the turn's
  `result` `error_during_execution`. The `sleep` was gone.
- The keys are always `task_id`, `run_id`, `tool_use_id`, `status`, `output_file`, `summary` (and `uuid`, `session_id`).
  The status words of such a line seen so far: `completed`, `failed`, `stopped`. Each belongs to a Bash call (its
  `tool_use_id`): it must end no helper row and no background row.

## 8. An MCP switch while a turn runs; a server that cannot start, turned off and on in a NEW process

Question: `mcp_toggle` off and on during a turn that is about to call that server's tool. And a server that cannot start at the process's start, then off and on in a new process (the saved switch).

NOT RECORDED: the switch while a turn runs. A switch is saved in `~/.claude.json` (part 5, section 1), so the rules of these recordings make every run that switches a server use a throw-away HOME, and such a HOME has no login: no turn can run there. A run with the real HOME would write the user's own `~/.claude.json`. Bundle, not recorded: the CLI also reads `CLAUDE_CONFIG_DIR` for its config folder; a throw-away config folder with the real login could make this recordable, but that was not tried (it touches how the login is found).

Fixtures: `r8-mcp-cannot-start-saved-<label>-a`, `-b`, `-c`, labels 1, 2. A THROW-AWAY HOME (no login, no model request). `ping-flaky` (exits at once with code 1 while the file `switch.on` is missing; one line in `started.txt` per start) comes with `--mcp-config`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --mcp-config /tmp/work/r8-mcp-cannot-start-saved-1.json`. Process a: `switch.on` missing; `mcp_status`, `mcp_toggle` off, `mcp_status`. Process b, a new process in the same folder: `mcp_status`, `mcp_toggle` on (still missing), `mcp_status`, then `switch.on` is made, `mcp_reconnect`, `mcp_status`. Process c, a new process (`switch.on` there): `mcp_status`.

The requests and answers of b, first label:

```json
{"type":"control_request","request_id":"host-1","request":{"subtype":"mcp_status"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"mcpServers":[{"name":"ping-flaky","status":"disabled","config":{"type":"stdio","command":"python3","args":["mcp_ping_server.py","started.txt","need=switch.on"]},"transport":"stdio","scope":"dynamic","source":"dynamic"}]}}}
{"type":"control_request","request_id":"host-2","request":{"subtype":"mcp_toggle","serverName":"ping-flaky","enabled":true}}
{"type":"control_response","response":{"subtype":"error","request_id":"host-2","error":"Connection closed"}}
{"type":"control_request","request_id":"host-3","request":{"subtype":"mcp_status"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-3","response":{"mcpServers":[{"name":"ping-flaky","status":"failed","error":"Connection closed","config":{"type":"stdio","command":"python3","args":["mcp_ping_server.py","started.txt","need=switch.on"]},"transport":"stdio","scope":"dynamic","source":"dynamic"}]}}}
{"type":"control_request","request_id":"host-4","request":{"subtype":"mcp_reconnect","serverName":"ping-flaky"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-4"}}
```

| process, step | answer | `ping-flaky` after it | saved `disabledMcpServers` | starts so far |
|---|---|---|---|---|
| a, start + 3 s |  | failed (`Connection closed`) | `null` | 2 |
| a, mcp_toggle off | `success` | disabled | `["ping-flaky"]` | 2 |
| b, start + 3 s |  | disabled | `["ping-flaky"]` | 2 |
| b, mcp_toggle on while the program exits at once | `error: Connection closed` | failed (`Connection closed`) | `[]` | 4 |
| b, mcp_reconnect after switch.on was made | `success` | connected | `[]` | 5 |
| c, start + 3 s |  | connected | `[]` | 6 |

What the adapter can rely on (n=2, the two labels agree step for step):
- A server that cannot start at the process start is `failed` (`Connection closed`); the CLI started it twice
  (two lines in `started.txt`). `mcp_toggle` off makes it `disabled` and saves the switch.
- A NEW process in the folder starts it `disabled` and does not start it (no new line).
- `mcp_toggle` on there while it still cannot start: a control ERROR `Connection closed`, status `failed` with the same
  words, and the saved switch is ON again (`[]`), as for a failed "Turn on" in the same process (part 5, section 9).
- Once the cause is gone, `mcp_reconnect` answers success (`connected`), and the next new process starts it
  `connected`.

## 9. The request's own mode suggestion on a BASH card

Question: an allow answer to a Bash request that carries several suggestions: with only its `setMode` sent back; with all of them; and a `setMode` to another mode than `acceptEdits`.

Fixtures: `r8-bash-set-mode-<variant>-<label>`. Flags: `--permission-mode default --strict-mcp-config --setting-sources local --tools Bash,Read,Edit`. Turn 1: Bash `touch r8-one.txt`; its request gets the variant's allow (`only-set-mode`: the request's `setMode` item only; `all`: every suggestion of the request; `plan` / `dont-ask`: the `setMode` item with `mode` changed to `plan` / `dontAsk`). Every later request gets a plain allow. Turn 2: Bash `touch r8-two.txt`. Turn 3: an Edit of `hello.txt`.

The request and the answer of `r8-bash-set-mode-all-1`, and the status line after it:

```json
{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Bash","display_name":"Bash","input":{"command":"touch r8-one.txt","description":"Create empty file r8-one.txt in working directory"},"description":"Create empty file r8-one.txt in working directory","permission_suggestions":[{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"touch r8-one.txt"}],"behavior":"allow","destination":"localSettings"},{"type":"addDirectories","directories":["/tmp/work/r8-bash-set-mode-all-1"],"destination":"session"},{"type":"setMode","mode":"acceptEdits","destination":"session"}],"blocked_path":"/tmp/work/r8-bash-set-mode-all-1/r8-one.txt","tool_use_id":"toolu-1"}}
{"_sent":{"type":"control_response","response":{"subtype":"success","request_id":"req-1","response":{"behavior":"allow","updatedInput":{"command":"touch r8-one.txt","description":"Create empty file r8-one.txt in working directory"},"updatedPermissions":[{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"touch r8-one.txt"}],"behavior":"allow","destination":"localSettings"},{"type":"addDirectories","directories":["/tmp/work/r8-bash-set-mode-all-1"],"destination":"session"},{"type":"setMode","mode":"acceptEdits","destination":"session"}]}}},"_t":1.188}
{"type":"system","subtype":"status","status":null,"permissionMode":"acceptEdits"}
```

| recording | suggestions of the request | sent back | status lines | `init` mode of turns 2, 3 | requests in turns 2, 3 | `r8-two.txt` made | `hello.txt` at the end | `.claude/settings.local.json` |
|---|---|---|---|---|---|---|---|---|
| `r8-bash-set-mode-only-set-mode-1` | `addRules`, `addDirectories`, `setMode:acceptEdits` | `setMode:acceptEdits` | `acceptEdits` | `acceptEdits`, `acceptEdits` | 0, 0 | yes | `hello three` | none |
| `r8-bash-set-mode-only-set-mode-2` | `addRules`, `addDirectories`, `setMode:acceptEdits` | `setMode:acceptEdits` | `acceptEdits` | `acceptEdits`, `acceptEdits` | 0, 0 | yes | `hello three` | none |
| `r8-bash-set-mode-all-1` | `addRules`, `addDirectories`, `setMode:acceptEdits` | `addRules`, `addDirectories`, `setMode:acceptEdits` | `acceptEdits` | `acceptEdits`, `acceptEdits` | 0, 0 | yes | `hello three` | `{"permissions":{"allow":["Bash(touch r8-one.txt)"]}}` |
| `r8-bash-set-mode-all-2` | `addRules`, `addDirectories`, `setMode:acceptEdits` | `addRules`, `addDirectories`, `setMode:acceptEdits` | `acceptEdits` | `acceptEdits`, `acceptEdits` | 0, 0 | yes | `hello three` | `{"permissions":{"allow":["Bash(touch r8-one.txt)"]}}` |
| `r8-bash-set-mode-plan-1` | `addRules`, `addDirectories`, `setMode:acceptEdits` | `setMode:plan` | `plan` | `plan`, `plan` | 0, 0 | no | `hello` | none |
| `r8-bash-set-mode-plan-2` | `addRules`, `addDirectories`, `setMode:acceptEdits` | `setMode:plan` | `plan` | `plan`, `plan` | 0, 0 | no | `hello` | none |
| `r8-bash-set-mode-dont-ask-1` | `addRules`, `addDirectories`, `setMode:acceptEdits` | `setMode:dontAsk` | `dontAsk` | `dontAsk`, `dontAsk` | 0, 0 | no | `hello` | none |
| `r8-bash-set-mode-dont-ask-2` | `addRules`, `addDirectories`, `setMode:acceptEdits` | `setMode:dontAsk` | `dontAsk` | `dontAsk`, `dontAsk` | 0, 0 | no | `hello` | none |

What the adapter can rely on (n=2 per variant):
- On a Bash card too, the request's own `setMode` item ALONE in `updatedPermissions` switches the session to
  `acceptEdits`: the status line `{"type":"system","subtype":"status","status":null,"permissionMode":"acceptEdits"}`
  at once, the next turns' `init` say `acceptEdits`, and the next `touch` and the next Edit ran with no request. So one
  answer for "approve and accept edits" works on any card that carries a `setMode` suggestion: send that item only.
- Sending ALL suggestions does the same and more: the `addRules` item (`destination` `localSettings`) WRITES the work
  folder's `.claude/settings.local.json` with an allow rule for that exact command. Do not send all of them back.
- A `setMode` with another mode is obeyed: `plan` and `dontAsk` switched the session (status line, next `init`). In
  both, the later turns asked nothing and changed nothing (`r8-two.txt` not made, `hello.txt` unchanged): in `plan` the
  agent plans only, in `dontAsk` every tool use that is not allowed already is refused without a request. (The `plan`
  runs cost more: in plan mode the CLI sent some requests to another model; models seen in them: `claude-haiku-5-5`, `claude-sonnet-5-5`.)

## 10. A slow tool of an HTTP MCP server: the other server kinds and the settings

Question: a tool that takes 70 s on an HTTP server that streams its answer (an event stream) and on one that sends progress notifications: does the call still end at 60 s with "The operation timed out."? Does a setting or a variable the CLI documents move that limit?

Fixtures: `r8-http-slow-<variant>-<label>`. The server `ping-http` runs in the driver (Streamable HTTP on 127.0.0.1) and comes with `--mcp-config` as type `http`; its tool waits 70 s. `sse`: `tools/call` is answered with an event stream (headers at once, the result as one event at 70 s). `progress`: the same stream with a `notifications/progress` event every 10 s. `json-env`: the JSON answer of part 5 (nothing until the tool is done) with `MCP_TOOL_TIMEOUT=120000` in the CLI's environment. `json-timeout`: the JSON answer with `"timeout": 120000` in the server's entry of the config file. Flags of `sse-1`: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --mcp-config /tmp/work/r8-http-slow-sse-1.json`. Bundle: an HTTP request of an MCP client gets a limit of 60 s, or the server's `timeout` / `MCP_TOOL_TIMEOUT` when one is set and larger; it is cleared when the answer's headers come.

The call of `r8-http-slow-progress-1`, its `tool_progress` lines and its result:

```json
# t=2.01
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-1","name":"mcp__ping-http__ping","input":{}}],"…":"…"},"…":"…"}
# t=32.07
{"type":"tool_progress","tool_use_id":"toolu-1-heartbeat-0","tool_name":"mcp__ping-http__ping","parent_tool_use_id":"toolu-1","elapsed_time_seconds":30,"heartbeat":true}
# t=62.07
{"type":"tool_progress","tool_use_id":"toolu-1-heartbeat-1","tool_name":"mcp__ping-http__ping","parent_tool_use_id":"toolu-1","elapsed_time_seconds":60,"heartbeat":true}
# t=72.08
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-1","type":"tool_result","content":[{"type":"text","text":"pong after 70 s"}]}]},"…":"…"}
```

| recording | headers sent (s after the call came) | progress events sent | `progressToken` in the call | `tool_progress` lines (s) | `tool_use` -> tool result (s) | result |
|---|---|---|---|---|---|---|
| `r8-http-slow-sse-1` | 0.000 | 0 | yes | 30.02, 60.03 | 70.038 | `pong after 70 s` |
| `r8-http-slow-sse-2` | 0.000 | 0 | yes | 30.01, 60.02 | 70.025 | `pong after 70 s` |
| `r8-http-slow-progress-1` | 0.001 | 6 | yes | 30.01, 60.01 | 70.023 | `pong after 70 s` |
| `r8-http-slow-progress-2` | 0.000 | 6 | yes | 30.01, 60.01 | 70.019 | `pong after 70 s` |
| `r8-http-slow-json-env-1` | at the end | 0 | yes | 30.03, 60.03 | 70.046 | `pong after 70 s` |
| `r8-http-slow-json-env-2` | at the end | 0 | yes | 30.01, 60.01 | 70.015 | `pong after 70 s` |
| `r8-http-slow-json-timeout-1` | at the end | 0 | yes | 30.01, 60.01 | 70.027 | `pong after 70 s` |
| `r8-http-slow-json-timeout-2` | at the end | 0 | yes | 30.01, 60.01 | 70.016 | `pong after 70 s` |

What the adapter can rely on (n=2 per variant):
- A server that streams its answer (an event stream whose headers come at once) is NOT cut at 60 s: the 70 s call
  returned its result, with or without progress events. Part 5's 60 s end (`The operation timed out.`) belongs to a
  server that sends nothing until the tool is done.
- That 60 s limit can be moved: `MCP_TOOL_TIMEOUT=120000` in the CLI's environment, or `"timeout": 120000` in the
  server's entry of the MCP config, and the same JSON server's 70 s call returned.
- Progress notifications of the server make no `tool_progress` line of their own: in every run the only lines were the
  CLI's heartbeats at 30 and 60 s (`"heartbeat":true`), as for a stdio server. The call did carry a `progressToken`.
- So the watchdog's comment (`has_heartbeat`) can say that an HTTP call gets the same heartbeat, and the sentence for
  the user when an HTTP tool ends at 60 s can name the cause: a server that answers only when done, and the two
  documented ways to give it more time (`MCP_TOOL_TIMEOUT`, the server's `timeout`).

## 11. A shutdown with everything at once: a turn, a background helper and a background command

Question: `interrupt`, the lines until the `result`, `stop_task` for the command, stdin closed: the order, the exit time, anything left running?

Fixtures: `r8-shutdown-all-1`, `r8-shutdown-all-2`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash,Agent`. The CLI is the leader of its own process group, as the app starts it. Turn 1 starts a background command (`echo bg; sleep 229`) and a background helper (its Bash call: `echo begin; sleep 233; echo end`) and ends. Turn 2 runs `echo fg; sleep 239` in the foreground; 2 s after the host sees all three `sleep` processes: `interrupt`, the wait for turn 2's `result`, `stop_task` with the command's id, stdin closed 0.5 s after its answer.

From the note before the stop to the end (`r8-shutdown-all-1`):

```json
# t=6.78
{"_meta":{"before_the_stop":{"list":[{"task_id":"tid-1","run_id":"run-1","task_type":"local_agent","subagent_type":"general-purpose","description":"Run background sleep command"},{"task_id":"tid-2","run_id":"run-2","task_type":"local_bash","description":"Run background echo and sleep"}],"a_turn_runs":true,"command_task":"tid-2","helper_task":"tid-1","sleep_229_alive":true,"sleep_233_alive":true,"sleep_239_alive":true}},"_t":6.783}
# t=6.78
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"interrupt"}},"_t":6.783}
# t=6.78
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"killed","end_time":1795484185614}}
# t=6.78
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-2","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r8-shutdown-all-1/sess-1/tasks/tid-1.output","summary":"Run background sleep command"}
# t=6.78
{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"tid-2","run_id":"run-2","task_type":"local_bash","description":"Run background echo and sleep"}]}
# t=6.78
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"still_queued":[]}}}
# t=6.78
{"type":"system","subtype":"task_notification","task_id":"tid-3","run_id":"run-3","tool_use_id":"toolu-3","status":"stopped","output_file":"","summary":"Print begin, sleep 233 seconds, print end"}
# t=6.78
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-3"}]},"parent_tool_use_id":"toolu-2","agent_id":"tid-1","…":"…"}
# t=6.78
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]},"parent_tool_use_id":"toolu-2","agent_id":"tid-1","…":"…"}
# t=6.78
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-4"}]},"…":"…"}
# t=6.78
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]},"…":"…"}
# t=6.78
{"type":"result","subtype":"error_during_execution","is_error":true,"num_turns":3,"stop_reason":"tool_use","terminal_reason":"aborted_tools","errors":["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=tool_use"],"queued_turn_count":0,"result_index":1,"…":"…"}
# t=7.00
{"_sent":{"type":"control_request","request_id":"host-2","request":{"subtype":"stop_task","task_id":"tid-2"}},"_t":6.999}
# t=7.00
{"type":"system","subtype":"task_updated","task_id":"tid-2","run_id":"run-2","patch":{"status":"killed","end_time":1795484185825}}
# t=7.00
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-1","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r8-shutdown-all-1/sess-1/tasks/tid-2.output","summary":"Run background echo and sleep"}
# t=7.00
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=7.00
{"type":"control_response","response":{"subtype":"success","request_id":"host-2","response":{}}}
# t=7.65
{"_meta":{"before_stdin_is_closed":{"lines_since_the_interrupt":["system/task_updated \"killed\"","system/task_notification \"stopped\"","…(12 more)"],"list":[],"sleep_229_alive":false,"sleep_233_alive":false,"sleep_239_alive":false},"seconds_from_the_interrupt":{"answer":0.011,"result":0.032,"helper_notification":0.011,"command_notification":0.22},"seconds_from_stop_task":{"answer":0.004,"command_notification":0.004}},"_t":7.646}
# t=7.65
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":7.646}
# t=8.37
{"_meta":{"process_exited":true},"_t":8.371}
# t=8.70
{"_meta":{"exit_code":1},"_t":8.703}
# t=9.85
{"_meta":{"after_the_exit":{"lines_after_stdin_was_closed":[],"sleep_229_alive":false,"sleep_233_alive":false,"sleep_239_alive":false,"processes_left_in_the_group":[]}},"_t":9.854}
```

From the note before the stop to the end (`r8-shutdown-all-2`):

```json
# t=6.75
{"_meta":{"before_the_stop":{"list":[{"task_id":"tid-1","run_id":"run-1","task_type":"local_agent","subagent_type":"general-purpose","description":"Run background echo and sleep"},{"task_id":"tid-2","run_id":"run-2","task_type":"local_bash","description":"Run background echo and 229s sleep"}],"a_turn_runs":true,"command_task":"tid-2","helper_task":"tid-1","sleep_229_alive":true,"sleep_233_alive":true,"sleep_239_alive":true}},"_t":6.752}
# t=6.75
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"interrupt"}},"_t":6.752}
# t=6.75
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"killed","end_time":1795484196278}}
# t=6.75
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-2","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r8-shutdown-all-2/sess-1/tasks/tid-1.output","summary":"Run background echo and sleep"}
# t=6.75
{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"tid-2","run_id":"run-2","task_type":"local_bash","description":"Run background echo and 229s sleep"}]}
# t=6.75
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"still_queued":[]}}}
# t=6.75
{"type":"system","subtype":"task_notification","task_id":"tid-4","run_id":"run-4","tool_use_id":"toolu-4","status":"stopped","output_file":"","summary":"Run foreground echo and 239s sleep"}
# t=6.75
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-4"}]},"…":"…"}
# t=6.75
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]},"…":"…"}
# t=6.75
{"type":"system","subtype":"task_notification","task_id":"tid-3","run_id":"run-3","tool_use_id":"toolu-3","status":"stopped","output_file":"","summary":"Print begin, sleep 233 seconds, print end"}
# t=6.75
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-3"}]},"parent_tool_use_id":"toolu-2","agent_id":"tid-1","…":"…"}
# t=6.75
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]},"parent_tool_use_id":"toolu-2","agent_id":"tid-1","…":"…"}
# t=6.75
{"type":"result","subtype":"error_during_execution","is_error":true,"num_turns":3,"stop_reason":"tool_use","terminal_reason":"aborted_tools","errors":["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=tool_use"],"queued_turn_count":0,"result_index":1,"…":"…"}
# t=6.96
{"_sent":{"type":"control_request","request_id":"host-2","request":{"subtype":"stop_task","task_id":"tid-2"}},"_t":6.96}
# t=6.96
{"type":"system","subtype":"task_updated","task_id":"tid-2","run_id":"run-2","patch":{"status":"killed","end_time":1795484196484}}
# t=6.96
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-1","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r8-shutdown-all-2/sess-1/tasks/tid-2.output","summary":"Run background echo and 229s sleep"}
# t=6.96
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=6.96
{"type":"control_response","response":{"subtype":"success","request_id":"host-2","response":{}}}
# t=7.61
{"_meta":{"before_stdin_is_closed":{"lines_since_the_interrupt":["system/task_updated \"killed\"","system/task_notification \"stopped\"","…(13 more)"],"list":[],"sleep_229_alive":false,"sleep_233_alive":false,"sleep_239_alive":false},"seconds_from_the_interrupt":{"answer":0.005,"result":0.014,"helper_notification":0.005,"command_notification":0.211},"seconds_from_stop_task":{"answer":0.003,"command_notification":0.003}},"_t":7.613}
# t=7.62
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":7.615}
# t=8.34
{"_meta":{"process_exited":true},"_t":8.335}
# t=8.69
{"_meta":{"exit_code":1},"_t":8.685}
# t=9.87
{"_meta":{"after_the_exit":{"lines_after_stdin_was_closed":[],"sleep_229_alive":false,"sleep_233_alive":false,"sleep_239_alive":false,"processes_left_in_the_group":[]}},"_t":9.868}
```

What the adapter can rely on (n=2):
- `interrupt` with a turn running ends the HELPER first (`killed`, `stopped`, a list line that still names the
  command: 1 listed) and answers. Then each Bash call that had become a task gets a `task_notification` `stopped` (the
  helper's own call in both runs; the turn's foreground call in the one run where it had run long enough, about 3 s,
  to become a task), the helper's two `user` lines and the turn's two `user` lines (rejected tool result,
  `[Request interrupted by user for tool use]`) come in either order, then the turn's `result`
  `error_during_execution` (0.032 / 0.014 s after the request). The command is not touched.
- `stop_task` with the command's id then ends it (`killed`, `stopped`, the list `[]`, the answer `{}`).
- After stdin close the process was gone in 0.73 / 0.72 s, exit code 1 (the interrupted turn had failed). Nothing was left: no
  `sleep` of the scenario, an empty process group.
- So the order of part 5 holds with a turn running: `interrupt`, wait for the turn's `result`, `stop_task` for each
  listed command, close.

## 12. `apply_flag_settings` with an effort level while a turn runs

Question: `effortLevel: null` and a level, sent while a turn runs: the answer, and what `get_settings` says during and after the turn.

Fixtures: `r8-effort-mid-turn-null-*` (`{"effortLevel": null}`), `r8-effort-mid-turn-high-*` (`{"effortLevel": "high"}`). Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --effort low`. The turn runs `echo go; sleep 13.5`; 1 s after the host sees the `sleep`: `get_settings`, the request, `get_settings`; then `get_settings` after the turn's result and after one more turn.

The request and its answer while the call runs (`r8-effort-mid-turn-high-1`):

```json
{"type":"control_request","request_id":"host-4","request":{"subtype":"apply_flag_settings","settings":{"effortLevel":"high"}}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-4"}}
```

| recording | as started | during the call, before the request | during the call, after the request | after the turn's result | after the next turn | the request's answer |
|---|---|---|---|---|---|---|
| `r8-effort-mid-turn-null-1` | `low` | `low` | `medium` | `medium` | `medium` | `success` |
| `r8-effort-mid-turn-null-2` | `low` | `low` | `medium` | `medium` | `medium` | `success` |
| `r8-effort-mid-turn-high-1` | `low` | `low` | `high` | `high` | `high` | `success` |
| `r8-effort-mid-turn-high-2` | `low` | `low` | `high` | `high` | `high` | `success` |

What the adapter can rely on (n=2 per variant):
- The request is taken while a turn runs (its call still running): answer success, and `get_settings` says the new
  level at once (`null` -> `medium`, `high` -> `high`), after the turn's result and after the next turn.
- So an effort change need not be refused while a turn runs, as far as the CLI's own state goes. Still not visible:
  whether the model request that follows in the same turn uses the new level (no line says which level a request
  carries).

## Notes written late

Each note of this part that lists lines is compared with the lines in the fixture between the step the script marked just before it (named in the last column) and the note:

| fixture | note | lines in its list | lines in the file before it | counted from |
|---|---|---|---|---|
| `r8-after-budget-helper-goes-on-1-calib` | `after_the_exit` | 0 | 0 | the close of stdin |
| `r8-after-budget-helper-goes-on-1-t1` | `forty_s_after_the_stop` | 0 | 0 | the note `run` |
| `r8-after-budget-helper-goes-on-1-t1` | `after_the_exit` | 0 | 0 | the close of stdin |
| `r8-after-budget-helper-goes-on-1-t2` | `forty_s_after_the_stop` | 0 | 0 | the note `run` |
| `r8-after-budget-helper-goes-on-1-t2` | `after_the_exit` | 2 | 2 | the close of stdin |
| `r8-after-budget-helper-goes-on-1-t3` | `forty_s_after_the_stop` | 0 | 0 | the note `run` |
| `r8-after-budget-helper-goes-on-1-t3` | `after_the_exit` | 2 | 2 | the close of stdin |
| `r8-after-budget-helper-goes-on-2-calib` | `after_the_exit` | 0 | 0 | the close of stdin |
| `r8-after-budget-helper-goes-on-2-t1` | `forty_s_after_the_stop` | 0 | 0 | the note `run` |
| `r8-after-budget-helper-goes-on-2-t1` | `after_the_exit` | 0 | 0 | the close of stdin |
| `r8-after-budget-helper-goes-on-2-t2` | `forty_s_after_the_stop` | 0 | 0 | the note `run` |
| `r8-after-budget-helper-goes-on-2-t2` | `after_the_exit` | 2 | 2 | the close of stdin |
| `r8-after-budget-interrupt-1-calib` | `after_the_exit` | 0 | 0 | the close of stdin |
| `r8-after-budget-interrupt-1-t1` | `after_the_exit` | 0 | 0 | the close of stdin |
| `r8-after-budget-interrupt-1-t2` | `after_the_request` | 1 | 1 | the `interrupt` or `stop_task` request |
| `r8-after-budget-interrupt-1-t2` | `after_the_exit` | 2 | 2 | the close of stdin |
| `r8-after-budget-interrupt-2-calib` | `after_the_exit` | 0 | 0 | the close of stdin |
| `r8-after-budget-interrupt-2-t1` | `after_the_exit` | 0 | 0 | the close of stdin |
| `r8-after-budget-interrupt-2-t2` | `after_the_request` | 1 | 1 | the `interrupt` or `stop_task` request |
| `r8-after-budget-interrupt-2-t2` | `after_the_exit` | 2 | 2 | the close of stdin |
| `r8-after-budget-stop-task-1-calib` | `after_the_exit` | 0 | 0 | the close of stdin |
| `r8-after-budget-stop-task-1-t1` | `after_the_request` | 3 | 3 | the `interrupt` or `stop_task` request |
| `r8-after-budget-stop-task-1-t1` | `after_the_exit` | 0 | 0 | the close of stdin |
| `r8-after-budget-stop-task-2-calib` | `after_the_exit` | 0 | 0 | the close of stdin |
| `r8-after-budget-stop-task-2-t1` | `after_the_exit` | 0 | 0 | the close of stdin |
| `r8-after-budget-stop-task-2-t2` | `after_the_request` | 3 | 3 | the `interrupt` or `stop_task` request |
| `r8-after-budget-stop-task-2-t2` | `after_the_exit` | 0 | 0 | the close of stdin |
| `r8-bash-task-fails-1` | `turn` | 5 | 5 | the `interrupt` request |
| `r8-bash-task-fails-2` | `turn` | 5 | 5 | the `interrupt` request |
| `r8-shutdown-all-1` | `before_stdin_is_closed` | 14 | 14 | the `interrupt` request |
| `r8-shutdown-all-1` | `after_the_exit` | 0 | 0 | the close of stdin |
| `r8-shutdown-all-2` | `before_stdin_is_closed` | 15 | 15 | the `interrupt` request |
| `r8-shutdown-all-2` | `after_the_exit` | 0 | 0 | the close of stdin |
| `r8-stop-command-last-request-1` | `after_stop_task` | 6 | 6 | the `stop_task` request |
| `r8-stop-command-last-request-2` | `after_stop_task` | 6 | 6 | the `stop_task` request |
| `r8-stop-command-mid-turn-1` | `after_stop_task` | 10 | 10 | the `stop_task` request |
| `r8-stop-command-mid-turn-3` | `after_stop_task` | 10 | 10 | the `stop_task` request |
| `r8-stop-command-mid-turn-4` | `after_stop_task` | 10 | 10 | the `stop_task` request |
| `r8-stop-helper-last-request-1` | `after_stop_task` | 12 | 12 | the `stop_task` request |
| `r8-stop-helper-last-request-2` | `after_stop_task` | 12 | 12 | the `stop_task` request |

All 40 notes of this part that list lines list exactly the lines in their window: none is late.

## Can the waiting code be built now? (as of the day of recording)

| # | the code that waits | can it be built now? |
|---|---|---|
| 1 | `claude-part5-truth` item 1: `build_args` drops `--strict-mcp-config` in a folder you have not trusted when "Use my Claude setup" is on | partly: with `--setting-sources user` yes (the repo's `.mcp.json` server never loads, 24 recordings here and 16 before; a plugin the user turned off stays off). With `user,local` NO: the repo's `.claude/settings.local.json` turns on a plugin the user turned off, and its server starts (n=4). Take the item back for `user,local` |
| 2 | "Undo a turn" for the conversation (`checkpoints.rs`; the rewind list of `rewind-from-thread`) | yes: `--resume <id> --resume-session-at <uuid of the kept turn's last assistant line>` drops the later turns (n=2), same session id, the old lines stay in the file; the uuid comes from the stream's assistant lines. A dropped part cannot be resumed again by its uuid once a new turn was added |
| 3 | "Try again from here" and "branch this agent" | yes: `--resume <id> --fork-session` (with `--resume-session-at <uuid>` to cut) gives a new session id and leaves the old session as it was (n=2). Count cost from `model_usage_since_last_result`: the fork's `total_cost_usd` carries the old session's total |
| 4 | Codly's restore note vs the CLI's own rewind (`with_restore_note`) | partly: `CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING=1` turns `rewind_files` on (the setting does not); it puts Edit's files back to the state before the turn of a user message uuid of the host (n=2). Not recorded: files written by Bash, new files, helpers' edits |
| 5 | `stopHint` (`session.ts`) and the gate's stop (`background.rs`) | yes: a helper stopped during the turn's last model request gets a turn by itself after the `result` (n=2); during a tool call of the turn it is told inside the turn (part 5). A stopped background COMMAND started no turn (n=5) |
| 6 | `run_limit_turn_end` (`breaker_run.rs`) and the comment at `SpawnSpec::max_budget_usd` | partly: a call that runs on ends by `stop_task` with its task id (n=2) or by closing stdin, not by `interrupt` (n=2); after the `result` no line came in 40 s (n=2). Before the `result` a helper made one more model request past the limit (n=2) and the total ended 19% and 45% above the budget; the CLI stopped a helper that still worked in one run (n=1). Ending the run is needed only for the call that runs on; the `max_budget_usd` comment must say the total can pass the budget |
| 7 | `on_task` (a line that must end no helper and no background row) | yes: a foreground call that fails ends with `task_notification` `failed`, one cut by an interrupt with `stopped`, same keys, `output_file` `""` (n=2 each) |
| 8 | "Turn off" while a turn runs (`AgentMcp.svelte`, `session.ts`) | no: the switch while a turn runs was not recorded (a throw-away HOME has no login). Recorded: a server that cannot start, off, then on in a new process: error `Connection closed`, the saved switch on again (n=2) |
| 9 | One answer for "approve and accept edits" on any card (`grantable`) | yes: on a Bash card too, the request's `setMode` item alone switches to `acceptEdits` (n=2). Never send all suggestions back: `addRules` writes `.claude/settings.local.json`. Another mode (`plan`, `dontAsk`) is obeyed |
| 10 | The `has_heartbeat` comment (`watchdog.rs`) and the sentence for an HTTP tool that ends at 60 s | yes: a streaming HTTP server is not cut at 60 s; the limit moves with `MCP_TOOL_TIMEOUT` or the server's `timeout` (n=2 each); progress notifications add no line, only the heartbeat |
| 11 | `begin_shutdown` with a turn running (`claude.rs`) | yes: `interrupt`, the turn's `result`, `stop_task` for the command, close: exit in 0.73 / 0.72 s, nothing left (n=2) |
| 12 | An effort change while a turn runs (`live_model_switch`, `runs.rs`) | partly: the CLI takes it mid-turn and `get_settings` shows it at once (n=2 per variant); which level the turn's next model request uses is not visible |

## Table: question -> answer -> fixture

| # | question | answer | n | fixture |
|---|---|---|---|---|
| 1 | Your own setup in a folder you have not trusted: user flags for project servers, plugins | The repo's `.mcp.json` server never starts; a user's plugin loads; under `user,local` the repo's `settings.local.json` turns on a plugin the user turned off | 2 per form | `r8-untrusted-mcp-*` |
| 2 | Rewind with `--resume-session-at` | Later turns dropped, same session id, old lines kept; a plain resume follows the rewound branch; the dropped uuid is not found later | 2 | `r8-rewind-*` |
| 3 | Fork with `--fork-session` | New session id; knows the old conversation (or up to `--resume-session-at`); `total_cost_usd` = old total + own turn; the old session unchanged | 2 | `r8-fork-*` |
| 4 | The CLI's own file rewind | On only with `CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING`; puts files back to before the message's turn; off: "File rewinding is not enabled." | 2 per way | `r8-file-rewind-*` |
| 5 | `stop_task` in the last moment; on a command mid-turn | Helper during the last request: a turn by itself follows; a command: no turn | 2, 5 | `r8-stop-*` |
| 6 | After a budget stop | `interrupt` leaves the running call; `stop_task` ends it; a helper made one more request past the limit, then nothing came after the `result` in 40 s; the total passed the budget | 2 each (the CLI stopping a working helper: 1) | `r8-after-budget-*` |
| 7 | A Bash task that fails; one cut by an interrupt | `task_notification` `failed` / `stopped`, same keys, `output_file` `""` | 2 | `r8-bash-task-fails-*` |
| 8 | MCP switch mid-turn; a failing server off and on in a new process | Mid-turn: not recorded. New process: `disabled`, not started; "on" fails with `Connection closed` and saves the switch on | 2 | `r8-mcp-cannot-start-saved-*` |
| 9 | `setMode` on a Bash card | The `setMode` item alone switches to `acceptEdits`; all suggestions also write `settings.local.json`; `plan` and `dontAsk` obeyed | 2 per variant | `r8-bash-set-mode-*` |
| 10 | A slow HTTP MCP tool, other kinds and settings | An event stream outlasts 60 s; `MCP_TOOL_TIMEOUT` and the server's `timeout` move the limit; only heartbeats | 2 per variant | `r8-http-slow-*` |
| 11 | Shutdown with a turn, a helper and a command | interrupt ends the helper and the turn, `stop_task` the command, exit after close, nothing left | 2 | `r8-shutdown-all-*` |
| 12 | Effort switch while a turn runs | Taken at once (`null` -> `medium`, `high`), shown by `get_settings` during and after | 2 per variant | `r8-effort-mid-turn-*` |

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
- A turn that fails by an API error while a helper works.
- A REAL home for the saved MCP switch (all switch recordings use a throw-away HOME).

Open after these recordings:
- 1: a plugin the user never installed, named by a repo file (with `extraKnownMarketplaces`); a plugin's hooks and
  skills under `user,local`; the logged-in forms of this section.
- 2: a rewind to a user message's uuid; a rewind in a turn with tool calls or helpers; `--resume-drops-turn`.
- 3: a fork of a session with a helper or a background command still running.
- 4: a file written by Bash, a new file, a helper's edit; whether the agent is told of the rewind.
- 6: a helper whose limit passes during a long single request (the story) while it still has more to do.
- 8: `mcp_toggle` while a turn runs (needs a throw-away HOME with a login).
- 10: an HTTP server whose event stream sends nothing for more than 5 minutes (bundle: an idle limit of 300 s for
  servers that are not stdio, `CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT`).
- 12: the level a model request carries after the switch.

