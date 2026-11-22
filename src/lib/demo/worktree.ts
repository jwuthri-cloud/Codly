// Demo backend for Settings → Safety → New worktrees: the demo repo has a small setup script,
// off until you turn it on for the text shown (as the core does, by its fingerprint).

import type { SetupChoice } from '../gen/SetupChoice';
import type { WorktreeSetup } from '../gen/WorktreeSetup';

const SCRIPT = '#!/usr/bin/env bash\n# Runs once in each new worktree.\npnpm install --frozen-lockfile\necho "PORT=$CODLY_PORT_BASE" >> .env.local\n';

/** Stands in for the core's SHA-256: one value per text. */
function fingerprint(text: string): string {
  let h = 0;
  for (let i = 0; i < text.length; i++) h = (Math.imul(h, 31) + text.charCodeAt(i)) | 0;
  return `demo-${(h >>> 0).toString(16)}`;
}

export class WorktreeDemo {
  /** The text of `.codly/setup.sh` in the demo repo. A pull or a merged task can change it. */
  script = SCRIPT;
  /** The fingerprint of the text you turned on. */
  private allowed: string | null = null;

  /** `worktree_setup`: the repo's setup; `set` turns the script on (for the text shown) or off. */
  setup(set: SetupChoice | null): WorktreeSetup {
    const sha = fingerprint(this.script);
    if (set) {
      if (set.allow !== null && set.allow !== sha) throw new Error('.codly/setup.sh changed since you read it. Read it again, then turn it on');
      this.allowed = set.allow;
    }
    return {
      script: this.script,
      sha,
      risks: ['network (package install)', 'outside worktree'],
      on: this.allowed === sha,
      changed: this.allowed !== null && this.allowed !== sha,
      problem: null,
      include: ['config/local.json', '.dev.vars'],
    };
  }
}
