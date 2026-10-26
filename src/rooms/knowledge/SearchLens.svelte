<!--
  Code search over Codly's index: paths, symbols and code, each hit with the mechanical evidence
  for it (never model prose) and a confidence. Hover a hit to highlight who touched it.
-->
<script lang="ts">
  import { api } from '../../lib/api';
  import type { SearchResult } from '../../lib/gen/SearchResult';
  import { copyText } from '../../lib/state/actions';
  import { ui } from '../../lib/state/ui.svelte';
  import EmptyState from '../../components/EmptyState.svelte';
  import Icon from '../../components/Icon.svelte';
  import Provenance from '../../components/Provenance.svelte';

  let q = $state('');
  let res = $state<SearchResult | null>(null);
  let busy = $state(false);
  let el: HTMLInputElement | undefined = $state();
  let seq = 0;
  $effect(() => {
    const query = q.trim();
    if (query.length < 2) {
      res = null;
      return;
    }
    const my = ++seq;
    busy = true;
    const t = setTimeout(() => {
      api.search(query).then((r) => my === seq && (res = r)).catch(() => my === seq && (res = null)).finally(() => my === seq && (busy = false));
    }, 160);
    return () => clearTimeout(t);
  });
  $effect(() => {
    el?.focus();
  });
  const CONF: Record<string, string> = { exact: 'Exact match', high: 'High confidence', medium: 'Medium confidence', low: 'Low confidence' };
</script>

<div class="sl">
  <label class="q"><Icon name="search" size={16} /><input bind:this={el} bind:value={q} placeholder="Search the code — a symbol, a path, an idea" spellcheck="false" />{#if busy}<span class="muted">Searching…</span>{/if}</label>
  <div class="scroll">
    {#if !res}
      <EmptyState compact title="Ask the index." body="The same search agents use: paths, symbols and 60-line chunks, ranked, with the evidence for every hit." />
    {:else}
      <p class="head"><span class="conf c-{res.confidence}">{CONF[res.confidence]}</span><span class="muted num">{res.citations.length} hits · {res.took_ms} ms</span></p>
      <ol class="hits">
        {#each res.citations as c (c.id)}
          <li>
            <button class="hit" onpointerenter={() => ui.setHover({ path: c.path })} onpointerleave={() => ui.setHover(null)} onclick={() => ui.selectFile(c.path)}>
              <span class="cid">{c.id}</span>
              <span class="body">
                <span class="path"><code>{c.path}</code><span class="ln num">:{c.start_line}–{c.end_line}</span>{#if c.symbol}<span class="sym">{c.symbol}</span>{/if}</span>
                <span class="why">{#each c.why as w (w)}<span class="w">{w}</span>{/each}<Provenance kind="observed" compact /></span>
              </span>
              <span class="score num">{c.score.toFixed(1)}</span>
            </button>
            <span class="acts">
              <button class="icon-btn sm" aria-label="Mention in chat" data-tip="Ask the Lead about it" onclick={() => ui.seedComposer(ui.request, `\`${c.path}:${c.start_line}-${c.end_line}\` `)}><Icon name="chat" size={13} /></button>
              <button class="icon-btn sm" aria-label="Copy path" data-tip="Copy path" onclick={() => copyText(c.path, 'Path copied')}><Icon name="copy" size={13} /></button>
            </span>
          </li>
        {/each}
      </ol>
      {#if res.fallback_rg.length}<p class="rg">Try: {#each res.fallback_rg as r (r)}<code>{r}</code>{/each}</p>{/if}
    {/if}
  </div>
</div>

<style>
  .sl { flex: 1; min-height: 0; display: flex; flex-direction: column; padding: 0 28px 24px; }
  .q { display: flex; align-items: center; gap: 10px; height: 48px; padding: 0 18px; border-radius: var(--r-full); background: var(--surface-2); color: var(--ink-3); margin-bottom: 18px; }
  .q input { flex: 1; border: 0; outline: 0; background: transparent; font-size: 15px; color: var(--ink-1); }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; }
  .head { display: flex; align-items: center; gap: 12px; margin-bottom: 10px; font-size: var(--t-sm); }
  .conf { font-weight: var(--w-semi); }
  .c-exact, .c-high { color: var(--ok); }
  .c-medium { color: var(--amber-ink); }
  .c-low { color: var(--bad); }
  .hits { display: flex; flex-direction: column; gap: 4px; }
  .hits li { position: relative; }
  .hit { display: flex; align-items: center; gap: 14px; width: 100%; padding: 10px 90px 10px 14px; border-radius: var(--r-md); text-align: left; }
  .hit:hover { background: var(--surface-1); }
  .cid { font-family: var(--font-mono); font-size: 11px; color: var(--ink-3); width: 26px; }
  .body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .path { display: flex; align-items: baseline; gap: 2px; min-width: 0; }
  .path code { font-size: 12.5px; color: var(--ink-1); }
  .ln { font-family: var(--font-mono); font-size: 11.5px; color: var(--ink-3); }
  .sym { margin-left: 10px; font-size: var(--t-sm); color: var(--aurora-m); }
  .why { display: flex; flex-wrap: wrap; gap: 5px; align-items: center; }
  .w { height: 19px; padding: 0 7px; border-radius: var(--r-full); background: var(--surface-2); font-size: var(--t-xs); line-height: 19px; color: var(--ink-2); }
  .score { font-size: var(--t-xs); color: var(--ink-3); }
  .acts { position: absolute; right: 44px; top: 50%; transform: translateY(-50%); display: flex; gap: 2px; opacity: 0; transition: opacity var(--d-2); }
  .hits li:hover .acts, .acts:focus-within { opacity: 1; }
  .rg { margin-top: 14px; font-size: var(--t-sm); color: var(--ink-3); display: flex; gap: 8px; }
  .rg code { font-size: 11.5px; color: var(--ink-2); }
</style>
