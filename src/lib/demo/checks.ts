// Demo backend for Settings → Safety → Required checks and the Git tab's "Check the work": the
// demo repo has a small checks file, off until you turn it on for the text shown (as the core
// does, by its fingerprint).

import type { AgentCheck } from '../gen/AgentCheck';
import type { CheckedWork } from '../gen/CheckedWork';
import type { CheckResult } from '../gen/CheckResult';
import type { ChecksChoice } from '../gen/ChecksChoice';
import type { RequiredChecks } from '../gen/RequiredChecks';

const TEXT = '# What must pass before work lands.\npnpm check\npnpm test\n';

/** Stands in for the core's SHA-256: one value per text. */
function fingerprint(text: string): string {
  let h = 0;
  for (let i = 0; i < text.length; i++) h = (Math.imul(h, 31) + text.charCodeAt(i)) | 0;
  return `demo-${(h >>> 0).toString(16)}`;
}

/** The commands of a checks file: its lines that are not blank and not a comment. */
function commands(text: string): string[] {
  return text.split('\n').map((l) => l.trim()).filter((l) => l && !l.startsWith('#'));
}

export class ChecksDemo {
  /** The text of `.codly/checks` in the demo repo. A pull or a merged task can change it. */
  text = TEXT;
  /** The fingerprint of the text you turned on. */
  private allowed: string | null = null;
  /** The last "Check the work" of each agent, until the agent works again. */
  private checked = new Map<string, CheckedWork>();

  /** `on` (`?checks=on`): start with the checks turned on, as after a click in Settings. */
  constructor(on = false) {
    if (on) this.allowed = fingerprint(this.text);
  }

  /** `required_checks`: the repo's checks; `set` turns them on (for the text shown) or off. */
  status(set: ChecksChoice | null): RequiredChecks {
    const sha = fingerprint(this.text);
    if (set) {
      if (set.allow !== null && set.allow !== sha) throw new Error('.codly/checks changed since you read it. Read it again, then turn it on');
      this.allowed = set.allow;
    }
    const on = this.allowed === sha;
    return { text: this.text, sha, commands: commands(this.text).map((command) => ({ command, risks: [] })), on, changed: this.allowed !== null && !on, problem: null };
  }

  /** What the gate runs for every task, after the task's own checks: your commands while they are on, none while they are off or changed. */
  required(): string[] {
    const now = this.status(null);
    return now.on ? now.commands.map((c) => c.command) : [];
  }

  /** `check_agent`: every command passes in the demo, once the checks are on. `at`: now (ms). */
  run(agentId: string, at: number): CheckResult[] {
    const now = this.status(null);
    if (now.changed) throw new Error('Codly did not run your required checks: .codly/checks changed since you turned them on. Read them in Settings → Safety → Required checks and turn them on again, or turn them off.');
    if (!now.on) throw new Error('this repo has no required checks turned on. Write your commands in .codly/checks, one on each line, then turn them on in Settings → Safety → Required checks');
    const results = now.commands.map((c) => ({ command: c.command, ok: true, exit_code: 0, timed_out: false, tail: 'ok' }));
    this.checked.set(agentId, { results, at });
    return results;
  }

  /** `agent_check_state`: the demo's commands end at once, so none runs when the Git tab asks.
   *  The core drops a result when the agent's files change; in the demo every turn changes
   *  them, so the result goes when the agent works (`working`). */
  state(agentId: string, working: boolean): AgentCheck {
    if (working) this.checked.delete(agentId);
    return { running: false, last: this.checked.get(agentId) ?? null };
  }
}
