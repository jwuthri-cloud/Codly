// `/new` in a 1:1 chat: the agent's next message starts a new session of its CLI, with an empty
// context, as `/clear` or `/new` do in Claude Code and Codex (orchestrator/new_conversation.rs).
// The same agent, folder and model; its files and checkpoints stay. It asks once before it
// acts: the `/` menu closes when a row runs, so the question waits in a toast, whose button
// starts, and Esc on the empty composer goes back (esc.ts `composerEsc`). The question is about
// the conversation as it was when it was asked: when that conversation goes on, it goes.
import { api } from '../lib/api';
import type { Agent } from '../lib/gen/Agent';
import type { Task } from '../lib/gen/Task';
import { busy } from '../lib/state/queue';
import { store } from '../lib/state/store.svelte';
import { toast, toasts } from '../lib/state/toasts.svelte';
import { ui } from '../lib/state/ui.svelte';
import type { SlashContext } from './composerActions';
import type { SlashCommand } from './composerMenu';

const msg = (e: unknown) => (e instanceof Error ? e.message : typeof e === 'string' ? e : 'Unknown error');

const QUESTION = 'Start a new conversation?';

/** Why a task keeps the agent from a new conversation, in the core's words; null: none does.
 *  The task it runs, or one given to it that is not done or cancelled (as the core looks). */
function onTask(a: Agent, tasks: Task[]): string | null {
  const held = tasks.find((t) => t.id === a.task_id || (t.assignee === a.id && t.status !== 'done' && t.status !== 'cancelled'));
  if (held && !a.task_id && (held.status === 'failed' || held.status === 'blocked')) return `${a.name} still has ${held.key} (${held.status}): retry it or cancel it first`;
  if (a.task_id || held) return `${a.name} is on ${held?.key ?? 'a task'}: start a new conversation when the task is done`;
  return null;
}

/** The row of `/new` in a 1:1 agent's chat: it runs when the agent is free, else it says why not, in the core's words. */
export function newConversationRow(c: SlashContext & { agent: Agent }): SlashCommand {
  const row = { name: 'new', label: 'New conversation', icon: 'plus' };
  const a = c.agent;
  const task = onTask(a, store.tasks);
  if (task) return { ...row, off: task };
  if (c.busy) return { ...row, off: `${a.name} is still working: stop its turn first` };
  if (c.takeover?.takeover) return { ...row, off: `${a.name} is in your terminal: give it back first` };
  return { ...row, hint: `Start a new conversation: ${a.name} forgets this one`, run: () => askNewConversation(a) };
}

/**
 * The question before a new conversation: nothing happens until you press Start. Asked once:
 * while it shows, `/new` asks nothing more. It goes when what it asks about is not there any
 * more: you left the chat, the agent is gone or at work, or you said something to it since.
 */
export function askNewConversation(a: Pick<Agent, 'id' | 'name'>) {
  const body = `${a.name} forgets what was said. Files and checkpoints stay as they are. Esc to go back.`;
  if (toasts.list.some((t) => t.action && t.title === QUESTION && t.body === body)) return;
  const thread = `agent:${a.id}`;
  const lastSaid = () => store.chats.findLast((m) => m.thread === thread && m.author === 'user')?.id;
  const said = lastSaid();
  const asked = Date.now();
  const gone = () => {
    const now = store.agentMap.get(a.id);
    return ui.thread !== thread || !now || !!now.task_id || busy(thread) || lastSaid() !== said || store.outbox.some((o) => o.thread === thread && o.ts >= asked);
  };
  toast({ kind: 'attention', title: QUESTION, body, duration: 0, action: { label: 'Start', run: () => void startNewConversation(a), gone } });
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
