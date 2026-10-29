// The add-agent wizard's intents: start the reviewed hire, and "Generate with AI" (ask Claude
// for a schema-enforced manifest draft). Neither skips the review: a draft only pre-fills the wizard.
import { api } from '../api';
import type { HireManifest } from '../gen/HireManifest';
import type { HireReview } from '../gen/HireReview';
import { PROVIDER_LABEL } from '../util/labels';
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
 * Ask Claude for a manifest draft for `brief`, forced into the hire schema by the core. Nothing
 * starts; resolves with the core's review of the draft. Rejects when `signal` aborts first.
 */
export async function askForHire(brief: string, signal: AbortSignal): Promise<HireReview> {
  const review = await api.hireDraft(brief);
  if (signal.aborted) throw new Error('cancelled');
  return review;
}
