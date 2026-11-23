// Codly's check runs in words: a task's checks on its sheet, and the result of "Check the work"
// (an agent's Git tab), which runs the repo's required checks.

import type { CheckResult } from '../gen/CheckResult';
import type { JsonValue } from '../gen/serde_json/JsonValue';
import type { Decision, Seg, TaskMark } from '../state/store.svelte';

/** One check of a task, as its sheet shows it. */
export type TaskCheck = {
  cmd: string;
  state: 'running' | 'pass' | 'fail' | 'none' | 'wait';
  detail: string;
  /** Who ran it last: Codly (the gate), or the task's agent in its own turn. */
  by: 'codly' | 'agent' | null;
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
  const codly = (m: TaskMark, who: string): TaskCheck => ({ cmd: m.text, state: m.ok ? 'pass' : 'fail', detail: `${m.ok ? 'passed' : 'failed'} · ${who}`, by: 'codly' });

  const rows = own.map((cmd): TaskCheck => {
    const m = last.get(cmd);
    if (m) return codly(m, 'Codly');
    // The agent's own run of it may be wrapped (`cd app && pnpm test`): any command that holds it.
    const s = [...segs].reverse().find((x) => (x.target.command ?? '').includes(cmd) || cmd.includes(x.target.command ?? '\u0000'));
    if (!s) return { cmd, state: 'none', detail: 'not run yet', by: null };
    if (s.end == null && waits(s.callId)) return { cmd, state: 'wait', detail: 'needs your approval', by: 'agent' };
    if (s.end == null) return { cmd, state: 'running', detail: 'running now', by: 'agent', since: s.start };
    const counts = s.testsPassed != null ? (s.testsFailed ? `${s.testsFailed} failed, ${s.testsPassed} passed` : `${s.testsPassed} passed`) : s.ok ? 'passed' : 'failed';
    return { cmd, state: s.ok === false || s.testsFailed ? 'fail' : 'pass', detail: `${counts} · its run`, by: 'agent' };
  });

  const ownRan = Math.max(0, ...own.map((cmd) => last.get(cmd)?.eventId ?? 0));
  const required = [...last.values()].filter((m) => !own.includes(m.text) && m.eventId > ownRan).sort((a, b) => a.eventId - b.eventId);
  return [...rows, ...required.map((m) => codly(m, 'Codly · required by you'))];
}

/** Did Codly's own run of every check pass? Then the task's acceptance criteria are ticked. */
export function checksPassed(checks: TaskCheck[]): boolean {
  return checks.length > 0 && checks.every((c) => c.state === 'pass' && c.by === 'codly');
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

/** One `CheckResult` of a decision's detail; nothing for a value that is not one. */
function readResult(c: JsonValue): CheckResult[] {
  if (!c || typeof c !== 'object' || Array.isArray(c)) return [];
  const { command, ok, exit_code, timed_out, tail } = c;
  if (typeof command !== 'string' || typeof ok !== 'boolean' || typeof timed_out !== 'boolean' || typeof tail !== 'string') return [];
  if (exit_code !== null && typeof exit_code !== 'number') return [];
  return [{ command, ok, exit_code, timed_out, tail }];
}

/** The last "Check the work" of an agent: each command's result, and when Codly ran them. Read
 *  from the `gate` decision the core records for it (`check_agent`), so the Git tab still has it
 *  when you come back.
 *  Null: Codly did not check this work, or the agent got a turn since (`turnStartedAt`, ms): that
 *  result is about files it may have changed. */
export function lastAgentCheck(decisions: Decision[], agentId: string, turnStartedAt: number | null): { results: CheckResult[]; ts: number } | null {
  let found: { ts: number; eventId: number; checks: JsonValue[] } | null = null;
  for (const d of decisions) {
    if (d.type !== 'gate' || !d.detail || typeof d.detail !== 'object' || Array.isArray(d.detail)) continue;
    const { agent_id, checks } = d.detail;
    if (agent_id === agentId && Array.isArray(checks) && (!found || d.eventId > found.eventId)) found = { ts: d.ts, eventId: d.eventId, checks };
  }
  if (!found || (turnStartedAt !== null && turnStartedAt > found.ts)) return null;
  const results = found.checks.flatMap(readResult);
  return results.length === found.checks.length ? { results, ts: found.ts } : null;
}
