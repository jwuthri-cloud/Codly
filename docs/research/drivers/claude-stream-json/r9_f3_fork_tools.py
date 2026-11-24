"""F3: fork a conversation that has tool calls and a helper, and fork one whose first process is still alive with a
background command (`--resume <id> --fork-session`).

Session 1 (r9-fork-tools-<label>-a … -d), `--tools Bash,Read,Edit,Agent`, mode acceptEdits, every request allowed.
Process a: two turns with host uuids: turn 1 (`lantern`) an Edit of hello.txt and a Bash call; turn 2 (`pebble`) a
           helper (Agent) that runs a Bash call.
  b  `--resume <a> --fork-session`
  c  `--resume <a> --fork-session --resume-session-at <the last assistant line of turn 1>`
  d  `--resume <a>` (no fork): does the first session answer as before?
Each asks for every secret word (no tools); a note: the words, whether the session id is a's, the costs.

Session 2 (r9-fork-live-<label>-a, -b): process a starts a new session; turn 1 (`harbor`) starts the Bash command
`echo bg-start; sleep 263; echo bg-end` IN THE BACKGROUND and ends. While a still runs (and its command):
  b  `--resume <a> --fork-session`: one turn asks for every secret word; then b's stdin is closed.
Then, in process a: is the command alive, what does its task list say; turn 2 (`anchor`); turn 3 asks for every
secret word; `stop_task` with the command's task id; stdin closed. What is left of `sleep 263` is killed at the end.

argv[1]: a label for the recording (1, 2): the whole scenario, recorded more than once.
"""
import sys, time, uuid
from drv import Session
from r5_lib import Procs, ask, next_turn, open_tasks, results, task_started
from r6_lib import kinds, wait_alive
from r8_lib import assistant_uuids, last_result, transcript, words_in
LABEL = sys.argv[1] if len(sys.argv) > 1 else "1"
WORDS = ("lantern", "pebble", "harbor", "anchor")
ASK = ("Without using any tool: list every secret word I asked you to remember in this conversation, in one line. "
       "If there is none, reply with the single word: none")
TOOLS = ["--tools", "Bash,Read,Edit,Agent"]


def costs(s, sid):
    r = last_result(s)
    own = sum(m.get("costUSD", 0.0) for m in (r.get("model_usage_since_last_result") or {}).values())
    return dict(total_cost_usd=r.get("total_cost_usd"), cost_of_this_turn=own, result_session_id_is_the_first_session=r.get("session_id") == sid)


def ask_words(s, part, sid, first_total):
    m = s.mark()
    ok = next_turn(s, ASK, 60)
    # A fork of a live session first answers a `result` with no model request (num_turns 0) for a task notification
    # it makes itself (r9-fork-live): the answer to the question is the next result.
    with s.cv:
        empty = [e for e in s.events[m:] if e.get("type") == "result" and e.get("num_turns") == 0]
    if empty:
        ok = s.wait_results(results(s) + 1, 60)
    init = s.wait_for(lambda e: e.get("type") == "system" and e.get("subtype") == "init", 1, 0) or {}
    r = last_result(s, m)
    s.note(asked=part, ok=ok, results_with_no_model_request=len(empty), words_named=words_in(r.get("result"), WORDS),
           init_session_id_is_the_first_session=init.get("session_id") == sid, first_sessions_last_total_cost_usd=first_total, **costs(s, sid))
    print(part, "words:", words_in(r.get("result"), WORDS), "same session:", r.get("session_id") == sid)


# ---- session 1
NAME = "r9-fork-tools-" + LABEL
a = Session(NAME + "-a", repo=NAME, extra=TOOLS, hard_timeout=240)
with open(a.cwd + "/hello.txt", "w") as f:
    f.write("hello\n")
sid, ends = a.sid, []
for text in ("Remember this secret word: lantern. Read hello.txt, then use the Edit tool to change its text from `hello` to `hello lantern`, "
             "then run the Bash command `echo lantern-ok`. Then reply with the single word: done",
             "Remember this secret word: pebble. Use the Agent tool once: ask a helper to run the Bash command `echo pebble-helper` and to "
             "report its output. Then reply with the single word: done"):
    m = a.mark()
    ok = next_turn(a, text, 120, uuid=str(uuid.uuid4()))
    uu = assistant_uuids(a, m)
    ends.append(uu[-1] if uu else None)
    with a.cv:
        used = [b.get("name") for e in a.events[m:] if e.get("type") == "assistant" for b in e["message"]["content"] if b.get("type") == "tool_use"]
    a.note(turn_ok=ok, tools_used=used, helper_lines=a.count(lambda e: e.get("parent_tool_use_id") is not None and e.get("type") in ("assistant", "user")))
first_total = a.cost()
a.close(after_exit=lambda: dict(transcript=transcript(sid, WORDS)))
if None in ends:
    raise SystemExit("a turn had no assistant line: record again")
for part, extra in (("b", ["--fork-session"]), ("c", ["--fork-session", "--resume-session-at", ends[0]]), ("d", [])):
    s = Session("%s-%s" % (NAME, part), repo=NAME, extra=TOOLS + ["--resume", sid] + extra, session_id=None, hard_timeout=120)
    if part == "c":
        s.note(resume_session_at="the uuid of the last assistant line of turn 1 of process a")
    ask_words(s, part, sid, first_total)
    s.close(after_exit=lambda: dict(transcript=transcript(sid, WORDS)))

# ---- session 2
NAME2 = "r9-fork-live-" + LABEL
sleeper = Procs("sleep 263")
a = Session(NAME2 + "-a", repo=NAME2, extra=["--tools", "Bash"], hard_timeout=300)
sid2 = a.sid
ok = next_turn(a, "Remember this secret word: harbor. Run the Bash command `echo bg-start; sleep 263; echo bg-end` IN THE BACKGROUND "
                  "(run_in_background: true), do not wait for it, and reply with the single word: started", 90)
task = task_started(a, 0, 5)
alive = wait_alive(sleeper, 20)
a.note(turn_ok=ok, background_task_id=(task or {}).get("task_id"), sleep_263_alive=alive, list=open_tasks(a))
m_fork = a.mark()
b = Session(NAME2 + "-b", repo=NAME2, extra=["--tools", "Bash", "--resume", sid2, "--fork-session"], session_id=None, hard_timeout=120)
ask_words(b, "b", sid2, a.cost())
with b.cv:
    b_tasks = [e.get("subtype") for e in b.events if e.get("type") == "system" and e.get("subtype") in ("task_started", "task_notification", "background_tasks_changed")]
b.note(task_lines_in_the_fork=b_tasks, sleep_263_alive=bool(sleeper.alive()))
b.close(after_exit=lambda: dict(sleep_263_alive=bool(sleeper.alive())))
a.note(after_the_fork=True, sleep_263_alive=bool(sleeper.alive()), list=open_tasks(a), lines_while_the_fork_ran=kinds(a, m_fork))
m = a.mark()
ok = next_turn(a, "Remember this secret word: anchor. Reply with the single word: noted", 60)
a.note(turn=2, ok=ok, sleep_263_alive=bool(sleeper.alive()), list=open_tasks(a), result_session_id_is_the_first_session=last_result(a, m).get("session_id") == sid2)
ask_words(a, "a", sid2, None)
if task:
    ask(a, "stop_task", task_id=task["task_id"])
    time.sleep(1.0)
a.note(after_stop_task=True, sleep_263_alive=bool(sleeper.alive()), list=open_tasks(a))
a.close(after_exit=lambda: dict(transcript=transcript(sid2, WORDS), left=sleeper.kill_left()))
