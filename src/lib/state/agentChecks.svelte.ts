// "Check the work", by agent: what the core says about it. The core knows whether an agent's
// required checks run now and which result is still about the agent's files, so nothing of that
// is kept only in the window: after a reload the Git tab still says that the commands run, and
// a result goes when the files change (a turn, restored files, a merge, your own edits).
import { api } from '../api';
import type { AgentCheck } from '../gen/AgentCheck';

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** When the window gets the focus, the core is asked about an agent once in this time at most:
 *  each question makes it compare the agent's files, which starts git. */
export const FOCUS_READ_MS = 10_000;

class AgentChecks {
  /** What the core last said about each agent. */
  state = $state<Record<string, AgentCheck>>({});
  /** Why an agent's last run gave no result, until its next run. */
  failed = $state<Record<string, string | null>>({});
  /** Why the core could not say what it has for an agent, until it can. */
  unread = $state<Record<string, string | null>>({});
  /** The newest question to the core about each agent: an older answer that comes later is dropped. */
  private asked: Record<string, number> = {};
  /** When the core was last asked about each agent. */
  private askedAt: Record<string, number> = {};
  /** By agent: the question of a focus that came too soon after the last one, until it is asked
   *  (`timer`), with when that focus came (`since`). */
  private waiting: Record<string, { timer: ReturnType<typeof setTimeout>; since: number }> = {};

  /** Ask the core what it has for the agent now: when its Git tab opens, when its worktree was
   *  read again, after a run, and while a run is on its way. (The window's focus: `focused`.)
   *  An answer to a question asked after a focus that waits is about the files as they were
   *  after that focus: the focus's own question is not asked any more. */
  async load(agentId: string) {
    const mine = (this.asked[agentId] = (this.asked[agentId] ?? 0) + 1);
    const askedAt = (this.askedAt[agentId] = Date.now());
    try {
      const now = await api.agentCheckState(agentId);
      if (this.asked[agentId] !== mine) return;
      this.state = { ...this.state, [agentId]: now };
      this.unread = { ...this.unread, [agentId]: null };
      const wait = this.waiting[agentId];
      if (wait && wait.since <= askedAt) {
        clearTimeout(wait.timer);
        delete this.waiting[agentId];
      }
    } catch (e) {
      if (this.asked[agentId] === mine) this.unread = { ...this.unread, [agentId]: message(e) };
    }
  }

  /**
   * Codly's window got the focus back: a file you changed in your editor meanwhile is not the
   * checked one any more, and no event says so, so the core is asked. A window that gets the
   * focus again and again asks once in `FOCUS_READ_MS` for an agent. A focus inside that time
   * is not dropped: its question is asked when the time is over, so a result that is not about
   * the files any more never stays longer than that.
   */
  focused(agentId: string) {
    if (agentId in this.waiting) return;
    const wait = (this.askedAt[agentId] ?? -Infinity) + FOCUS_READ_MS - Date.now();
    if (wait <= 0) {
      void this.load(agentId);
      return;
    }
    const timer = setTimeout(() => {
      delete this.waiting[agentId];
      void this.load(agentId);
    }, wait);
    this.waiting[agentId] = { timer, since: Date.now() };
  }

  /** Run the repo's required checks in the agent's worktree. One run for an agent at a time. */
  async run(agentId: string) {
    if (this.state[agentId]?.running) return;
    // An answer from before this run would say that nothing runs.
    this.asked[agentId] = (this.asked[agentId] ?? 0) + 1;
    this.state = { ...this.state, [agentId]: { running: true, last: null } };
    this.failed = { ...this.failed, [agentId]: null };
    try {
      await api.checkAgent(agentId);
    } catch (e) {
      this.failed = { ...this.failed, [agentId]: message(e) };
    }
    // This run is over. The core says what it has now: the new result, or that another run
    // (one from before a reload) is still on its way.
    this.state = { ...this.state, [agentId]: { running: false, last: null } };
    await this.load(agentId);
  }
}

export const agentChecks = new AgentChecks();
