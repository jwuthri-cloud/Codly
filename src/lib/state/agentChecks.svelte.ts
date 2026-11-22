// "Check the work" runs that are on their way, by agent. Kept here and not in the Git tab: the
// commands can take minutes, and the tab must still say that they run when you come back to it.
// The result is not kept here: the core records it as a decision (`lastAgentCheck` reads it).
import { api } from '../api';

class AgentChecks {
  /** Agents whose required checks run now. */
  running = $state<Record<string, boolean>>({});
  /** Why an agent's last run gave no result, until its next run. */
  failed = $state<Record<string, string | null>>({});

  /** Run the repo's required checks in the agent's worktree. One run for an agent at a time. */
  async run(agentId: string) {
    if (this.running[agentId]) return;
    this.running = { ...this.running, [agentId]: true };
    this.failed = { ...this.failed, [agentId]: null };
    try {
      await api.checkAgent(agentId);
    } catch (e) {
      this.failed = { ...this.failed, [agentId]: e instanceof Error ? e.message : String(e) };
    } finally {
      this.running = { ...this.running, [agentId]: false };
    }
  }
}

export const agentChecks = new AgentChecks();
