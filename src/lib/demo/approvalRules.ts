// Mock of "Always allow in this repo" for `pnpm dev` and the site demo. In the app the core
// decides what a rule may cover (orchestrator/approvals.rs). Here a request with a risk label
// offers nothing, as the story's requests do, and a command with none can be saved as itself;
// saving it answers the requests that wait with the same command.
import type { ApprovalRule } from '../gen/ApprovalRule';
import type { ApprovalRuleKind } from '../gen/ApprovalRuleKind';
import type { ApprovalRuleOffer } from '../gen/ApprovalRuleOffer';
import { uid, type Sim } from './sim';
import type { Story } from './story';

export class MockApprovalRules {
  private rules: ApprovalRule[] = [];

  constructor(private sim: Sim, private story: Story) {}

  offer(approvalId: string): ApprovalRuleOffer {
    const ap = this.sim.approvals.get(approvalId);
    if (!ap || ap.status !== 'pending' || ap.subject.kind !== 'command' || ap.risks.length) return { exact: null, prefix: null };
    return { exact: ap.subject.command.trim(), prefix: null };
  }

  add(approvalId: string, kind: ApprovalRuleKind): ApprovalRule {
    const command = this.offer(approvalId)[kind];
    if (!command) throw new Error('Codly always asks about this kind of request, so it cannot be saved as a rule.');
    let rule = this.rules.find((r) => r.kind === kind && r.command === command);
    if (!rule) {
      rule = { id: uid('rule-'), workspace_id: this.sim.workspace?.id ?? '', kind, command, created_at: this.sim.now() };
      this.rules = [...this.rules, rule];
    }
    for (const ap of [...this.sim.approvals.values()]) if (this.offer(ap.id)[kind] === command) this.story.resolveApproval(ap.id, 'allow_once', null);
    return rule;
  }

  list(): ApprovalRule[] {
    return this.rules;
  }

  remove(ruleId: string) {
    if (!this.rules.some((r) => r.id === ruleId)) throw new Error('That rule was already removed.');
    this.rules = this.rules.filter((r) => r.id !== ruleId);
  }
}
