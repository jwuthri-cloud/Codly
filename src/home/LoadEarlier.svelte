<!--
  "Load earlier", at the top of a conversation whose history goes further back than what is
  shown (Codly opens a repo with its newest events only). It shows only when there is more, and
  the line you were reading stays where it is while older ones come in above it.
-->
<script lang="ts">
  import { tick } from 'svelte';
  import { store } from '../lib/state/store.svelte';

  interface Props {
    thread: string;
    /** The conversation's scroll box. */
    scroller: HTMLElement | undefined;
  }
  let { thread, scroller }: Props = $props();

  const earlier = $derived(store.earlier[thread]);
  let failed = $state<string | null>(null);
  // Asked once per thread, and again when the store forgets the answer (another office, rows
  // that left its log).
  $effect(() => {
    if (!store.earlier[thread]) void store.peekEarlier(thread);
  });
  $effect(() => {
    void thread;
    failed = null;
  });

  async function load() {
    failed = null;
    const first = scroller?.querySelector('li');
    const top = first?.getBoundingClientRect().top ?? 0;
    const height = scroller?.scrollHeight ?? 0;
    try {
      await store.loadEarlier(thread);
    } catch (e) {
      failed = e instanceof Error ? e.message : String(e);
      return;
    }
    await tick();
    if (!scroller) return;
    // By the first row if it is still the same one, else by how much taller the list got.
    scroller.scrollTop += first?.isConnected ? first.getBoundingClientRect().top - top : scroller.scrollHeight - height;
  }
</script>

{#if earlier?.more}
  <div class="earlier">
    <button class="btn ghost sm" disabled={earlier.loading} onclick={load}>{earlier.loading ? 'Loading…' : 'Load earlier'}</button>
    {#if failed}<p class="bad" role="alert">Couldn’t load earlier history — {failed}</p>{/if}
  </div>
{/if}

<style>
  .earlier { display: flex; flex-direction: column; align-items: center; gap: 6px; padding: 0 32px 20px; }
  .bad { font-size: var(--t-sm); color: var(--bad); text-align: center; }
</style>
