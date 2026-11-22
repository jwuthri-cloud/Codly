// "Check the work" (an agent's Git tab): the result of the repo's required checks, in words.

import type { CheckResult } from '../gen/CheckResult';

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
