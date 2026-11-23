// Codly's check runs in words: a task's checks on its sheet and on its Done card, and the result
// of "Check the work" (an agent's Git tab), which runs the repo's required checks.

import type { CheckResult } from '../gen/CheckResult';
import type { Seg, TaskMark } from '../state/store.svelte';

/** One check of a task, as its sheet shows it. */
export type TaskCheck = {
  cmd: string;
  state: 'running' | 'pass' | 'fail' | 'none' | 'wait';
  detail: string;
  /** Who ran it last: Codly (the gate), or the task's agent in its own turn. */
  by: 'codly' | 'agent' | null;
  /** Yours: a required check of the repo (`.codly/checks`), not one of the task's own. */
  required: boolean;
  /** When the agent's run of it started, while it runs. */
  since?: number;
};

/** A task's checks: first its own verify commands (the Lead wrote them), then the repo's required
 *  checks that Codly ran for it (yours, `.codly/checks`).
 *  A run of Codly belongs to the command with the same text, and to no other: `cargo test` is not
 *  `cargo test -p core`. A required check counts only when it ran after the task's own checks last
 *  did (the gate runs them in that order): an older one is from a pass before, and may be a
 *  command you took out of the file since.
 *  `marks`: the task's finished steps, oldest first. `segs`: its agent's own command runs, oldest
 *  first. `waits`: does this call wait for your approval? */
export function taskChecks(verify: string[], marks: TaskMark[], segs: Seg[], waits: (callId: string) => boolean): TaskCheck[] {
  const own = [...new Set(verify)];
  const last = new Map<string, TaskMark>();
  for (const m of marks) if (m.kind === 'verified') last.set(m.text, m);
  const codly = (m: TaskMark, required: boolean): TaskCheck => ({
    cmd: m.text, state: m.ok ? 'pass' : 'fail', detail: `${m.ok ? 'passed' : 'failed'} · Codly${required ? ' · required by you' : ''}`, by: 'codly', required,
  });

  const rows = own.map((cmd): TaskCheck => {
    const m = last.get(cmd);
    if (m) return codly(m, false);
    // The agent's own run of it may be wrapped (`cd app && pnpm test`): any command that holds it.
    const s = [...segs].reverse().find((x) => (x.target.command ?? '').includes(cmd) || cmd.includes(x.target.command ?? '\u0000'));
    if (!s) return { cmd, state: 'none', detail: 'not run yet', by: null, required: false };
    if (s.end == null && waits(s.callId)) return { cmd, state: 'wait', detail: 'needs your approval', by: 'agent', required: false };
    if (s.end == null) return { cmd, state: 'running', detail: 'running now', by: 'agent', required: false, since: s.start };
    const counts = s.testsPassed != null ? (s.testsFailed ? `${s.testsFailed} failed, ${s.testsPassed} passed` : `${s.testsPassed} passed`) : s.ok ? 'passed' : 'failed';
    return { cmd, state: s.ok === false || s.testsFailed ? 'fail' : 'pass', detail: `${counts} · its run`, by: 'agent', required: false };
  });

  const ownRan = Math.max(0, ...own.map((cmd) => last.get(cmd)?.eventId ?? 0));
  const required = [...last.values()].filter((m) => !own.includes(m.text) && m.eventId > ownRan).sort((a, b) => a.eventId - b.eventId);
  return [...rows, ...required.map((m) => codly(m, true))];
}

/** Did Codly's own run of every check pass? Then the task's acceptance criteria are ticked. */
export function checksPassed(checks: TaskCheck[]): boolean {
  return checks.length > 0 && checks.every((c) => c.state === 'pass' && c.by === 'codly');
}

/** Codly's check runs on the card of a finished task: the task's own check to show (one that
 *  failed, else the last one Codly ran), and your required checks of the same pass.
 *  `marks`: the task's finished steps, oldest first. */
export function doneChecks(verify: string[], marks: TaskMark[]): { own: TaskCheck | null; required: TaskCheck[] } {
  const ran = taskChecks(verify, marks, [], () => false).filter((c) => c.by === 'codly');
  const own = ran.filter((c) => !c.required);
  return { own: own.find((c) => c.state === 'fail') ?? own.at(-1) ?? null, required: ran.filter((c) => c.required) };
}

/** A task's required checks in one line: "Required by you: 2 checks passed", "Required by you: 1
 *  of 2 checks failed". */
export function requiredLine(checks: TaskCheck[]): string {
  const all = `${checks.length} ${checks.length === 1 ? 'check' : 'checks'}`;
  const failed = checks.filter((c) => c.state === 'fail').length;
  return `Required by you: ${failed ? `${failed} of ${all} failed` : `${all} passed`}`;
}

/** How one check ended: "passed", "failed (exit 1)", "ran out of time", "did not finish". */
export function checkLine(r: CheckResult): string {
  if (r.ok) return 'passed';
  if (r.timed_out) return 'ran out of time';
  return r.exit_code === null ? 'did not finish' : `failed (exit ${r.exit_code})`;
}

/** All of them in one line: "2 required checks passed", "1 of 2 required checks failed". */
export function checksLine(results: CheckResult[]): string {
  const checks = `${results.length} required ${results.length === 1 ? 'check' : 'checks'}`;
  const failed = results.filter((r) => !r.ok).length;
  return failed ? `${failed} of ${checks} failed` : `${checks} passed`;
}
