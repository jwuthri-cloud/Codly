// Demo backend for Settings → Safety → New worktrees: the demo repo has a small setup script,
// off until you turn it on for the text shown (as the core does, by its fingerprint).

import type { SetupChoice } from '../gen/SetupChoice';
import type { WorktreeSetup } from '../gen/WorktreeSetup';

const SCRIPT = '# Runs once in each new worktree.\npnpm install --frozen-lockfile\necho "PORT=$CODLY_PORT_BASE" >> .env.local\n';
const SHA = 'demo-setup-1';

export class WorktreeDemo {
  private on = false;

  /** `worktree_setup`: the repo's setup; `set` turns the script on (for the text shown) or off. */
  setup(set: SetupChoice | null): WorktreeSetup {
    if (set) {
      if (set.allow !== null && set.allow !== SHA) throw new Error('.codly/setup.sh changed since you read it. Read it again, then turn it on');
      this.on = set.allow !== null;
    }
    return {
      script: SCRIPT,
      sha: SHA,
      risks: ['network (package install)', 'outside worktree'],
      on: this.on,
      changed: false,
      include: ['config/local.json', '.dev.vars'],
    };
  }
}
