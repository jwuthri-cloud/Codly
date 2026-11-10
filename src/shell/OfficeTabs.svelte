<!--
  Office tabs, at the top of the rail: one tile per repo you keep at hand, the open one marked,
  a teal count while its agents are on tasks, an amber one for what waits for you there. Every
  office with work keeps working; a click only changes where you look. ⏸ pauses one office
  (running tasks finish, nothing new starts), × closes its tab and pauses it, + opens another repo.
-->
<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { openWorkspace } from '../lib/state/actions';
  import { offices, type OfficeTab } from '../lib/state/offices.svelte';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { tildify } from '../lib/util/sanitize';
  import Icon from '../components/Icon.svelte';

  const ACTIVE = new Set(['running', 'verifying', 'reviewing', 'changes_requested', 'merging']);
  const ws = $derived(store.workspace);
  const here = $derived(store.tasks.filter((t) => ACTIVE.has(t.status)).length);

  // The open repo gets a tab; a switch refreshes the others' counts.
  $effect(() => {
    const w = ws;
    untrack(() => {
      if (w) offices.add(w);
      void offices.refresh();
    });
  });
  onMount(() => {
    const t = setInterval(() => document.hidden || void offices.refresh(), 5000);
    return () => clearInterval(t);
  });

  const busy = (t: OfficeTab) => (t.id === ws?.id ? here : (offices.working.get(t.id) ?? 0));
  /** Approvals, plans to approve and blocking questions in an office you are not looking at (the open one shows its own). */
  const waits = (t: OfficeTab) => (t.id === ws?.id ? 0 : (offices.needs.get(t.id) ?? 0));
  function initials(name: string): string {
    const words = name.replace(/^.*\//, '').split(/[^a-z0-9]+/i).filter(Boolean);
    const s = words.length > 1 ? words[0][0] + words[1][0] : (words[0] ?? '?').slice(0, 2);
    return s[0].toUpperCase() + s.slice(1).toLowerCase();
  }
  const tasks = (n: number) => `${n} task${n === 1 ? '' : 's'}`;
  const state = (n: number, p: boolean, w: number) => (p ? ` · paused${n ? `, ${tasks(n)} finishing` : ''}` : n ? ` · ${tasks(n)} in flight` : '') + (w ? ` · ${w} waiting for you` : '');

  function close(t: OfficeTab) {
    const i = offices.tabs.indexOf(t);
    offices.close(t.id);
    if (t.id !== ws?.id) return;
    const next = offices.tabs[Math.min(i, offices.tabs.length - 1)];
    if (next) void openWorkspace(next.path);
  }
</script>

<ul class="offices" aria-label="Offices">
  {#each offices.tabs as t (t.id)}
    {@const n = busy(t)}
    {@const on = t.id === ws?.id}
    {@const p = offices.paused(t.id)}
    {@const w = waits(t)}
    <li class="tab" class:on class:paused={p}>
      <button
        class="office"
        aria-current={on ? 'page' : undefined}
        aria-label="{t.name}{state(n, p, w)}"
        data-tip="{t.name} — {tildify(t.path)}{state(n, p, w)}"
        onclick={() => !on && openWorkspace(t.path)}
      >
        <span class="ini">{initials(t.name)}</span>
        {#if ui.railOpen}<span class="label">{t.name}</span>{/if}
        {#if p}<span class="n num p" aria-hidden="true">⏸</span>{:else if n}<span class="n num">{n}</span>{/if}
        {#if w}<span class="n num w">{w}</span>{/if}
      </button>
      <button
        class="pp"
        aria-label="{p ? 'Resume' : 'Pause'} {t.name}"
        data-tip={p ? `Resume ${t.name}: new tasks start again` : `Pause ${t.name}: running tasks finish, nothing new starts`}
        onclick={() => offices.pause(t.id, !p)}>{p ? '▶' : '⏸'}</button>
      {#if offices.tabs.length > 1}
        <button
          class="x"
          aria-label="Close the {t.name} tab"
          data-tip={n ? `Close and pause ${t.name}: its ${tasks(n)} finish, nothing new starts` : `Close and pause ${t.name}: reopen it to resume`}
          onclick={() => close(t)}><Icon name="x" size={10} /></button>
      {/if}
    </li>
  {/each}
  <li>
    <button class="office add" aria-label="Open another repository" data-tip="Open another repository: every office keeps working" data-tip-kbd="⌘O" onclick={() => openWorkspace()}>
      <Icon name="plus" size={16} />
      {#if ui.railOpen}<span class="label">Open repo…</span>{/if}
    </button>
  </li>
</ul>

<style>
  .offices { display: flex; flex-direction: column; gap: 4px; max-height: 32vh; overflow-y: auto; scrollbar-width: none; padding-bottom: 10px; margin-bottom: 10px; border-bottom: 1px solid var(--line); }
  .tab { position: relative; }
  .office { position: relative; display: grid; place-items: center; width: 38px; height: 38px; border-radius: 12px; color: var(--ink-3); transition: background-color var(--d-2) var(--ease-out), color var(--d-2) var(--ease-out); }
  .office:hover { color: var(--ink-1); background: var(--surface-2); }
  .on .office { color: var(--ink-1); background: var(--surface-3); cursor: default; }
  .ini { display: grid; place-items: center; width: 26px; height: 26px; border-radius: 8px; font-size: 11.5px; font-weight: 600; letter-spacing: 0.02em; background: var(--surface-2); box-shadow: inset 0 0 0 1px var(--line); }
  .on .ini { background: var(--surface-1); box-shadow: inset 0 0 0 1.5px var(--ink-2); }
  .n { position: absolute; right: 1px; bottom: 1px; min-width: 14px; height: 14px; padding: 0 3px; border-radius: 7px; background: var(--live); color: var(--night); font-size: 9.5px; font-weight: 700; line-height: 14px; text-align: center; box-shadow: 0 0 0 2px var(--glass-solid); }
  /* Hidden by opacity, not display: an element that is not rendered cannot take keyboard focus.
     :focus-visible, not :focus-within: in Chromium a clicked tile keeps focus, and they would stay. */
  .x { position: absolute; top: 0; left: 0; display: grid; place-items: center; width: 15px; height: 15px; border-radius: 50%; color: var(--ink-2); background: var(--glass-solid); box-shadow: 0 0 0 1px var(--line-2); opacity: 0; transition: opacity var(--d-1) var(--ease-out); }
  .x:hover { color: var(--ink-1); }
  .pp { position: absolute; bottom: 0; left: 0; display: grid; place-items: center; width: 15px; height: 15px; border-radius: 50%; font-size: 7.5px; color: var(--ink-2); background: var(--glass-solid); box-shadow: 0 0 0 1px var(--line-2); opacity: 0; transition: opacity var(--d-1) var(--ease-out); }
  .pp:hover { color: var(--ink-1); }
  .tab:hover .x, .tab:has(:focus-visible) .x, .tab:hover .pp, .tab:has(:focus-visible) .pp { opacity: 1; }
  .paused .ini { opacity: 0.55; }
  .n.p { background: var(--surface-3); color: var(--ink-2); font-size: 7.5px; }
  /* What waits for you: amber, top right (the task count keeps the bottom), paused or not.
     The rail's own waiting badge (Rail.svelte .b-wait): dark ink on amber in both themes. */
  .n.w { top: 1px; bottom: auto; background: var(--amber); color: #241504; }
  .label { font-size: 13px; font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; }
  :global(.rail.open) .office { width: 100%; grid-template-columns: 26px 1fr auto; justify-items: start; gap: 9px; padding: 0 40px 0 6px; }
  :global(.rail.open) .add { padding-right: 6px; }
  :global(.rail.open) .n { position: static; box-shadow: none; }
  /* Open rail: on the tile's corner, so the row keeps its three columns. */
  :global(.rail.open) .n.w { position: absolute; top: 1px; left: 24px; right: auto; box-shadow: 0 0 0 2px var(--glass-solid); }
  :global(.rail.open) .x { top: 50%; left: auto; right: 4px; margin-top: -7.5px; }
  :global(.rail.open) .pp { top: 50%; bottom: auto; left: auto; right: 22px; margin-top: -7.5px; }
</style>
