<!--
  The confirm before anything is sent for a pull request. It names the remote, the Codly branch
  that is pushed and the base, and shows the title and description Codly drafted from the plan,
  for you to edit. When gh cannot open the pull request it says why and offers Push branch, then
  the compare page. When the branch has a pull request that is still open, it only pushes the new
  commits; when that one is merged or closed by now, it opens a new one.
-->
<script lang="ts">
  import type { PrPushed } from '../lib/gen/PrPushed';
  import { confirmMode, createPullRequest, prKey, prUi, pushPrBranch } from '../lib/state/pullRequest.svelte';
  import { openExternal } from '../lib/util/markdown';
  import Dialog from '../components/Dialog.svelte';
  import Icon from '../components/Icon.svelte';

  const { source, draft } = prUi.dialog!;
  const record = prUi.opened[prKey(source)] ?? null;
  const mode = confirmMode(record, draft);
  const pushOnly = mode !== 'create';
  // The pull request the push adds to: open as gh just said (join), or as last read (push).
  const open = pushOnly && record?.state === 'open' ? record : null;
  let base = $state(open?.base ?? draft.base);
  let title = $state(draft.title);
  let body = $state(draft.body);
  let busy = $state(false);
  let pushed = $state<PrPushed | null>(null);
  const confirm = $derived({ remote: draft.remote, branch: draft.branch, base: base.trim() });
  const close = () => (prUi.dialog = null);

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    if (pushOnly) pushed = (await pushPrBranch(source, confirm)) ?? null;
    else if (await createPullRequest(source, confirm, title, body)) close();
    busy = false;
  }
</script>

<Dialog label={open ? 'Push new commits' : 'Create pull request'} width={640} onclose={close}>
  <form onsubmit={submit}>
    <div class="body">
      <p class="desc">
        {#if mode === 'join'}
          Codly pushes the new commits on this branch. They join the pull request that is open.
        {:else if open}
          Codly pushes this branch. If its pull request is still open, the new commits join it.
        {:else if pushOnly}
          Codly pushes this branch. You open the pull request in your browser.
        {:else if record}
          The pull request from before is {record.state}. Codly pushes this branch and opens a new one with your own gh sign-in.
        {:else}
          Codly pushes this branch and opens a pull request with your own gh sign-in.
        {/if}
        No other branch is pushed, and never with force.
      </p>
      <dl class="where">
        <dt class="eyebrow">Remote</dt>
        <dd><code>{draft.remote}</code></dd>
        <dt class="eyebrow">Branch</dt>
        <dd><code>{draft.branch}</code></dd>
        <dt class="eyebrow">Base</dt>
        <dd>
          {#if mode === 'join'}
            <code>{base}</code>
          {:else}
            <input bind:value={base} aria-label="Base branch" spellcheck="false" autocomplete="off" disabled={busy} />
          {/if}
        </dd>
      </dl>
      {#if draft.warning && base.trim() === draft.base}<p class="note">{draft.warning}</p>{/if}
      {#if draft.gh_problem}<p class="note">{draft.gh_problem}</p>{/if}
      {#if !pushOnly}
        <label class="field">
          <span class="eyebrow">Title</span>
          <input bind:value={title} disabled={busy} />
        </label>
        <label class="field">
          <span class="eyebrow">Description</span>
          <textarea bind:value={body} rows="12" spellcheck="false" disabled={busy}></textarea>
        </label>
      {/if}
      {#if pushed}
        <p class="sent">
          <Icon name="check" size={13} stroke={2} />
          <span>{pushed.branch} is on {pushed.remote}.{#if !open && !pushed.compare_url} Open the pull request on your git host.{/if}</span>
          {#if mode !== 'join' && pushed.compare_url}
            {@const url = pushed.compare_url}
            <button type="button" class="btn sm" onclick={() => openExternal(url)}><Icon name="external" size={12} />Open the compare page</button>
          {/if}
        </p>
      {/if}
    </div>
    <footer>
      <button type="button" class="btn sm ghost" onclick={close}>{pushed ? 'Close' : 'Cancel'}</button>
      {#if !pushed}
        <button type="submit" class="btn sm primary" disabled={busy || !base.trim() || (!pushOnly && !title.trim())}>
          {pushOnly ? `Push branch to ${draft.remote}` : 'Create pull request'}
        </button>
      {/if}
    </footer>
  </form>
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: 14px; padding: 22px 26px; }
  .desc { color: var(--ink-2); font-size: var(--t-md); }
  .where { display: grid; grid-template-columns: max-content 1fr; align-items: center; gap: 8px 14px; margin: 0; }
  .where dd { margin: 0; min-width: 0; }
  .where code { font-size: 12px; color: var(--ink-1); overflow-wrap: anywhere; }
  .where input { width: 100%; font-family: var(--font-mono); font-size: 12px; }
  .note { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--amber-ink, var(--ink-2)); }
  .field { display: flex; flex-direction: column; gap: 8px; }
  textarea { font-family: var(--font-mono); font-size: 12px; line-height: 1.5; }
  .sent { display: flex; align-items: center; gap: 8px; font-size: var(--t-md); color: var(--ok); }
  .sent span { flex: 1; min-width: 0; overflow-wrap: anywhere; }
  footer { display: flex; justify-content: flex-end; gap: 8px; padding: 0 26px 22px; }
</style>
