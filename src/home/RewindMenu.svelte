<!--
  "Undo a turn of <agent>": a small layer over the composer (Esc Esc, `/undo`, the palette's
  row). It lists the agent's newest turns, each with how many files a restore would change, and
  puts the files back to how they were before the turn you pick. ↑ ↓ pick a turn, → (or a press
  on its file count) lists its files, each of which can go back alone; ↵ shows the confirm line
  in place and a second ↵ restores; Esc closes one level. Files only: the conversation inside
  the CLI stays, and the core tells the agent which files went back. A refusal of the core is
  shown here in its words, with nothing changed. The list is read once, when the layer opens
  (it starts git). The rules are in rewind.ts.
-->
<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api } from '../lib/api';
  import type { Agent } from '../lib/gen/Agent';
  import type { AgentCheckpoints } from '../lib/gen/AgentCheckpoints';
  import { ui } from '../lib/state/ui.svelte';
  import { plural } from '../lib/util/format';
  import Icon from '../components/Icon.svelte';
  import { openSheet } from './composerActions';
  import { confirmLine, namesLine, pick, rewindItems, rewindKey, rewindOff, rewindRows, rewindStart, toggleFiles, type RewindAct, type RewindNav } from './rewind';
  import { putBack, rewindLayer } from './rewindLayer.svelte';

  interface Props {
    agent: Agent;
    onclose: () => void;
  }
  let { agent, onclose }: Props = $props();

  let el: HTMLDivElement | undefined = $state();
  /** The core's answer; null while it is read. */
  let kept = $state.raw<AgentCheckpoints | null>(null);
  /** The core refused to read or to restore: its words. */
  let error = $state<string | null>(null);
  let busy = $state(false);
  let nav = $state<RewindNav>({ at: 0, open: null, asking: null });

  const rows = $derived(kept ? rewindRows(kept, ui.now) : []);
  const off = $derived(kept ? rewindOff(kept, agent.name) : null);
  const items = $derived(kept ? rewindItems(rows, nav.open) : []);

  onMount(() => {
    // Whose list this is: by the time it leaves the screen, the composer may show another agent.
    const shown = agent.id;
    void (async () => {
      try {
        kept = await api.agentCheckpoints(agent.id);
        nav = rewindStart(rewindRows(kept, ui.now));
      } catch (e) {
        error = e instanceof Error ? e.message : String(e);
      }
    })();
    tick().then(() => el?.focus());
    const outside = (e: PointerEvent) => {
      if (el && !el.contains(e.target as Node)) onclose();
    };
    window.addEventListener('pointerdown', outside, true);
    return () => {
      window.removeEventListener('pointerdown', outside, true);
      // Gone from the screen with the list open (a key took you to another conversation or room).
      rewindLayer.release(shown);
    };
  });

  async function act(a: RewindAct) {
    if (!a) return;
    if (a.kind === 'close') return onclose();
    if (a.kind === 'sheet') {
      onclose();
      return openSheet(agent, 'tabs');
    }
    const row = rows.find((r) => r.n === a.n);
    if (!row || busy) return;
    busy = true;
    try {
      await putBack(agent.id, a.n, a.path, row.label);
      onclose();
    } catch (e) {
      // Nothing changed: the list stays, with the core's reason.
      error = e instanceof Error ? e.message : String(e);
      nav = { ...nav, asking: null };
    } finally {
      busy = false;
    }
  }

  function key(e: KeyboardEvent) {
    if (e.isComposing || (e.repeat && e.key === 'Enter')) return;
    if (!['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Enter', 'Escape'].includes(e.key)) return;
    e.preventDefault();
    e.stopPropagation();
    // While the list is read only Esc does something.
    if (!kept) {
      if (e.key === 'Escape') onclose();
      return;
    }
    if (busy) return;
    const r = rewindKey(nav, rows, e.key);
    nav = r.nav;
    void act(r.act);
  }

  function press(at: number) {
    if (busy) return;
    const r = pick(nav, rows, at);
    nav = r.nav;
    void act(r.act);
    el?.focus();
  }

  const itemAt = (kind: string, n: number, path: string | null = null) =>
    items.findIndex((it) => it.kind === kind && (it.kind === 'all' || (it.n === n && (it.kind !== 'file' || it.path === path))));
</script>

<div class="rewind" bind:this={el} role="dialog" aria-label="Undo a turn of {agent.name}" tabindex="-1" onkeydown={key}>
  <div class="head">
    <span class="title">Undo a turn of {agent.name}</span>
    <span class="sub">Files only: the conversation stays</span>
  </div>
  {#if error}<p class="bad" role="alert">{error}</p>{/if}
  {#if !kept}
    {#if !error}<p class="small">Reading the turns…</p>{/if}
  {:else}
    {@const all = itemAt('all', 0)}
    {#if off}
      <p class="small">{off}</p>
    {:else}
      <ul class="turns" role="listbox" aria-label="Turns of {agent.name}">
        {#each rows as r (r.n)}
          {@const at = itemAt('turn', r.n)}
          <li>
            <div class="row" class:active={nav.at === at} class:empty={!r.changed} role="option" aria-selected={nav.at === at} aria-disabled={!r.changed}>
              <button class="pick" disabled={!r.changed} onclick={() => press(at)}>
                <span class="label truncate">{r.label}</span>
                <span class="names truncate">{r.changed ? namesLine(r.files, r.changed) : 'nothing to put back'}</span>
              </button>
              <time>{r.age}</time>
              {#if r.changed}
                <button class="count" aria-expanded={nav.open === r.n} data-tip="Show the files: each one can go back alone" data-tip-kbd="→" onclick={() => { nav = toggleFiles(nav, rows, r.n); el?.focus(); }}>
                  {plural(r.changed, 'file')}<span class="chev" class:open={nav.open === r.n}><Icon name="chevron-right" size={11} stroke={2} /></span>
                </button>
              {/if}
            </div>
            {#if nav.asking?.n === r.n && nav.asking.path === null}
              <p class="ask">{confirmLine(agent.name, r, null)} <span class="keys">{busy ? 'Putting the files back…' : '↵ put back · Esc cancel'}</span></p>
            {/if}
            {#if nav.open === r.n}
              <ul class="files">
                {#each r.files as f (f)}
                  {@const fat = itemAt('file', r.n, f)}
                  <li>
                    <div class="row file" class:active={nav.at === fat} role="option" aria-selected={nav.at === fat}>
                      <code class="path truncate" data-tip={f}>{f}</code>
                      <button class="btn sm ghost" disabled={busy} onclick={() => press(fat)}>Put this file back</button>
                    </div>
                    {#if nav.asking?.n === r.n && nav.asking.path === f}
                      <p class="ask">{confirmLine(agent.name, r, f)} <span class="keys">{busy ? 'Putting the files back…' : '↵ put back · Esc cancel'}</span></p>
                    {/if}
                  </li>
                {/each}
                {#if r.changed > r.files.length}<li class="small">… and {r.changed - r.files.length} more: they go back with the whole turn.</li>{/if}
              </ul>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
    <button class="row all" class:active={nav.at === all} onclick={() => press(all)}>All checkpoints…</button>
  {/if}
</div>

<style>
  .rewind { position: absolute; left: 6px; bottom: calc(100% + 8px); width: min(520px, calc(100% - 12px)); max-height: min(440px, 55vh); overflow-y: auto; padding: 6px; border-radius: var(--r-lg); background: var(--pop); box-shadow: var(--pop-shadow); z-index: var(--z-pop); outline: none; animation: rise-in var(--d-3) var(--spring); }
  .head { display: flex; align-items: baseline; gap: 8px; padding: 6px 10px 6px; }
  .title { font-size: var(--t-md); font-weight: var(--w-semi); color: var(--ink-1); }
  .sub { font-size: var(--t-sm); color: var(--ink-3); }
  .small { padding: 4px 10px 8px; font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .bad { padding: 4px 10px 8px; font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--bad); }
  .row { display: flex; align-items: center; gap: 8px; width: 100%; min-height: 40px; padding: 0 6px 0 0; border-radius: var(--r-md); text-align: left; }
  .row.active { background: var(--surface-3); }
  .pick { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; padding: 5px 0 5px 10px; text-align: left; }
  .label { font-size: var(--t-md); font-weight: var(--w-med); color: var(--ink-1); }
  .names { font-size: var(--t-sm); color: var(--ink-3); }
  .row.empty .label { color: var(--ink-3); }
  time { flex-shrink: 0; font-size: var(--t-xs); color: var(--ink-3); white-space: nowrap; }
  .count { flex-shrink: 0; display: inline-flex; align-items: center; gap: 3px; height: 26px; padding: 0 6px 0 8px; border-radius: var(--r-full); font-size: var(--t-sm); color: var(--ink-2); white-space: nowrap; }
  .count:hover { background: var(--surface-2); color: var(--ink-1); }
  .chev { display: inline-flex; transition: transform var(--d-2) var(--ease-out); }
  .chev.open { transform: rotate(90deg); }
  .files { margin: 0 0 4px 16px; }
  .row.file { min-height: 32px; padding-left: 10px; }
  .path { flex: 1; min-width: 0; font-size: 11.5px; color: var(--ink-1); }
  .ask { margin: 2px 6px 6px 10px; padding: 8px 10px; border-radius: var(--r-md); background: var(--surface-2); font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-1); }
  .keys { color: var(--ink-3); white-space: nowrap; }
  .all { min-height: 34px; padding-left: 10px; font-size: var(--t-sm); color: var(--ink-2); }
</style>
