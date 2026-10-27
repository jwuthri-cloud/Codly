// The add-agent wizard's intents: start the reviewed hire, and "Generate with AI" (ask the Lead
// for a manifest draft). Neither skips the review: a draft only pre-fills the wizard.
import { api } from '../api';
import type { HireManifest } from '../gen/HireManifest';
import type { HireReview } from '../gen/HireReview';
import { PROVIDER_LABEL } from '../util/labels';
import { HIRE_SCHEMA, extractManifest } from '../util/hire';
import { store } from './store.svelte';
import { toast } from './toasts.svelte';
import { ui } from './ui.svelte';

const msg = (e: unknown) => (e instanceof Error ? e.message : typeof e === 'string' ? e : 'Unknown error');

/** Start the agent the person confirmed. Returns whether it started, so the wizard can close. */
export async function startHire(m: HireManifest, confirmShell: boolean): Promise<boolean> {
  try {
    const a = await api.hireFromManifest(m, confirmShell);
    store.upsertAgent(a);
    ui.selectAgent(a.id);
    toast({ kind: 'success', title: `${a.name} joined`, body: `${PROVIDER_LABEL[a.provider]} · 1:1 chat is ready` });
    return true;
  } catch (e) {
    toast({ kind: 'error', title: `Couldn’t start ${m.name.trim() || 'the agent'}`, body: msg(e) });
    return false;
  }
}

/**
 * Ask the Lead to draft a manifest for `brief` and wait for its reply (a new Lead message that
 * carries the manifest). The request shows in the Lead's conversation like any message.
 * Rejects on timeout or when `signal` aborts; resolves with the core's review of the draft.
 */
export async function askLeadForHire(brief: string, signal: AbortSignal, timeoutMs = 240_000): Promise<HireReview> {
  const thread = ui.request || 'lead';
  const seen = new Set(store.chats.map((c) => c.id));
  const prompt = await api.hireDraftPrompt(brief);
  await api.sendChat(thread, prompt);
  const deadline = Date.now() + timeoutMs;
  while (!signal.aborted) {
    const reply = store.chats.find((c) => !seen.has(c.id) && c.thread === thread && c.author !== 'user' && c.text.includes(HIRE_SCHEMA));
    const raw = reply && extractManifest(reply.text);
    if (raw) return api.hireReview(raw);
    if (reply) seen.add(reply.id); // mentions the schema but carries no manifest: keep waiting for the real one
    if (Date.now() > deadline) throw new Error('The Lead did not answer in time. Check its conversation, or write the briefing yourself.');
    await new Promise((r) => setTimeout(r, 400));
  }
  throw new Error('cancelled');
}
