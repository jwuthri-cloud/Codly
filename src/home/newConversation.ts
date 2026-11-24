// `/new` in a 1:1 chat: the agent's next message starts a new session of its CLI, with an empty
// context, as `/clear` or `/new` do in Claude Code and Codex (orchestrator/new_conversation.rs).
// The same agent, folder and model; its files and checkpoints stay. It asks once before it
// acts: the `/` menu closes when a row runs, so the question waits in a toast, whose button
// starts, and Esc on the empty composer goes back (esc.ts `composerEsc`).
import { api } from '../lib/api';
import type { Agent } from '../lib/gen/Agent';
import { toast } from '../lib/state/toasts.svelte';
import type { SlashContext } from './composerActions';
import type { SlashCommand } from './composerMenu';

const msg = (e: unknown) => (e instanceof Error ? e.message : typeof e === 'string' ? e : 'Unknown error');

/** The row of `/new` in a 1:1 chat: it runs for a 1:1 agent that is free, else it says why not, in the core's words. */
export function newConversationRow(c: SlashContext): SlashCommand {
  const row = { name: 'new', label: 'New conversation', icon: 'plus' };
  const a = c.agent;
  if (!a) return { ...row, off: 'This agent is gone' };
  if (a.role !== 'solo') return { ...row, off: 'Only a 1:1 agent can start a new conversation' };
  if (a.task_id) return { ...row, off: `${a.name} is on a task: start a new conversation when the task is done` };
  if (c.busy) return { ...row, off: `${a.name} is still working: stop its turn first` };
  if (c.takeover?.takeover) return { ...row, off: `${a.name} is in your terminal: give it back first` };
  return { ...row, hint: `Start a new conversation: ${a.name} forgets this one`, run: () => askNewConversation(a) };
}

/** The question before a new conversation: nothing happens until you press Start. */
export function askNewConversation(a: Pick<Agent, 'id' | 'name'>) {
  toast({
    kind: 'attention',
    title: 'Start a new conversation?',
    body: `${a.name} forgets what was said. Files and checkpoints stay as they are. Esc to go back.`,
    duration: 0,
    action: { label: 'Start', run: () => void startNewConversation(a) },
  });
}

/** Ask the core for the new conversation. Its thread then says so (the core's line), so only a refusal says something here. */
export async function startNewConversation(a: Pick<Agent, 'id' | 'name'>): Promise<boolean> {
  try {
    await api.newConversation(a.id);
    return true;
  } catch (e) {
    toast({ kind: 'error', title: `Couldn’t start a new conversation with ${a.name}`, body: msg(e) });
    return false;
  }
}
