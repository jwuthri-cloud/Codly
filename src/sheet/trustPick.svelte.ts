// The agent sheet's Trust choice (AgentTrust.svelte). Taking trust back is one click. Giving
// more asks first, like Stop and Let go: one misclick must not let an agent skip your approvals
// or the review.

import type { Rank } from '../lib/gen/Rank';
import { setTrust } from '../lib/state/actions';
import { raisesTrust } from '../lib/util/career';

export class TrustPick {
  /** The higher level you clicked, until you answer "Trust" or "Keep". */
  asking = $state<Rank | null>(null);
  saving = $state(false);

  /** A click on a level: a lower one is saved at once, a higher one waits for `confirm`. */
  async pick(agentId: string, current: Rank, next: Rank) {
    if (this.saving) return;
    this.asking = null;
    if (raisesTrust(current, next)) this.asking = next;
    else if (next !== current) await this.save(agentId, next);
  }

  /** "Trust": save the level you were asked about. */
  async confirm(agentId: string) {
    const level = this.asking;
    if (!level || this.saving) return;
    this.asking = null;
    await this.save(agentId, level);
  }

  /** "Keep": the trust stays as it is. */
  keep() {
    this.asking = null;
  }

  private async save(agentId: string, level: Rank) {
    this.saving = true;
    await setTrust(agentId, level);
    this.saving = false;
  }
}
