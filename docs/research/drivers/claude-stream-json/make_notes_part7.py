"""Builds the part-7 document from the sanitized r9-* fixtures, so every JSON line quoted there is copied from a fixture.

python3 make_notes_part7.py [output file]      (default: $CAPTURE_DIR/NOTES-part7.md)

Numbers and words that the text states about a fixture are checked against it (`check`): the build fails when a
recording no longer says what the text says, and when a note that lists lines was written late ("Notes written late").
Section 1 also reads the fixtures of parts 4 to 6 (r6-project-mcp-*, r7-untrusted-mcp-*, r8-untrusted-mcp-*) for its
last sentence.
"""
import os
import sys

from notes_lib import OUT, block, check, flags, fx, is_cli, j, meta, results, sub, tl

P = []
w = P.append

FIX = os.environ.get("FIXTURES_DIR") or os.path.join(OUT, "fixtures")
NAMES = sorted(f[:-len(".ndjson")] for f in os.listdir(FIX) if f.endswith(".ndjson"))
ALL = [n for n in NAMES if n.startswith("r9-")]
for n in ALL:
    check("claude 2.1.311, recorded 2026-11-23." in fx(n)[0]["_note"], "the version and the day of recording of " + n)


def named(prefix):
    return [n for n in ALL if n.startswith(prefix)]


def note(name, key):
    """The one driver note of a fixture that has this key: the value of the key."""
    found = meta(name, key)
    check(len(found) == 1, "one note %r in %s" % (key, name))
    return found[0][key]


def whole(name, key):
    """The one driver note of a fixture that has this key, whole."""
    found = meta(name, key)
    check(len(found) == 1, "one note %r in %s" % (key, name))
    return found[0]


def one(name, key, value):
    found = [m for m in meta(name, key) if m[key] == value]
    check(len(found) == 1, "one note %s=%r in %s" % (key, value, name))
    return found[0]


def both(names):
    return "`%s`" % "`, `".join(names)


def labels(prefix, part=None):
    """The labels of a scenario: `<prefix><label>-<part>` (or `<prefix><label>` when part is None)."""
    if part is None:
        return sorted(n[len(prefix):] for n in named(prefix) if n[len(prefix):].isdigit())
    return sorted({n[len(prefix):-len(part) - 1] for n in named(prefix) if n.endswith("-" + part) and n[len(prefix):-len(part) - 1].isdigit()})


def turn_costs(names):
    """What the recordings spent: the cost of each turn's own model requests (`model_usage_since_last_result`, costUSD),
    summed over every `result` line. A resumed or forked session's `total_cost_usd` carries the earlier cost."""
    return sum(m.get("costUSD", 0.0) for n in names for r in results(n) for m in (r.get("model_usage_since_last_result") or {}).values())


def running_totals(names):
    """The sum, per fixture, of each session's last `total_cost_usd` (what cost.py prints)."""
    total = 0.0
    for n in names:
        last = {}
        for r in results(n):
            last[r.get("session_id")] = r.get("total_cost_usd") or 0.0
        total += sum(last.values())
    return total


SPENT, RUNNING = turn_costs(ALL), running_totals(ALL)
check(SPENT < 1.5 and RUNNING < 1.5, "the session's cost under the stop line of 1.50 USD")

# ================================================================= 1: the numbers the head states
PLUGIN, MARKET = "ping-plugin", "codly-test-market"
HOOKS = ["SessionStart", "UserPromptSubmit"]
FORMS1 = ("repo-both", "repo-local", "user-declared", "off-repo-local", "on", "off")
SRC1 = {"user": "user", "user-local": "user,local"}
R1 = {}  # (form, src) -> [label]
for form in FORMS1:
    for src in SRC1:
        R1[form, src] = labels("r9-plugin-from-repo-%s-%s-" % (form, src), "a")
        check(len(R1[form, src]) >= 2 and all("r9-plugin-from-repo-%s-%s-%s-b" % (form, src, lab) in ALL for lab in R1[form, src]),
              "two runs of two processes each of %s-%s" % (form, src))


def run1(form, src, lab):
    """What a run of section 1 started: (server starts, hook lines) after process a and after process b (b's counts go
    on from a's: the marker files are not reset between the two starts), whether init named the plugin in a and b."""
    a, b = ["r9-plugin-from-repo-%s-%s-%s-%s" % (form, src, lab, p) for p in "ab"]
    na, nb = whole(a, "after_the_turn"), whole(b, "after_the_turn")
    check(whole(a, "before_first_message")["server_starts"] == na["server_starts"], "no server start after the message in " + a)
    return dict(a=(na["server_starts"], na["hook_lines"]), b=(nb["server_starts"], nb["hook_lines"]),
                init=(na["init_names_the_plugin"], nb["init_names_the_plugin"]), servers=(na["init_mcp_servers"], nb["init_mcp_servers"]),
                files=(na["plugin_files_in_the_home"], nb["plugin_files_in_the_home"]))


LOADED = ((1, HOOKS), (2, HOOKS + HOOKS))
NOTHING = ((0, []), (0, []))
OUT1 = {k: [run1(k[0], k[1], lab) for lab in labs] for k, labs in R1.items()}
for k, runs in OUT1.items():
    loads = k[0] in ("user-declared", "on") or k == ("off-repo-local", "user-local")
    for r in runs:
        check((r["a"], r["b"]) == (LOADED if loads else NOTHING) and r["init"] == ((True, True) if loads else (False, False)), "what %s-%s loaded" % k)
        if loads:
            check(r["servers"][0] == r["servers"][1] == [{"name": "plugin:ping-plugin:ping-plugged", "status": "connected", "source": "plugin"}],
                  "the plugin's server in the init of %s-%s" % k)
NEVER = [k for k in OUT1 if k[0] in ("repo-both", "repo-local")]
N_NEVER = sum(len(R1[k]) for k in NEVER)
N_REPO_ON = len(R1["off-repo-local", "user-local"])
for k in NEVER:
    check(all(r["files"] == ([], []) for r in OUT1[k]), "nothing of the plugin in the HOME for %s-%s" % k)


def argv_sources(name):
    a = fx(name)[0]["_argv"]
    return a[a.index("--setting-sources") + 1] if "--setting-sources" in a else None, "--strict-mcp-config" in a


def folder_started_nothing(name):
    """Under `--setting-sources user` alone: True when no file of the work folder started or loaded anything, by the
    notes of that part's scripts. The scenario's servers and plugins that are the USER's own (a plugin installed and
    on for the user, a local-scope server, the same declaration in the user settings file) are not files of the
    folder and are not counted."""
    if name.startswith("r6-"):
        m = [x for x in meta(name, "server_starts")][0]
        return m["server_starts"] == 0 and m.get("hook_file_lines", 0) == 0
    if name.startswith("r7-"):
        return [x for x in meta(name, "server_starts")][0]["server_starts"]["project"] == 0
    if name.startswith("r8-"):
        s = [x for x in meta(name, "server_starts")][0]["server_starts"]
        return s["project"] == 0 and (s["plugin"] == 0 or "-plugin-user-" in name)
    r = whole(name, "after_the_turn")
    return r["server_starts"] == 0 and r["hook_lines"] == []


USER_ALONE = {}
for prefix, part in (("r6-project-mcp-", 4), ("r7-untrusted-mcp-", 5), ("r8-untrusted-mcp-", 6), ("r9-plugin-from-repo-", 7)):
    found = []
    for n in NAMES:
        if not n.startswith(prefix):
            continue
        src, strict = argv_sources(n)
        if src != "user" or strict:
            continue
        if part == 4 and "-hooks-" in n:
            continue  # the check of the hook switch: hooks of the HOME's user settings, no file of the folder
        if part == 6 and "-plugin-user-" in n:
            continue  # the user's own plugin, installed and on: no file of the folder
        if part == 7 and ("-user-declared-" in n or "-on-user-" in n or "-off-user-" in n):
            continue  # controls: what the user's own settings do, with no file of the folder
        found.append(n)
    USER_ALONE[part] = found
check(all(USER_ALONE[p] for p in USER_ALONE), "recordings under `user` alone in each of parts 4 to 7")
check(all(folder_started_nothing(n) for ns in USER_ALONE.values() for n in ns), "nothing of the folder started under `user` alone")
N_USER_ALONE = sum(len(v) for v in USER_ALONE.values())

w("""# Claude Code CLI stream-json: recorded protocol shapes, part 7

**QUESTION 1 FIRST. A PLUGIN THE USER NEVER INSTALLED, NAMED BY A FILE OF THE FOLDER (`extraKnownMarketplaces` WITH A
FOLDER SOURCE PLUS `enabledPlugins`, IN `.claude/settings.json` AND `.claude/settings.local.json`, OR IN
`.claude/settings.local.json` ALONE), WAS NOT LOADED: NO SERVER, NO HOOK, NOTHING OF IT IN THE HOME, UNDER `--setting-sources
user` AND UNDER `user,local`, AT A FIRST AND AT A SECOND START (%d RUNS, 2 STARTS EACH). THE SAME TWO KEYS IN THE USER'S
OWN SETTINGS FILE DO LOAD IT (CONTROL, %d RUNS). BUT A PLUGIN THE USER INSTALLED AND TURNED OFF, TURNED ON BY THE
FOLDER'S `.claude/settings.local.json`, RUNS ITS HOOKS (`SessionStart`, `UserPromptSubmit`) AND STARTS ITS SERVER
UNDER `user,local` (%d of %d runs, every start); UNDER `user` IT DOES NOT (%d of %d runs). UNDER `--setting-sources user`
ALONE NO FILE OF THE FOLDER STARTED OR LOADED ANYTHING IN ANY OF THE %d RECORDINGS OF PARTS 4 TO 7 THAT TRIED
(part 4: %d, part 5: %d, part 6: %d, part 7: %d).** So for a folder you have not trusted: `user` alone keeps every
file of the folder out in what was recorded; `user,local` lets the folder's local settings file run a turned-off
plugin's hooks (and part 4 recorded that file's own hooks running too).

- CLI: `claude` 2.1.311 (the same version as parts 1 to 6; checked with `claude --version` before the runs).
- Recorded: 2026-11-23, macOS, `--model haiku` (`claude-haiku-5-5`), subscription login (`"apiKeySource":"none"`).
- Method, base command, fixture line format and sanitizing: as in part 1 ("Base command", "Files", "Sanitizing") with
  the additions of parts 2 to 6. Fixtures of this part are `fixtures/claude-2.1.311/r9-*.ndjson`; the scripts are
  `drivers/claude-stream-json/r9_*.py` (README there says how to record again).
- Cost of the %d recordings at list price: %.4f USD, the sum of each turn's own model requests
  (`model_usage_since_last_result`). `cost.py` prints %.4f: it sums each session's last `total_cost_usd` per
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
""" % (N_NEVER, sum(len(R1["user-declared", s]) for s in SRC1), N_REPO_ON, N_REPO_ON, 0, len(R1["off-repo-local", "user"]), N_USER_ALONE,
       len(USER_ALONE[4]), len(USER_ALONE[5]), len(USER_ALONE[6]), len(USER_ALONE[7]), len(ALL), SPENT, RUNNING))

# ================================================================= 1
w("## 1. A plugin named by a file of the folder, and a plugin's hooks, in a folder you have not trusted\n")
w("Question: with a throw-away HOME, no `--strict-mcp-config`, `--setting-sources user` and `user,local`: is anything of a plugin the "
  "user NEVER installed loaded or started when a file of the folder names it (`extraKnownMarketplaces` plus `enabledPlugins`)? Does "
  "the HOOK of a plugin the user turned off run when the folder's local settings file turns it on?\n")
w("Fixtures: `r9-plugin-from-repo-<form>-<sources>-<label>-a` and `-b` (two starts in the same HOME and folder). Flags of "
  "`off-repo-local-user-local-1-a`: %s. Every run has a THROW-AWAY HOME (no login: each message ends at once with `Not logged in`; the "
  "recordings show what was LOADED and STARTED, and what the hooks did). The plugin `ping-plugin` lives in a local marketplace folder "
  "`codly-test-market` under the capture folder; its `.mcp.json` brings `ping-plugged` (one line in `started-plugin.txt` per start) and its "
  "`hooks/hooks.json` has a `SessionStart` and a `UserPromptSubmit` hook (each appends its event's name to `hook-ran.txt`). The forms: "
  "`repo-both` (the user never installed it; both repo files say `extraKnownMarketplaces` with a `directory` source plus `enabledPlugins`), "
  "`repo-local` (the same in `.claude/settings.local.json` only), `user-declared` (control: the same two keys in the HOME's user "
  "settings file), `on` (control: installed and on for the user, with `claude plugin marketplace add` and `claude plugin install --scope "
  "user`), `off` (control: then `claude plugin disable --scope user`), `off-repo-local` (turned off for the user; the repo's "
  "`.claude/settings.local.json` turns it on).\n" % flags("r9-plugin-from-repo-off-repo-local-user-local-%s-a" % R1["off-repo-local", "user-local"][0]))
n = "r9-plugin-from-repo-repo-both-user-local-%s-a" % R1["repo-both", "user-local"][0]
w("The set-up note and the `init` line of `%s` (the user never installed the plugin):\n" % n)
w(block([j(note(n, "setup"), maxlist=9)] + tl(n, lambda l: sub(l) == "init")))
n = "r9-plugin-from-repo-off-repo-local-user-local-%s-a" % R1["off-repo-local", "user-local"][0]
w("`%s` (turned off for the user, on in the repo's local file): the hook lines of the stream, the `init` line and the note after the "
  "turn:\n" % n)
w(block(tl(n, lambda l: sub(l) in ("hook_started", "hook_response", "init")) + [j(whole(n, "after_the_turn"), maxlist=2)]))
n = "r9-plugin-from-repo-off-repo-local-user-%s-a" % R1["off-repo-local", "user"][0]
w("The same files under `--setting-sources user` (`%s`): the `init` line and the note after the turn:\n" % n)
w(block(tl(n, lambda l: sub(l) == "init") + [j(whole(n, "after_the_turn"))]))
FORM1 = {"repo-both": "never installed; both repo files declare and enable it", "repo-local": "never installed; the repo's local file declares and enables it",
         "user-declared": "never installed; the user settings file declares and enables it (control)", "on": "installed and on for the user (control)",
         "off": "installed, off for the user (control)", "off-repo-local": "installed, off for the user; the repo's local file turns it on"}


def counts(x):
    return "%d, %s" % (x[0], "+".join(x[1]) or "none")


w("| form | `--setting-sources` | what the HOME and the repo have | runs | after start a: server starts, hook lines | after start b (counted from a on) | `init` names the plugin (a, b) |")
w("|---|---|---|---|---|---|---|")
for (form, src), runs in OUT1.items():
    w("| `%s` | `%s` | %s | %d | %s | %s | %s |" % (form, SRC1[src], FORM1[form], len(runs), " / ".join(counts(r["a"]) for r in runs),
                                                 " / ".join(counts(r["b"]) for r in runs), " / ".join("%s, %s" % r["init"] for r in runs)))
for src in SRC1:
    for lab in R1["off-repo-local", src]:
        m = note("r9-plugin-from-repo-off-repo-local-%s-%s-a" % (src, lab), "setup")
        check(m["user_settings_file"]["enabledPlugins"] == {"%s@%s" % (PLUGIN, MARKET): False} and m["repo_files"] == {"settings.local.json": {"enabledPlugins": {"%s@%s" % (PLUGIN, MARKET): True}}},
              "the plugin off for the user and on in the repo in off-repo-local-%s-%s" % (src, lab))
w("""
What the adapter can rely on:
- A plugin the user never installed is NOT loaded when a file of the folder names it, under `user` or `user,local`
  (`repo-both`, `repo-local`; n=2 per form and sources, %d runs, each with two starts): no server start, no hook line,
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
  got both lines), at both starts, and its server started (n=%d runs). Under `user` it stays off (n=%d). So part 6's
  finding holds for the plugin's hooks too: under `user,local` the folder's local file can make code the user turned
  off run before the first model request, and without a login.
- Under `user` alone no file of the folder started or loaded anything in any of the %d recordings of parts 4 to 7 that
  put such a file there (part 4: %s; part 5: %s; part 6: %s; part 7: %s).
- Not recorded: a marketplace a repo file declares with a network source (`github`, `git`, `url`: the CLI would fetch it;
  this part has no network form), an inline `settings` source, and any of this with the real login.
""" % (N_NEVER, N_REPO_ON, len(R1["off-repo-local", "user"]), N_USER_ALONE, both(USER_ALONE[4]), both(USER_ALONE[5]),
       "the %d `r8-untrusted-mcp-*-user-*` recordings but `plugin-user` (the user's own plugin)" % len(USER_ALONE[6]),
       "the %d recordings of the forms `repo-both`, `repo-local`, `off-repo-local` under `user`" % len(USER_ALONE[7])))
check(not any("_stderr" in l for k in NEVER for lab in R1[k] for p in "ab" for l in fx("r9-plugin-from-repo-%s-%s-%s-%s" % (k[0], k[1], lab, p))),
      "no stderr line in the never-installed forms")

# ================================================================= 2
w("## 2. Rewind a conversation that has tool calls, a helper and an interrupted turn\n")
w("Question: after `--resume <id> --resume-session-at <uuid>` with the uuid of the last assistant line of a turn with tool calls, of a "
  "turn with a helper, and of an interrupted turn: does the next turn start, does it know only the kept turns, does any line say that a "
  "tool call has no result? Which `uuid` does the host see on the final assistant line of a turn (with `--include-partial-messages`)? "
  "The control: the same uuid resumed before a rewound turn was added. Also: a rewind to a user message's uuid; `--resume-drops-turn`.\n")
L2 = labels("r9-rewind-tools-", "a")
check(len(L2) == 2 and all("r9-rewind-tools-%s-%s" % (lab, p) in ALL for lab in L2 for p in "bcdef"), "two labels of section 2, six processes each")
WORDS2 = ["lantern", "pebble", "violet", "copper"]
a = "r9-rewind-tools-%s-a" % L2[0]
w("Fixtures: `r9-rewind-tools-<label>-a` … `-f` and `r9-rewind-drops-<label>-a` … `-c`, labels %s. Flags of `-a`: %s. Process a: four "
  "turns, each tells one secret word (%s) and carries a uuid of the host: turn 1 text only; turn 2 a Read, an Edit and a Bash call; turn 3 a "
  "helper (`Agent`) that runs a Bash call; turn 4 a Bash call `echo copper-start; sleep 19.5; echo copper-end`, and an `interrupt` once "
  "its `sleep` runs. Then one process each, in this order, asks (no tools) for every secret word: b `--resume-session-at <the last "
  "assistant line of turn 4>` (the interrupted call's line), c `… <the end of turn 3>`, d `… <the end of turn 2>`, e `… <the uuid of "
  "turn 2's USER message>`, f `… <the end of turn 3>` again (c is its control: the same uuid before d and e added turns on an earlier "
  "branch).\n" % (", ".join(L2), flags(a), ", ".join("`%s`" % x for x in WORDS2)))
w("Process a's note per turn (`%s`):\n" % a)
w(block([j(one(a, "turn", i), maxlist=6) for i in (1, 2, 3, 4)]))
ROWS2 = []
for lab in L2:
    a = "r9-rewind-tools-%s-a" % lab
    t = [one(a, "turn", i) for i in (1, 2, 3, 4)]
    check([x["result_subtype"] for x in t] == ["success"] * 3 + ["error_during_execution"], "turn 4 is the interrupted one in " + a)
    check(all(x["replayed_user_lines"] == [{"uuid_is_the_hosts": True, "isReplay": True}] and x["result_user_message_uuid_is_the_hosts"] for x in t), "the echo in " + a)
    check(all(all(x["assistant_uuids_in_the_transcript"]) for x in t) and all(x["stream_event_uuids_in_the_transcript"] == 0 < x["stream_events_with_a_uuid"] for x in t),
          "the uuids of the stream and of the transcript in " + a)
    check(sorted(t[1]["tools_used"]) == ["Bash", "Edit", "Read"] and "Agent" in t[2]["tools_used"] and t[3]["tools_used"] == ["Bash"], "the tools of the four turns in " + a)
    check(whole(a, "before_the_interrupt")["sleep_19_5_alive"] and note(a, "transcript")["transcript_found"], "the call ran at the interrupt in " + a)
    asked = {p: one("r9-rewind-tools-%s-%s" % (lab, p), "asked", p) for p in "bcdef"}
    for p in "bcde":
        x = asked[p]
        check(x["result_subtype"] == "success" and x["init_came"] and x["init_session_id_is_the_first_session"] and x["result_session_id_is_the_first_session"],
              "a resumed turn in the same session in %s-%s" % (lab, p))
    check([asked[p]["words_named"] for p in "bcde"] == [WORDS2, WORDS2[:3], WORDS2[:2], WORDS2[:2]], "the words the rewound turns knew in " + lab)
    check(asked["b"]["lines"] == ["system/init", "assistant", "assistant", "result/success"] or asked["b"]["lines"] == ["system/init", "assistant", "result/success"],
          "no line about a call with no result in %s-b" % lab)
    check(all(asked[p]["tools_used"] == [] for p in "bcd") and asked["e"]["tools_used"] != [], "the tools of the rewound turns in " + lab)
    f = asked["f"]
    check(f["result_subtype"] == "error_during_execution" and not f["init_came"] and f["result_errors"][0].startswith("No message found with message.uuid of: ")
          and note("r9-rewind-tools-%s-f" % lab, "exit_code") == 1, "f's error in " + lab)
    for p in "bcdef":
        check(not any("_stderr" in l for l in fx("r9-rewind-tools-%s-%s" % (lab, p))) or p == "f", "no stderr line in %s-%s" % (lab, p))
    ROWS2.append((lab, asked))
b = "r9-rewind-tools-%s-b" % L2[0]
w("Process b (rewound to the interrupted call's line): every stdout line but stream events, and the transcript note after its exit:\n")
w(block(tl(b, lambda l: is_cli(l) and l.get("type") != "control_response") + [j(whole(b, "transcript"), maxlist=20)]))
e = "r9-rewind-tools-%s-e" % L2[0]
w("Process e (rewound to turn 2's user message): its note:\n")
w(block([j(one(e, "asked", "e"))]))
f = "r9-rewind-tools-%s-f" % L2[0]
w("Process f (the end of turn 3 again, after d and e): the stderr line and the result:\n")
w(block([j({"_stderr": l["_stderr"]}) for l in fx(f) if "_stderr" in l] + [j({k: l[k] for k in ("type", "subtype", "is_error", "num_turns", "errors")}) for l in results(f)]))
w("| label | b: end of turn 4 (interrupted) | c: end of turn 3 | d: end of turn 2 | e: turn 2's user message | f: end of turn 3 again |")
w("|---|---|---|---|---|---|")
for lab, asked in ROWS2:
    cell = lambda x: ", ".join(x["words_named"]) + ("; tools: " + ", ".join(x["tools_used"]) if x["tools_used"] else "")
    w("| %s | %s | %s | %s | %s | `%s` |" % (lab, cell(asked["b"]), cell(asked["c"]), cell(asked["d"]), cell(asked["e"]), asked["f"]["result_subtype"]))
L2D = labels("r9-rewind-drops-", "a")
check(L2D == L2, "the labels of the drops runs")
DROPS = []
for lab in L2D:
    bd, cd = ["r9-rewind-drops-%s-%s" % (lab, p) for p in "bc"]
    xb, xc = one(bd, "asked", "b"), one(cd, "asked", "c")
    check(xb["result_subtype"] == "error_during_execution" and not xb["init_came"] and xb["result_errors"][0].startswith("Resume rejected by --resume-drops-turn: "),
          "the refused drop in " + bd)
    check("range does not start with the declared turn prompt" in xb["result_errors"][0], "the reason of the refused drop in " + bd)
    check(xc["result_subtype"] == "success" and xc["words_named"] == ["lantern", "pebble"] and xc["init_session_id_is_the_first_session"], "the drop of one turn in " + cd)
    DROPS.append((xb, xc))
w("\n`--resume-drops-turn`: `r9-rewind-drops-<label>-b` (`--resume-session-at <end of turn 1> --resume-drops-turn <U3>`: the dropped part "
  "also holds turn 2) and `-c` (`--resume-session-at <end of turn 2> --resume-drops-turn <U3>`). Flags of `%s`: %s. The result of b:\n"
  % ("r9-rewind-drops-%s-b" % L2D[0], flags("r9-rewind-drops-%s-b" % L2D[0])))
w(block([j({k: l[k] for k in ("type", "subtype", "is_error", "num_turns", "errors")}, maxstr=400) for l in results("r9-rewind-drops-%s-b" % L2D[0])]))
w("""What the adapter can rely on (n=2):
- A rewind to the end of a turn with tool calls (d) or with a helper (c) works like part 6's rewind of text turns:
  the next turn starts (`init`, the answer, `result` `success`, the same session id) and knows only the kept turns
  (d: %s; c: %s).
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
""" % (", ".join(WORDS2[:2]), ", ".join(WORDS2[:3])))

# ================================================================= 3
w("## 3. Fork a conversation that has tool calls, and one whose first process is still alive\n")
w("Question: `--resume <id> --fork-session`, with and without `--resume-session-at`, of a session with tool calls and a helper; and a fork "
  "while the first process still runs with a background command: does the first go on untouched (its next turn, its command)?\n")
L3 = labels("r9-fork-tools-", "a")
check(len(L3) == 2 and labels("r9-fork-live-", "a") == L3, "two labels of section 3")
a = "r9-fork-tools-%s-a" % L3[0]
w("Fixtures: `r9-fork-tools-<label>-a` … `-d` and `r9-fork-live-<label>-a`, `-b`, labels %s. Flags of `-a`: %s. Process a: turn 1 "
  "(`lantern`) a Read, an Edit and a Bash call; turn 2 (`pebble`) a helper that runs a Bash call. b `--resume <a> --fork-session`; c the same "
  "with `--resume-session-at <the end of turn 1>`; d a plain `--resume <a>`. Each asks (no tools) for every secret word. `r9-fork-live`: "
  "process a starts a new session whose turn 1 (`harbor`) starts `echo bg-start; sleep 263; echo bg-end` in the background and ends; while "
  "a still runs, b is `--resume <a> --fork-session` with one turn; then a gets turn 2 (`anchor`) and a question for every word; `stop_task` "
  "ends the command.\n" % (", ".join(L3), flags(a)))
w("| label | turn 1, 2 of a: tools | b: words, new session?, `total_cost_usd` = a + own turn | c: words, new session? | d: words, same session? |")
w("|---|---|---|---|---|")
for lab in L3:
    a, b, c, d = ["r9-fork-tools-%s-%s" % (lab, p) for p in "abcd"]
    t = meta(a, "tools_used")
    check(len(t) == 2 and {"Read", "Edit", "Bash"} <= set(t[0]["tools_used"]) and "Agent" in t[1]["tools_used"] and t[1]["helper_lines"] > 0, "the tools of " + a)
    x = {p: one(n, "asked", p) for p, n in (("b", b), ("c", c), ("d", d))}
    check(x["b"]["words_named"] == ["lantern", "pebble"] and not x["b"]["init_session_id_is_the_first_session"] and not x["b"]["result_session_id_is_the_first_session"], "b of " + lab)
    check(x["c"]["words_named"] == ["lantern"] and not x["c"]["result_session_id_is_the_first_session"], "c of " + lab)
    check(x["d"]["words_named"] == ["lantern", "pebble"] and x["d"]["result_session_id_is_the_first_session"], "d of " + lab)
    first = x["b"]["first_sessions_last_total_cost_usd"]
    check(abs(x["b"]["total_cost_usd"] - (first + x["b"]["cost_of_this_turn"])) < 1e-9, "b's total carries a's in " + lab)
    w("| %s | %s; %s | %s, new, %.8f = %.8f + %.8f | %s, new | %s, the same |" % (
        lab, ", ".join(t[0]["tools_used"]), ", ".join(t[1]["tools_used"]), ", ".join(x["b"]["words_named"]), x["b"]["total_cost_usd"], first,
        x["b"]["cost_of_this_turn"], ", ".join(x["c"]["words_named"]), ", ".join(x["d"]["words_named"])))
LIVE = []
for lab in L3:
    a, b = ["r9-fork-live-%s-%s" % (lab, p) for p in "ab"]
    t1, af, t2, aa, st = (whole(a, "background_task_id"), whole(a, "after_the_fork"), one(a, "turn", 2), one(a, "asked", "a"), whole(a, "after_stop_task"))
    xb = one(b, "asked", "b")
    check(t1["turn_ok"] and t1["sleep_263_alive"] and len(t1["list"]) == 1, "the command runs before the fork in " + a)
    check(af["sleep_263_alive"] and af["list"] == t1["list"] and af["lines_while_the_fork_ran"] == [], "a untouched by the fork in " + a)
    check(t2["ok"] and t2["sleep_263_alive"] and t2["list"] == t1["list"] and t2["result_session_id_is_the_first_session"], "a's next turn in " + a)
    check(aa["words_named"] == ["harbor", "anchor"] and aa["results_with_no_model_request"] == 0, "a's words in " + a)
    check(not st["sleep_263_alive"] and st["list"] == [] and note(a, "left") == [], "stop_task ends the command in " + a)
    check(xb["results_with_no_model_request"] == 1 and xb["words_named"] == ["harbor"] and not xb["init_session_id_is_the_first_session"]
          and not xb["result_session_id_is_the_first_session"], "the fork's answer in " + b)
    exited = [m for m in meta(b, "sleep_263_alive") if set(m) == {"sleep_263_alive"}]  # the note after the fork's exit
    check(note(b, "task_lines_in_the_fork") == ["task_notification"] and whole(b, "task_lines_in_the_fork")["sleep_263_alive"]
          and len(exited) == 1 and exited[0]["sleep_263_alive"], "the command lives on after the fork in " + b)
    tn = [l for l in fx(b) if sub(l) == "task_notification"]
    check(len(tn) == 1 and tn[0]["status"] == "stopped" and tn[0]["summary"] == "Background shell command didn't finish before the previous session ended",
          "the fork's notification in " + b)
    empty = [r for r in results(b) if r["num_turns"] == 0]
    check(len(empty) == 1 and empty[0]["total_cost_usd"] == 0, "the fork's empty result in " + b)
    first_sent = next(i for i, l in enumerate(fx(b)) if "_sent" in l)
    check(first_sent < min(i for i, l in enumerate(fx(b)) if is_cli(l)), "the host's message is sent before the fork's first line in " + b)
    check(xb["first_sessions_last_total_cost_usd"] > 0 and abs(xb["total_cost_usd"] - xb["cost_of_this_turn"]) < 1e-9,
          "the live fork's total is its own turn only in " + b)
    LIVE.append((lab, xb))
b = "r9-fork-live-%s-b" % L3[0]
w("\nThe fork of the live session (`%s`): its first lines, up to the empty `result` (no model request: `num_turns` 0), the answer and the "
  "note:\n" % b)
w(block(tl(b, lambda l: sub(l) in ("task_notification", "init") or l.get("type") == "result" or (l.get("type") == "assistant" and any(x["type"] == "text" for x in l["message"]["content"])))
        + [j(one(b, "asked", "b"))]))
w("""What the adapter can rely on (n=2):
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
- The live fork's `total_cost_usd` is its own turn only (%s), not the first session's total
  (%s) plus its turn, unlike the fork of an ended session above: a host that adds up a
  session's cost must not read the fork's total as carrying the first session's.
- Not recorded: a fork while the first process's TURN runs; a fork of a session whose helper still runs in the
  background; a fork that gets no message at once; a fork of a session whose last turn was interrupted in a tool call
  (the forks here are of a new two-turn session, not of section 2's session); a plain `--resume` of a live session
  (the control for whether the `stopped` line belongs to the fork).
""" % (" / ".join("%.8f" % x["total_cost_usd"] for _, x in LIVE), " / ".join("%.8f" % x["first_sessions_last_total_cost_usd"] for _, x in LIVE)))

# ================================================================= 4
w("## 4. The CLI's own file rewind beyond the Edit tool\n")
w("Question: with `CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING=1`, `rewind_files` after a turn that wrote a file with Bash, made a new file "
  "with Write, and had a helper edit a file: which files go back; is the next turn told anything?\n")
L4 = labels("r9-file-rewind-more-")
check(len(L4) == 2, "two labels of section 4")
n4 = "r9-file-rewind-more-%s" % L4[0]
w("Fixtures: %s. Flags: %s, with the variable in the CLI's environment. The work folder has `notes.txt` (`notes`) and `README.md` (first "
  "line `# demo`). Turn 1 (host uuid U1): Bash writes `notes.txt` (`bash wrote this`), the Write tool makes `new.txt` (`new file`), a "
  "helper (foreground) edits `README.md`'s first line to `# demo edited by helper`. Then `rewind_files U1` as a dry run and for real; then "
  "turn 2 asks (no tools) whether anything told it that files were put back.\n" % (both(["r9-file-rewind-more-" + x for x in L4]), flags(n4)))
w("The two requests and answers of `%s`:\n" % n4)
lines4 = fx(n4)
qa = []
for i, l in enumerate(lines4):
    if "_sent" in l and l["_sent"].get("type") == "control_request" and l["_sent"]["request"]["subtype"] == "rewind_files":
        rid = l["_sent"]["request_id"]
        qa += [j(l["_sent"]), j(next(m for m in lines4[i + 1:] if m.get("type") == "control_response" and m["response"].get("request_id") == rid))]
w(block(qa))
w("| label | tools of turn 1 (helper's marked) | files after turn 1 | dry run: `filesChanged` | files after the rewind | transcript lines the rewind added | turn 2's answer |")
w("|---|---|---|---|---|---|---|")
for lab in L4:
    n = "r9-file-rewind-more-" + lab
    t1, dry, real, t2 = one(n, "turn", 1), one(n, "after", "rewind_files U1 dry run"), one(n, "after", "rewind_files U1"), one(n, "turn", 2)
    check(t1["ok"] and t1["agent_call_in_the_background"] == [False] and ["Edit", True] in t1["tools_used_and_by_a_helper"]
          and ["Bash", False] in t1["tools_used_and_by_a_helper"] and ["Write", False] in t1["tools_used_and_by_a_helper"], "the tools of turn 1 in " + n)
    after1 = {"notes.txt": "bash wrote this", "new.txt": "new file", "README.md": "# demo edited by helper"}
    check(t1["files"] == after1 and dry["files"] == after1, "the files after turn 1 and the dry run in " + n)
    check(real["files"] == {"notes.txt": "bash wrote this", "new.txt": None, "README.md": "# demo edited by helper"}, "the files after the rewind in " + n)
    pairs4 = []
    for i, l in enumerate(fx(n)):
        if "_sent" in l and l["_sent"].get("type") == "control_request" and l["_sent"]["request"]["subtype"] == "rewind_files":
            rid = l["_sent"]["request_id"]
            pairs4.append(next(m for m in fx(n)[i + 1:] if m.get("type") == "control_response" and m["response"].get("request_id") == rid)["response"]["response"])
    check(len(pairs4) == 2 and [os.path.basename(p) for p in pairs4[0]["filesChanged"]] == ["new.txt"] and pairs4[0]["deletions"] == 1 and pairs4[0]["insertions"] == 0
          and pairs4[1] == {"canRewind": True, "skippedLinks": 0}, "the answers of " + n)
    added = note(n, "transcript_types_added_by_the_rewind")
    check(added == [], "the rewind adds nothing to the transcript in " + n)
    check(t2["ok"] and t2["answer"].strip().lower().startswith("no") and t2["files"] == real["files"], "turn 2 of " + n)
    check(not any(x.startswith("attachment/") and "rewind" in x for x in t2["transcript_types_added_by_the_turn"]), "no rewind note in the transcript of " + n)
    tools = ", ".join(x[0] + (" (helper)" if x[1] else "") for x in t1["tools_used_and_by_a_helper"])
    w("| %s | %s | %s | `%s` | %s | %d | %s |" % (lab, tools, ", ".join("%s: `%s`" % kv for kv in t1["files"].items()),
                                               ", ".join(os.path.basename(p) for p in pairs4[0]["filesChanged"]),
                                               ", ".join("%s: %s" % (k, "`%s`" % v if v is not None else "gone") for k, v in real["files"].items()), len(added), t2["answer"]))
w("""
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
""")

# ================================================================= 5
w("## 5. Your own slash commands and skills\n")
w("Question: with a user command, a project command and a skill: what does `init` list; what does a user message \"/<command> <words>\" do "
  "in stream-json (does the command's prompt run, which lines mark it); an unknown \"/nope\"; a project command in a folder that is not "
  "trusted (`--setting-sources user`)?\n")
V5 = {v: labels("r9-own-commands-%s-" % v) for v in ("home-all", "home-user", "login")}
check(all(len(x) == 2 for x in V5.values()), "two recordings per variant of section 5")
SENT5 = ["/codly-hello world", "/codly-proj world", "/codly-skill", "/codly-proj-skill", "/nope world"]
n5 = "r9-own-commands-login-%s" % V5["login"][0]
w("Fixtures: `r9-own-commands-<variant>-<label>`. The scenario's files: a user command `HOME/.claude/commands/codly-hello.md` and a user "
  "skill `HOME/.claude/skills/codly-skill/SKILL.md` (throw-away HOME variants only), a project command `.claude/commands/codly-proj.md` "
  "and a project skill `.claude/skills/codly-proj-skill/SKILL.md` in the work folder. Each asks for one fixed reply (`hello says "
  "<words>`, `proj says <words>`, `skill ran`, `proj skill ran`). Variants: `home-all` (throw-away HOME, no login, `--setting-sources "
  "user,project,local`), `home-user` (the same, `--setting-sources user`), `login` (the real login, `--strict-mcp-config "
  "--setting-sources project`: only the work folder's files are a source). Flags of `%s`: %s. Messages, one turn each: %s.\n"
  % (n5, flags(n5), ", ".join("`%s`" % x for x in SENT5)))
w("`%s`: the echo of each message (`--replay-user-messages`) and the turn's result:\n" % n5)
w(block(tl(n5, lambda l: (l.get("type") == "user" and is_cli(l) and l.get("isReplay")) or l.get("type") == "result", text=160)))
w("| variant | `init.slash_commands` of the scenario | `init.skills` of the scenario | %s |" % " | ".join("`%s`" % x for x in SENT5))
w("|---|---|---|---|---|---|---|---|")
for v, labs in V5.items():
    for lab in labs:
        n = "r9-own-commands-%s-%s" % (v, lab)
        cmds, skills = note(n, "init_slash_commands_of_the_scenario"), note(n, "init_skills_of_the_scenario")
        want = {"home-all": (["codly-hello", "codly-proj", "codly-proj-skill", "codly-skill"], ["codly-proj-skill", "codly-skill"]),
                "home-user": (["codly-hello", "codly-skill"], ["codly-skill"]), "login": (["codly-proj", "codly-proj-skill"], ["codly-proj-skill"])}[v]
        check((cmds, skills) == want, "what init lists in " + n)
        cells = []
        for s in SENT5:
            m = one(n, "sent", s)
            check(m["ok"] and m["result_subtype"] == "success" and m["user_lines"][:1] == [{"isReplay": True, "isSynthetic": None, "content_kind": "text"}], "the turn of %s in %s" % (s, n))
            if v != "login":
                check(m["result"] == "Not logged in · Please run /login", "no login in " + n)
                cells.append("`Not logged in`")
            else:
                cells.append(m["result"][:60].replace("\n", " ") + ("…" if len(m["result"]) > 60 else ""))
        if v == "login":
            check(one(n, "sent", "/codly-proj world")["result"] == "proj says world" and one(n, "sent", "/codly-proj-skill")["result"] == "proj skill ran", "the project's command and skill ran in " + n)
            for s in ("/codly-hello world", "/codly-skill", "/nope world"):
                check("isn't installed" in one(n, "sent", s)["result"] or "not installed" in one(n, "sent", s)["result"], "the model got the unknown command as text in " + n)
            replay = [l for l in fx(n) if l.get("type") == "user" and is_cli(l) and l.get("isReplay")]
            check([l["message"]["content"] for l in replay] == ["/codly-hello world",
                   "<command-message>codly-proj</command-message>\n<command-name>/codly-proj</command-name>\n<command-args>world</command-args>", "/codly-skill",
                   "<command-message>codly-proj-skill</command-message>\n<command-name>/codly-proj-skill</command-name>", "/nope world"], "the echoes of " + n)
        w("| `%s` | %s | %s | %s |" % (n[len("r9-own-commands-"):], ", ".join("`%s`" % x for x in cmds), ", ".join("`%s`" % x for x in skills), " | ".join(cells)))
w("""
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
""")

# ================================================================= 6
w("## 6. A turn with no network\n")
w("Question: a turn with the API unreachable (a proxy variable that points to a closed local port; a host name that does not resolve): "
  "which lines come, how long until the turn ends, the error text and the exit code, and what a turn does when the network is back.\n")
L6P, L6D = labels("r9-no-network-proxy-"), labels("r9-no-network-dns-", "a")
check(len(L6P) == 2 and len(L6D) == 2, "two recordings per variant of section 6")
n6 = "r9-no-network-proxy-" + L6P[0]
w("Fixtures: %s (`HTTPS_PROXY`, `HTTP_PROXY` and their lower-case spellings point to a port on 127.0.0.1 that the script holds bound but "
  "not listening, so every connection is refused; turn 1 runs; then the script makes that port a CONNECT proxy that only copies bytes, and "
  "turn 2 runs in the same process) and `r9-no-network-dns-<label>-a`, `-b` (`ANTHROPIC_BASE_URL=https://api.no-such-host.invalid`, a name "
  "that never resolves; turn 1 runs; process b resumes the session with no such variable). Flags of `%s`: %s.\n"
  % (both(["r9-no-network-proxy-" + x for x in L6P]), n6, flags(n6)))
w("`%s`, turn 1: every line but stream events, from the host's message to the result:\n" % n6)
w(block(tl(n6, lambda l: is_cli(l) and sub(l) not in ("init", "status"), stop=lambda l: l.get("type") == "result", text=120)))


def retries(m):
    return [x for x in m["system_lines"] if x["subtype"] == "api_retry"]


w("| recording | turn 1: `api_retry` lines (attempt: s after the message, `retry_delay_ms`) | turn 1: result after | its text, `is_error`, `terminal_reason` | turn 2: result after, text |")
w("|---|---|---|---|---|")
ROWS6 = []
for kind_, labs in (("proxy", L6P), ("dns", L6D)):
    for lab in labs:
        if kind_ == "proxy":
            n1 = n2 = "r9-no-network-proxy-" + lab
        else:
            n1, n2 = "r9-no-network-dns-%s-a" % lab, "r9-no-network-dns-%s-b" % lab
        t1, t2 = one(n1, "turn", 1), one(n2, "turn", 2)
        r1 = [r for r in results(n1)][0]
        rt = retries(t1)
        check(t1["result_is_error"] and t1["result_subtype"] == "success" and r1["terminal_reason"] == "api_error" and t1["stderr_lines"] == 0, "turn 1 fails in " + n1)
        check([x["attempt"] for x in rt] == list(range(1, len(rt) + 1)) and all(x["max_retries"] == 10 and x["error_status"] is None for x in rt), "the retry lines of " + n1)
        check(not t2["result_is_error"] and t2["result"].strip().lower().startswith("pong") and t2["system_lines"] == [], "turn 2 works in " + n2)
        ROWS6.append((kind_, lab, n1, t1, rt, r1, t2))
        w("| `%s` | %s | %.1f s | `%s`, %s, `%s` | %.2f s, `%s` |" % (
            n1, "; ".join("%d: %.1f, %d" % (x["attempt"], x["_seconds"], x["retry_delay_ms"]) for x in rt) or "none", t1["seconds_to_the_result"],
            t1["result"], str(t1["result_is_error"]).lower(), r1["terminal_reason"], t2["seconds_to_the_result"], t2["result"]))
PX = [r for r in ROWS6 if r[0] == "proxy"]
DN = [r for r in ROWS6 if r[0] == "dns"]
check(all(r[3]["result"] == "Request timed out" and len(r[4]) == 10 for r in PX), "the proxy runs' error text and ten retries")
for r in PX:
    check(note(r[2], "proxy_connects") >= 1 and note(r[2], "exit_code") == 0, "the proxy carried turn 2 of " + r[2])
DNS_TEXT = sorted({r[3]["result"] for r in DN})
DNS_RETRIES = sorted({len(r[4]) for r in DN})
ATTEMPTS6 = sorted({x["attempt"] for r in ROWS6 for x in r[4]})
WAITS6 = ", ".join("%d: %.1f-%.1f s" % (a, min(x["retry_delay_ms"] for r in ROWS6 for x in r[4] if x["attempt"] == a) / 1000.0,
                                         max(x["retry_delay_ms"] for r in ROWS6 for x in r[4] if x["attempt"] == a) / 1000.0) for a in ATTEMPTS6)
synthetic = [l for l in fx(PX[0][2]) if l.get("type") == "assistant" and l["message"].get("model") == "<synthetic>"]
check(len(synthetic) == 1 and synthetic[0].get("error") == "server_error" and synthetic[0].get("is_api_error_message"), "the synthetic assistant line of " + PX[0][2])
w("""
What the adapter can rely on (n=2 per way):
- With the service unreachable the turn does not fail at once: the CLI retries. Each try is a line
  `{"type":"system","subtype":"api_retry","attempt":N,"max_retries":10,"retry_delay_ms":…,"error_status":null,"error":"unknown"}`.
  A refused proxy gave all 10 tries, and the turn's `result` came %s s after the message. The waits
  (`retry_delay_ms`, lowest and highest per attempt over the %d runs of both ways) grow:
  %s.
  Through the refused proxy each try itself also took some seconds
  (the first retry line came %s s after the message); for the name that does not resolve it failed at once (%s s).
- The end of the turn: an assistant line with `"model":"<synthetic>"`, `"error":"server_error"`,
  `"is_api_error_message":true` and the text, then the `result` with `"subtype":"success"` (not an error subtype),
  `"is_error":true`, `"terminal_reason":"api_error"`, `api_error_status` null and `result` = the same text. The text
  is `Request timed out` for the refused proxy (n=2): it does not say "no network". For the name that does not
  resolve: %s, after %s tries, %s s after the message. Nothing on stderr. The process stays up and takes the next
  message; when stdin was closed after the failed turn (the `dns` runs' process a) it exited with code %s.
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
""" % (" / ".join("%.1f" % r[3]["seconds_to_the_result"] for r in PX), len(ROWS6), WAITS6, " / ".join("%.1f" % r[4][0]["_seconds"] for r in PX),
       " / ".join("%.1f" % r[4][0]["_seconds"] for r in DN), " / ".join("`%s`" % x for x in DNS_TEXT), " / ".join(str(x) for x in DNS_RETRIES),
       " / ".join("%.1f" % r[3]["seconds_to_the_result"] for r in DN), " / ".join(sorted({str(note(r[2], "exit_code")) for r in DN}))))

# ================================================================= 7
w("## 7. A conversation started in the terminal\n")
w("Question: where are a folder's sessions kept (names and counts only)? Does `--resume <id>` in stream-json of a session that an "
  "interactive `claude` started work (`init`, the first turn, what it knows)? What does `claude --resume` with an unknown id say?\n")
L7 = labels("r9-terminal-", "a")
check(len(L7) == 2 and all("r9-terminal-%s-%s" % (lab, p) in ALL for lab in L7 for p in "bc"), "two labels of section 7")
w("Fixtures: `r9-terminal-<label>-a` (notes only: the interactive `claude --model haiku --strict-mcp-config --setting-sources local` in "
  "a pseudo-terminal, one message \"remember the word meadow\", then `/exit`), `-b` (stream-json `--resume <that session>`: one turn "
  "asks for the word; flags of `r9-terminal-%s-b`: %s) and `-c` (notes only: the interactive `claude --resume <a uuid no session has>`), "
  "labels %s. Label %s ran in a folder where an earlier try of the same script had made one session and had trusted the folder (so its "
  "\"before\" list has one file and it was not asked again); label %s ran in a new folder.\n"
  % (L7[0], flags("r9-terminal-%s-b" % L7[0]), ", ".join(L7), L7[0], L7[1]))
for lab in L7:
    w("The note of `r9-terminal-%s-a`:\n" % lab)
    w(block([j(whole("r9-terminal-%s-a" % lab, "session_files_after"), maxlist=40)]))
w("| label | asked to trust the folder | sessions folder (under the HOME) | session files before / after | `entrypoint` | b: init and result in that session, words | c: exit code, text |")
w("|---|---|---|---|---|---|---|")
for lab in L7:
    a, b, c = ["r9-terminal-%s-%s" % (lab, p) for p in "abc"]
    m = whole(a, "session_files_after")
    check(m["exit_code"] == 0 and m["session_found"] and m["entrypoint"] == "cli" and len(m["session_files_after"]) == len(m["session_files_before"]) + 1, "the terminal session of " + a)
    check(m["sessions_folder"] == ".claude/projects/-tmp-work-r9-terminal-" + lab, "the sessions folder of " + a)
    check("user" in m["transcript_types"] and "assistant" in m["transcript_types"], "the transcript of " + a)
    x = whole(b, "words_named")
    check(x["ok"] and x["words_named"] == ["meadow"] and x["init_session_id_is_the_terminal_session"] and x["result_session_id_is_the_terminal_session"]
          and x["result_subtype"] == "success", "the stream-json resume of " + b)
    y = whole(c, "printed")
    check(y["exit_code"] == 1 and y["printed"] == "No conversation found with session ID: sess-1", "the unknown id of " + c)
    w("| %s | %s | `~/%s` | %d / %d | `%s` | yes, yes, %s | %d, `%s` |" % (lab, str(m["asked_whether_to_trust_the_folder"]).lower(), m["sessions_folder"],
                                                              len(m["session_files_before"]), len(m["session_files_after"]), m["entrypoint"],
                                                              ", ".join(x["words_named"]), y["exit_code"], y["printed"]))
EARLIER7 = [n for n in NAMES if n.startswith("r5-resume-missing-")]
check(EARLIER7 and all(any(l.get("_stderr") == "No conversation found with session ID: sess-1" for l in fx(n)) for n in EARLIER7), "part 3's words for a missing session")
w("""
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
""")

# ================================================================= 8
w("## 8. A slow tool of a streaming HTTP MCP server that sends nothing for more than 5 minutes\n")
w("Question: a tool that sends nothing for 330 s on an HTTP server that answers with an event stream: when and how does the call end, do "
  "the 30 s heartbeat lines go on until then? One run is enough for this question (n=1).\n")
L8 = labels("r9-http-silent-")
check(len(L8) == 1, "one recording of section 8")
n8 = "r9-http-silent-" + L8[0]
m8 = whole(n8, "tool_progress_count")
w("Fixture: `%s`. Flags: %s. The server `ping-http` (`mcp_http_ping_server.py`, answer kind `sse`) sends the event stream's headers at once "
  "and then nothing until 330 s.\n" % (n8, flags(n8)))
w("The tool's result line and the note:\n")
w(block(tl(n8, lambda l: l.get("type") == "user" and is_cli(l), text=400) + [j(m8, maxlist=12)]))
call = [h for h in m8["server_requests"] if h.get("jsonrpc_method") == "tools/call"]
check(m8["tool_progress_count"] == 10 and [round(x) for x in m8["seconds_from_tool_use_to_each_tool_progress"]] == list(range(30, 301, 30)), "the heartbeats of " + n8)
check(299.0 < m8["seconds_from_tool_use_to_tool_result"] < 302.0 and m8["tool_results"][0]["is_error"], "the call ends at 300 s in " + n8)
check(m8["tool_results"][0]["content"].startswith('MCP server "ping-http" tool "ping" sent no response or progress for 300s; aborting.'), "the words of " + n8)
check(len(call) == 1 and "answered_at" not in call[0] and "answer_failed" not in call[0] and call[0]["progress_token_in_request"], "the server's view of " + n8)
r8 = results(n8)
check(len(r8) == 1 and r8[0]["subtype"] == "success", "the turn goes on after the error in " + n8)
w("""What the adapter can rely on (n=1):
- The CLI ends such a call itself at 300 s of silence (%.1f s after the `tool_use` line): the tool result is an error,
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
""" % m8["seconds_from_tool_use_to_tool_result"])

# ================================================================= late notes
w("## Notes written late\n")
w("Each note of this part that lists lines is compared with the lines in the fixture between the step the script marked just before it "
  "(named in the last column) and the note:\n")


def kindline(l):
    return is_cli(l) and l.get("type") not in ("stream_event", "rate_limit_event") and l.get("subtype") not in ("thinking_tokens", "commands_changed", "status")


def last_user(l):
    return "_sent" in l and l["_sent"].get("type") == "user"


SINCE = {"lines": ("the host's last message", last_user),
         "lines_while_the_fork_ran": ("the note with `background_task_id`", lambda l: "_meta" in l and "background_task_id" in l["_meta"])}
w("| fixture | note | lines in its list | lines in the file before it | counted from |")
w("|---|---|---|---|---|")
LATE, CHECKED = [], 0
for n in ALL:
    lines = fx(n)
    for i, l in enumerate(lines):
        m = l.get("_meta") or {}
        for key in m:
            if not key.startswith("lines") or not isinstance(m[key], list):
                continue
            check(key in SINCE, "a starting point for the note %r of %s" % (key, n))
            label, at = SINCE[key]
            starts = [x for x, y in enumerate(lines[:i]) if at(y)]
            # a resumed process's note `lines` lists every line since its start
            since = starts[-1] if starts and not (key == "lines" and "asked" in m and "rewind" in n) else 0
            seen = sum(1 for x in lines[since:i] if kindline(x))
            CHECKED += 1
            if seen != len(m[key]):
                LATE.append((n, key))
            w("| `%s` | `%s` | %d | %d | %s |" % (n, key, len(m[key]), seen, label if since else "the process's start"))
check(LATE == [], "no late note in part 7: %r" % LATE)
w("""
All %d notes of this part that list lines list exactly the lines in their window: none is late.
""" % CHECKED)

# ================================================================= code
w("""## Can the waiting code be built now? (as of the day of recording)

| # | the code that waits | can it be built now? |
|---|---|---|
| 1 | `claude-part5-truth` item 1 (`claude.rs` `build_args`, `orchestrator/trust.rs`): drop `--strict-mcp-config` in a folder you have not trusted when "Use my Claude setup" is on | yes for `--setting-sources user` alone: no file of the folder started or loaded anything in %d recordings of parts 4 to 7, and a plugin a repo file names but the user never installed is not loaded (n=%d). NO for `user,local`: the folder's `.claude/settings.local.json` runs the hooks and the server of a plugin the user turned off (n=%d), and part 4 saw that file's own hooks run. Never pass `local` in an untrusted folder |
| 2 | "Undo a turn" for the conversation (`checkpoints.rs`, `src/home/rewind.ts`) | yes: `--resume <id> --resume-session-at <uuid of the LAST complete assistant line of the last turn to keep>` works after tool calls, a helper and an interrupted turn (n=2 each), with no line about a cut call. Never a stream event's uuid; not a user message's uuid (the next turn redoes that message). Rewind only to points of the latest branch; `--resume-drops-turn <U>` can guard that exactly one turn goes |
| 3 | "Try again from here" / "branch this agent" | yes: as part 6, also with tool calls and a helper (n=2). A fork of a LIVE session leaves the first process untouched, but the fork itself reports the first's background command as `stopped` and sends an empty `result` before its answer to the host's first message (n=2): the adapter must ignore both. Not recorded: a fork of a session whose last turn was interrupted |
| 4 | Codly's restore note (`with_restore_note`, `checkpoints.rs`) vs the CLI's own rewind | no: `rewind_files` puts back only the main thread's file tools (a Write's new file deleted; part 6: an Edit), not Bash's writes and not a helper's edit, and tells the agent nothing (n=2). Keep Codly's restore and its note |
| 5 | The composer's "/" menu (`composerMenu.ts`) and `slash_commands` / `skills` of `init` | yes: `init.slash_commands` lists the user's and the project's commands and skills for the sources on (n=2 per variant); a listed command's prompt runs, echoed as `<command-message>…`; an unknown "/" text goes to the model as plain text (n=2): it gives a model turn, not an error. The menu can offer what `init` lists |
| 6 | The failure words for "cannot reach the service" (`claude.rs`) | yes for these two forms (a machine with no network at all, and a failed lookup of the default host, are not recorded): up to 10 `api_retry` lines with `error_status` null, then `terminal_reason` `api_error`, `is_error` true, `api_error_status` null, text `Request timed out` (refused proxy, n=2, about 4.6 minutes) or %s (name that does not resolve, n=2, about 3 minutes); the next turn works when the network is back (n=2 per way) |
| 7 | A list of an agent's earlier conversations; "pick up what I started in my terminal" | yes: `~/.claude/projects/<folder path with '-'>/<session id>.jsonl`; stream-json `--resume <id>` of a terminal session works and knows its turns (n=2) |
| 8 | The watchdog's limits for HTTP tools (`watchdog.rs`) | yes (n=1): a silent HTTP tool ends at 300 s with the CLI's own error naming `timeout` and `CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT`; the 30 s heartbeats go on until then |
""" % (N_USER_ALONE, N_NEVER, N_REPO_ON, " / ".join("`%s`" % x for x in DNS_TEXT)))

# ================================================================= table
w("""## Table: question -> answer -> fixture

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
""")

text = "\n".join(P) + "\n"
dest = sys.argv[1] if len(sys.argv) > 1 else os.path.join(OUT, "NOTES-part7.md")
with open(dest, "w") as f:
    f.write(text)
print("wrote", dest, len(text), "characters")
