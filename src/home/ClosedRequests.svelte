<!--
  The requests you closed, to read again. Closing a request stops its Lead and cancels its
  unfinished plans, but nothing that was said is deleted: Codly keeps it under an archive
  thread. This view lists them, newest first, and shows one as it was. It is read-only: there
  is no composer, and a closed request does not start again. ⌘F finds text in it. A request
  closed while this view is open joins the list as soon as the core has closed it.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from '../lib/api';
  import type { ClosedRequest } from '../lib/gen/ClosedRequest';
  import type { ClosedTranscript } from '../lib/gen/ClosedTranscript';
  import { store } from '../lib/state/store.svelte';
  import { CLOSED_LIST, ui } from '../lib/state/ui.svelte';
  import { ago, clock, fullDate, plural } from '../lib/util/format';
  import { oneLine } from '../lib/util/sanitize';
  import EmptyState from '../components/EmptyState.svelte';
  import Icon from '../components/Icon.svelte';
  import LoadError from '../components/LoadError.svelte';
  import Markdown from '../components/Markdown.svelte';

  interface Props {
    /** `CLOSED_LIST` for the list, or one archive thread ("closed:<thread>:<ms>"). */
    thread: string;
  }
  let { thread }: Props = $props();

  const listing = $derived(thread === CLOSED_LIST);
  let list = $state<ClosedRequest[] | null>(null);
  let read = $state<ClosedTranscript | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);
  let seq = 0;

  function load() {
    const my = ++seq;
    const one = listing ? null : thread;
    error = null;
    loading = true;
    // The list is asked for in both views: one request's title and time come from its row.
    Promise.all([api.closedRequests(), one ? api.closedRequest(one) : null])
      .then(([l, r]) => {
        if (my !== seq) return;
        list = l;
        read = r;
      })
      .catch((e) => my === seq && (error = e instanceof Error ? e.message : String(e)))
      .finally(() => my === seq && (loading = false));
  }
  $effect(() => {
    void thread;
    void store.workspace?.id;
    list = null;
    read = null;
    load();
  });
  // A request you closed a moment ago is not closed until the time for Undo is up (see
  // lib/state/closing.ts): when the core has closed it, the list is read again and shows it.
  let landed = ui.closedRev;
  $effect(() => {
    if (ui.closedRev === landed) return;
    landed = ui.closedRev;
    untrack(load);
  });

  /** A request of this office you closed a moment ago: the core closes it when the time for Undo is up. */
  const closing = $derived(ui.closingRequests.some((c) => c.office === (store.workspace?.id ?? null)));
  const row = $derived(listing ? null : (list?.find((c) => c.thread === thread) ?? null));
  const titleOf = (c: ClosedRequest) => (c.title ? oneLine(c.title, 140) : 'A request with no message from you');
  /** Who wrote a message: you, Codly, or an agent by the name it had. */
  const nameOf = (author: string) => read?.authors[author] ?? store.agentMap.get(author)?.name ?? 'An agent';
  const back = () => ui.openRequest(ui.request);
</script>

<div class="closed">
  <header class="top">
    {#if listing}
      <p class="what"><b>Closed requests</b><span>You can read them again. They do not start again.</span></p>
    {:else}
      <button class="btn ghost sm" onclick={() => ui.openThread(CLOSED_LIST)}><Icon name="arrow-left" size={12} />Closed requests</button>
      <p class="what truncate"><b class="truncate">{row ? titleOf(row) : 'Closed request'}</b>{#if row}<span data-tip={fullDate(row.closed_at)}>Closed {ago(row.closed_at, ui.now)} · read-only</span>{/if}</p>
    {/if}
    <button class="btn ghost sm" onclick={back}>Back to your requests</button>
  </header>

  <div class="scroll" data-find-root>
    {#if error}
      <div class="pad"><LoadError what={listing ? 'the closed requests' : 'this closed request'} {error} {loading} onretry={load} /></div>
    {:else if listing}
      {#if list && !list.length && closing}
        <EmptyState title="Closing your request…" body="It shows here in a few seconds, once the time for Undo is up." />
      {:else if list && !list.length}
        <EmptyState title="No closed requests." body="When you close a request, what was said in it is kept here." />
      {:else if list}
        <ul class="rows">
          {#each list as c (c.thread)}
            <li>
              <button class="row" onclick={() => ui.openThread(c.thread)}>
                <span class="t truncate">{titleOf(c)}</span>
                <span class="m num">{plural(c.messages, 'message')} · closed {ago(c.closed_at, ui.now)}</span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    {:else if read}
      {#if read.total > read.messages.length}
        <p class="note">This request has {read.total} messages. The newest {read.messages.length} are shown.</p>
      {/if}
      {#if !read.messages.length}
        <EmptyState title="Nothing to read." body="This closed request has no messages." />
      {:else}
        <ol class="items">
          {#each read.messages as m (m.id)}
            <li class="it">
              {#if m.author === 'user'}
                <div class="user selectable"><p>{m.text}</p></div>
              {:else if m.author === 'codly'}
                <p class="sys">{m.text}<time class="num">{clock(m.ts)}</time></p>
              {:else}
                <div class="who"><span class="name">{nameOf(m.author)}</span><time class="num">{clock(m.ts)}</time></div>
                <div class="say"><Markdown text={m.text} /></div>
              {/if}
            </li>
          {/each}
        </ol>
      {/if}
    {/if}
  </div>
</div>

<style>
  .closed { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .top { display: flex; align-items: center; gap: 10px; padding: 10px 14px 10px 22px; box-shadow: inset 0 -1px 0 var(--line); flex-shrink: 0; }
  .what { flex: 1; min-width: 0; display: flex; align-items: baseline; gap: 8px; font-size: var(--t-md); }
  .what b { color: var(--ink-1); font-weight: var(--w-semi); }
  .what span { font-size: var(--t-sm); color: var(--ink-3); white-space: nowrap; }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: 24px 0 28px; }
  .pad { padding: 0 32px; }
  .rows { display: flex; flex-direction: column; gap: 2px; padding: 0 20px; }
  .row { display: flex; align-items: baseline; gap: 12px; width: 100%; min-height: 40px; padding: 8px 12px; border-radius: var(--r-md); text-align: left; }
  .row:hover, .row:focus-visible { background: var(--surface-2); }
  .t { flex: 1; font-weight: var(--w-med); color: var(--ink-1); }
  .m { font-size: var(--t-sm); color: var(--ink-3); white-space: nowrap; }
  .note { padding: 0 32px 16px; font-size: var(--t-sm); color: var(--ink-3); text-align: center; }
  .items { display: flex; flex-direction: column; gap: 20px; padding: 0 32px; }
  .it { min-width: 0; }
  .user { display: flex; justify-content: flex-end; }
  .user p { max-width: 82%; padding: 10px 16px 11px; border-radius: 20px 20px 6px 20px; background: var(--surface-3); font-size: var(--t-lg); line-height: var(--lh-lg); white-space: pre-wrap; overflow-wrap: anywhere; }
  .sys { display: flex; justify-content: center; gap: 8px; font-size: var(--t-sm); color: var(--ink-3); }
  .sys time { color: var(--ink-4); }
  .who { display: flex; align-items: center; gap: 8px; margin-bottom: 6px; }
  .name { font-weight: var(--w-semi); font-size: var(--t-md); }
  .who time { font-size: var(--t-xs); color: var(--ink-3); }
  .say { max-width: 66ch; }
</style>
