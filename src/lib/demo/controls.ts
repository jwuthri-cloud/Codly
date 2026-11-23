// The browser demo's side of the per-agent live controls. `AgentControl` mirrors the core's state
// machine (crates/codly-core/src/orchestrator/controls.rs) so the demo behaves the same:
// a stop waits for the running turn, held notes wait for the resume, kill clears everything.
import type { Agent } from '../gen/Agent';
import type { ContextUsage } from '../gen/ContextUsage';
import type { ControlState } from '../gen/ControlState';
import type { PermissionMode } from '../gen/PermissionMode';
import type { Sim } from './sim';

const MAX_HELD = 32;

/** The name the agent panel gives each mode (the core's `PermissionMode::label`). */
export const MODE_LABEL: Record<PermissionMode, string> = { ask: 'ask for edits', accept_edits: 'accept edits', plan: 'plan first' };

export class AgentControl {
  blockTools = false;
  stopRequested = false;
  paused = false;
  held: string[] = [];

  state(): ControlState {
    return { block_tools: this.blockTools, stop_requested: this.stopRequested, paused: this.paused, held: this.held.length };
  }
  holding() {
    return this.stopRequested || this.paused;
  }
  requestStop(turnActive: boolean) {
    if (this.paused) return;
    if (turnActive) this.stopRequested = true;
    else this.paused = true;
  }
  turnCompleted() {
    if (!this.stopRequested) return;
    this.stopRequested = false;
    this.paused = true;
  }
  /** Returns the held notes to deliver now. */
  resume(): string[] {
    this.stopRequested = false;
    this.paused = false;
    const held = this.held;
    this.held = [];
    return held;
  }
  /** The text to deliver now, or null when it is held. */
  route(text: string): string | null {
    if (!this.holding()) return text;
    if (this.held.length >= MAX_HELD) this.held.shift();
    this.held.push(text);
    return null;
  }
}

export class MockControls {
  private byAgent = new Map<string, AgentControl>();
  constructor(private sim: Sim) {}

  private of(id: string) {
    let c = this.byAgent.get(id);
    if (!c) this.byAgent.set(id, (c = new AgentControl()));
    return c;
  }

  private agent(id: string) {
    const a = this.sim.agents.get(id);
    if (!a) throw new Error('Unknown agent');
    if (a.status === 'offline') throw new Error(`${a.name} is not running`);
    return a;
  }

  private publish(id: string): ControlState {
    const state = this.of(id).state();
    this.sim.emit({ type: 'control_changed', control: state }, { agent: id });
    return state;
  }

  private note(id: string, summary: string) {
    this.sim.decision('agent_control', 'human', summary, { agent_id: id }, { agent: id });
  }

  setBlockTools(id: string, on: boolean): ControlState {
    const a = this.agent(id);
    if (a.role === 'lead') throw new Error('the Lead needs its tools to plan; stop it instead');
    this.of(id).blockTools = on;
    this.note(id, `${on ? 'tools blocked' : 'tools allowed again'} for ${a.name}`);
    return this.publish(id);
  }

  stopAfterStep(id: string): ControlState {
    const a = this.agent(id);
    const act = this.sim.actor(id);
    const c = this.of(id);
    c.requestStop(act.inTurn);
    this.note(id, `${a.name} ${c.paused ? 'paused' : 'will pause after its current step'}`);
    if (c.stopRequested) {
      void this.sim.waitFor(() => !act.inTurn || !this.byAgent.has(id)).then(() => {
        if (this.byAgent.get(id) !== c || !c.stopRequested) return; // cancelled or killed meanwhile
        c.turnCompleted();
        this.note(id, 'paused after its step; resume it when you are ready');
        this.publish(id);
      });
    }
    return this.publish(id);
  }

  resume(id: string): ControlState {
    const a = this.agent(id);
    const c = this.of(id);
    if (!c.holding()) throw new Error(`${a.name} is not paused`);
    const held = c.resume();
    this.note(id, `${a.name} resumed (${held.length} held message${held.length === 1 ? '' : 's'})`);
    const state = this.publish(id);
    for (const text of held) void this.deliver(id, text);
    return state;
  }

  steer(id: string, text: string): ControlState {
    const t = text.trim();
    if (!t) throw new Error('write the steer note first');
    const a = this.agent(id);
    this.sim.chat(`agent:${id}`, 'user', `Steer note: ${t}`);
    this.note(id, `steer note for ${a.name}: ${t.slice(0, 80)}`);
    if (this.of(id).route(t) !== null) void this.deliver(id, t);
    return this.publish(id);
  }

  /** The mode of a 1:1 Claude agent: kept on the agent, as the core keeps it. */
  setMode(id: string, mode: PermissionMode): Agent {
    const a = this.sim.agents.get(id);
    if (!a) throw new Error('Unknown agent');
    if (a.role !== 'solo' || a.provider !== 'claude') throw new Error('only a 1:1 Claude agent has a mode to switch');
    const permissions = this.sim.requestPrefs[`agent:${id}`]?.approval_mode ?? this.sim.settings.approval_mode;
    if (mode === 'accept_edits' && permissions === 'strict') {
      throw new Error(`${a.name}'s permissions are "Ask first": every change asks you, so it cannot accept edits by itself. Give it other permissions first.`);
    }
    if ((a.permission_mode ?? 'ask') === mode) return a;
    this.note(id, `${a.name} now works in the mode "${MODE_LABEL[mode]}"`);
    return this.sim.patchAgent(id, { permission_mode: mode });
  }

  /** "Compact now", as in the core: a Codex agent between turns and off a task, a Claude agent
   *  between turns (the demo agent's context shrinks to a quarter). */
  compact(id: string): null {
    const a = this.agent(id);
    if (a.provider === 'codex') {
      if (this.sim.actor(id).inTurn) throw new Error(`${a.name} is working. Compact it when its turn ends.`);
      if (a.task_id) throw new Error(`${a.name} is on a task. Compact it between tasks.`);
      this.note(id, `you asked Codex to compact ${a.name}’s conversation`);
      this.sim.emit({ type: 'compacting', pre_tokens: null, post_tokens: null }, { agent: id });
      return null;
    }
    if (a.provider !== 'claude') throw new Error('only a Claude or a Codex agent can be compacted from here');
    if (this.sim.actor(id).inTurn) throw new Error(`${a.name} is still working: compact it when its turn ends`);
    const used = a.usage.context_used ?? 0;
    const left = Math.round(used / 4);
    this.note(id, `asked ${a.name} to compact its conversation`);
    this.sim.emit({ type: 'compacting', pre_tokens: used, post_tokens: left }, { agent: id });
    this.sim.patchAgent(id, { usage: { ...a.usage, context_used: left } });
    return null;
  }

  contextUsage(id: string): ContextUsage {
    const a = this.agent(id);
    if (a.provider !== 'claude') throw new Error('only a Claude agent takes this without a restart');
    const used = a.usage.context_used ?? 0;
    const max = a.usage.context_window ?? 200_000;
    return { used_tokens: used, max_tokens: max, percent: Math.round((used / max) * 100), auto_compact_at: Math.round(max * 0.95), parts: [{ name: 'Messages', tokens: used }] };
  }

  /** The note reaches the agent on its next turn: it acknowledges it in its own thread. */
  private async deliver(id: string, text: string) {
    const act = this.sim.actor(id);
    try {
      await this.sim.waitFor(() => !act.inTurn);
      await act.turn(`[Steer note] ${text}`, async () => {
        await act.say(`Got your note — “${text.slice(0, 60)}${text.length > 60 ? '…' : ''}”. I’ll take it into account in my next step.`, { thread: `agent:${id}` });
      });
    } catch {
      /* cancelled by a kill */
    }
  }

  /** Kill (stop_agent): the controls end with the process. */
  clear(id: string) {
    const c = this.byAgent.get(id);
    this.byAgent.delete(id);
    if (!c || (!c.blockTools && !c.paused && !c.stopRequested && !c.held.length)) return;
    this.sim.emit({ type: 'control_changed', control: { block_tools: false, stop_requested: false, paused: false, held: 0 } }, { agent: id });
  }
}
