// The pull-request step, from the buttons' side. Nothing is sent by itself: a draft is asked
// for when you press Create pull request, and a push or a pull request is sent only from the
// dialog that named the remote, the branch and the base (`confirm`). A failure says in a toast
// what was sent and what was not. Only reads run by themselves: an open pull request's checks.
import { api } from '../api';
import type { PrChecks } from '../gen/PrChecks';
import type { PrConfirm } from '../gen/PrConfirm';
import type { PrDraft } from '../gen/PrDraft';
import type { PrPushed } from '../gen/PrPushed';
import type { PrSource } from '../gen/PrSource';
import type { PullRequest } from '../gen/PullRequest';
import { toast, toasts } from './toasts.svelte';

class PrUi {
  /** The confirm dialog: what would be sent, for whom. */
  dialog = $state<{ source: PrSource; draft: PrDraft } | null>(null);
  /** The pull requests Codly opened, as last read, by `prKey` (null: none; missing: not read yet). */
  opened = $state<Record<string, PullRequest | null>>({});
}
export const prUi = new PrUi();

export const prKey = (source: PrSource) => `${source.kind}:${source.id}`;

const msg = (e: unknown) => (e instanceof Error ? e.message : typeof e === 'string' ? e : 'Unknown error');

/** The pull request on record for a plan or a solo agent. Reads Codly's own record: no network. */
export async function loadPullRequest(source: PrSource) {
  try {
    const pr = await api.pullRequest(source);
    prUi.opened[prKey(source)] = pr;
  } catch {
    // Not read: the row stays hidden, as while it loads. Create pull request says what is wrong.
  }
}

/**
 * Create pull request… or Push new commits…: draft it and open the confirm. Nothing leaves this
 * Mac yet. The draft also says where a pull request on record stands now (merged or closed on
 * GitHub since the last read), and the row shows that.
 */
export async function preparePullRequest(source: PrSource): Promise<boolean> {
  try {
    const draft = await api.draftPullRequest(source);
    const pr = prUi.opened[prKey(source)];
    if (pr && draft.pr_state) prUi.opened[prKey(source)] = { ...pr, state: draft.pr_state };
    prUi.dialog = { source, draft };
    return true;
  } catch (e) {
    toast({ kind: 'error', title: 'Couldn’t prepare the pull request', body: msg(e) });
    return false;
  }
}

/** Push the branch you confirmed and open the pull request with your own gh. */
export async function createPullRequest(source: PrSource, confirm: PrConfirm, title: string, body: string): Promise<boolean> {
  const id = toast({ kind: 'progress', title: `Pushing ${confirm.branch} to ${confirm.remote}…` });
  try {
    const pr = await api.createPullRequest(source, confirm, title, body);
    prUi.opened[prKey(source)] = pr;
    toasts.update(id, { kind: 'success', title: 'Pull request opened', body: pr.url });
    return true;
  } catch (e) {
    toasts.update(id, { kind: 'error', title: 'Couldn’t open the pull request', body: msg(e) });
    return false;
  }
}

/** Push the branch you confirmed, without a pull request. */
export async function pushPrBranch(source: PrSource, confirm: PrConfirm): Promise<PrPushed | undefined> {
  const id = toast({ kind: 'progress', title: `Pushing ${confirm.branch} to ${confirm.remote}…` });
  try {
    const pushed = await api.pushPrBranch(source, confirm);
    toasts.update(id, { kind: 'success', title: `Pushed ${pushed.branch}`, body: `It is on ${pushed.remote} now.` });
    return pushed;
  } catch (e) {
    toasts.update(id, { kind: 'error', title: `Couldn’t push ${confirm.branch}`, body: msg(e) });
    return undefined;
  }
}

/** How often an open pull request's checks are read again while its row is on screen, in ms. */
export const PR_REFRESH_MS = 30_000;

/**
 * Refresh: read the pull request's state and checks once more. A read you asked for (the
 * button) that fails says why in a toast. One the row makes by itself (`byItself`: every 30 s,
 * when the window gets the focus) says nothing when it fails: the row keeps the last reading and
 * says how old it is, and the next one tries again.
 */
export async function refreshPullRequest(source: PrSource, byItself = false) {
  try {
    const pr = await api.refreshPullRequest(source);
    prUi.opened[prKey(source)] = pr;
  } catch (e) {
    if (!byItself) toast({ kind: 'error', title: 'Couldn’t read the pull request', body: msg(e) });
  }
}

/**
 * What the confirm does, from the pull request on record (`pr`) and the draft:
 * - `create`: push the branch and open a pull request (none is on record, or that one is merged
 *   or closed by now);
 * - `join`: push new commits, which join the pull request that gh just said is open;
 * - `push`: only push. gh cannot open a pull request, or cannot say where the one on record stands.
 */
export function confirmMode(pr: PullRequest | null | undefined, draft: PrDraft): 'create' | 'join' | 'push' {
  if (draft.gh_problem) return 'push';
  if (!pr || (draft.pr_state && draft.pr_state !== 'open')) return 'create';
  return draft.pr_state === 'open' ? 'join' : 'push';
}

/** The checks in one line, the worst news first: "1 failed (lint) · 2 running · 3 passed". */
export function checksLine(c: PrChecks): string {
  const parts: string[] = [];
  if (c.failed) parts.push(`${c.failed} failed (${c.failing.join(', ')}${c.failed > c.failing.length ? ', …' : ''})`);
  if (c.pending) parts.push(`${c.pending} running`);
  if (c.passed) parts.push(`${c.passed} passed`);
  if (c.skipped) parts.push(`${c.skipped} skipped`);
  return parts.length ? parts.join(' · ') : 'none reported yet';
}

/** How the checks stand as a whole, for the colour of their line. */
export function checksState(c: PrChecks): 'bad' | 'wait' | 'ok' | 'none' {
  if (c.failed) return 'bad';
  if (c.pending) return 'wait';
  return c.passed ? 'ok' : 'none';
}
