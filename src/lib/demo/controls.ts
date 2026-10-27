// The browser demo's side of the per-agent live controls. `AgentControl` mirrors the core's state
// machine (crates/codly-core/src/orchestrator/controls.rs) so the demo behaves the same:
// a stop waits for the running turn, held notes wait for the resume, kill clears everything.
import type { ControlState } from '../gen/ControlState';
import type { Sim } from './sim';

const MAX_HELD = 32;

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

  state(id: string): ControlState {
    return this.of(id).state();
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
