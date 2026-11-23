// The add-agent wizard's intents: start the reviewed hire, and "Generate with AI" (ask Claude
// for a schema-enforced manifest draft). Neither skips the review: a draft only pre-fills the wizard.
import { api } from '../api';
import type { HireManifest } from '../gen/HireManifest';
import type { Applicant } from '../gen/Applicant';
import type { HireReview } from '../gen/HireReview';
import type { Provider } from '../gen/Provider';
import { article } from '../util/format';
import { blankManifest } from '../util/hire';
import { PROVIDER_LABEL } from '../util/labels';
import { store } from './store.svelte';
import { toast } from './toasts.svelte';
import { ui } from './ui.svelte';

const msg = (e: unknown) => (e instanceof Error ? e.message : typeof e === 'string' ? e : 'Unknown error');

/** Start the agent the person confirmed. Returns whether it started, so the wizard can close.
 *  `sheet`: its sheet opens beside its 1:1 chat. Either way the keyboard is in its composer. */
export async function startHire(m: HireManifest, confirmShell: boolean, sheet = true): Promise<boolean> {
  const ws = store.workspace?.id;
  try {
    const a = await api.hireFromManifest(m, confirmShell);
    // It joined the office it was hired in: after a switch it is not this one's to show.
    if (store.isOpen(ws)) {
      store.upsertAgent(a);
      // Its 1:1 chat (and its sheet) show the agent you just started; you write to it at once.
      ui.openThread(store.threadOf(a.id));
      if (sheet) ui.selectAgent(a.id);
      ui.composerFocus++;
    }
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

/**
 * The hire "Start an agent with <CLI>" makes in one step: the first applicant's name and look
 * (the core picks names and colours no agent has), the CLI's default model, no traits, no brief.
 */
export function quickManifest(provider: Provider, applicant: Pick<Applicant, 'name' | 'color' | 'avatar'>): HireManifest {
  return { ...blankManifest(provider), name: applicant.name, color: applicant.color, avatar: applicant.avatar };
}

/** "Start an agent with <CLI>": start it now, with the defaults, and open its 1:1 chat. */
export async function quickStart(provider: Provider): Promise<boolean> {
  let applicant: Applicant | undefined;
  try {
    [applicant] = await api.hireApplicants(1);
  } catch (e) {
    toast({ kind: 'error', title: `Couldn’t start ${article(PROVIDER_LABEL[provider])} ${PROVIDER_LABEL[provider]} agent`, body: msg(e) });
    return false;
  }
  if (!applicant) {
    toast({ kind: 'error', title: `Couldn’t start ${article(PROVIDER_LABEL[provider])} ${PROVIDER_LABEL[provider]} agent`, body: 'Codly found no name for it. Use the wizard: Start an agent (1:1)….' });
    return false;
  }
  return startHire(quickManifest(provider, applicant), false, false);
}
