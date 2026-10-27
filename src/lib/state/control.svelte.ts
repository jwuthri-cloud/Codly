// Per-agent live controls (agent panel): block tools, stop after this step, steer note.
// The core owns the state machine (orchestrator/controls.rs); this mirrors its latest
// `control_changed` event per agent (store.svelte.ts feeds `set`) and sends the intents.
// Kill is the existing `stopAgent` action.
import { api } from '../api';
import type { ControlState } from '../gen/ControlState';
import { toast } from './toasts.svelte';

export const NO_CONTROL: ControlState = { block_tools: false, stop_requested: false, paused: false, held: 0 };

const msg = (e: unknown) => (e instanceof Error ? e.message : typeof e === 'string' ? e : 'Unknown error');

class Controls {
  byAgent = $state.raw<Record<string, ControlState>>({});

  of(agentId: string): ControlState {
    return this.byAgent[agentId] ?? NO_CONTROL;
  }

  set(agentId: string, c: ControlState) {
    const { [agentId]: _old, ...rest } = this.byAgent;
    this.byAgent = c.block_tools || c.stop_requested || c.paused || c.held ? { ...rest, [agentId]: c } : rest;
  }
}

export const controls = new Controls();

async function run(what: string, agentId: string, fn: () => Promise<ControlState>): Promise<boolean> {
  try {
    controls.set(agentId, await fn()); // the event says the same; this is for the instant feedback
    return true;
  } catch (e) {
    toast({ kind: 'error', title: `Couldn’t ${what}`, body: msg(e) });
    return false;
  }
}

export const setBlockTools = (agentId: string, on: boolean) => run(on ? 'block its tools' : 'allow its tools', agentId, () => api.setAgentBlockTools(agentId, on));
export const stopAfterStep = (agentId: string) => run('stop it after this step', agentId, () => api.stopAgentAfterStep(agentId));
export const resumeAgent = (agentId: string) => run('resume it', agentId, () => api.resumeAgent(agentId));
export const steerAgent = (agentId: string, text: string) => run('send the steer note', agentId, () => api.steerAgent(agentId, text));
