<!--
  The observatory: the office full-bleed behind everything, a slim rail, the top bar with the crew,
  and one stage (Home or a room). The agent/task sheet slides in from the right only when
  something is selected, and the stage makes room for it.
-->
<script lang="ts">
  import { isTauriApp } from '../lib/api';
  import { diffstats } from '../lib/state/diffstats.svelte';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import Home from '../home/Home.svelte';
  import PlanRoom from '../rooms/plan/PlanRoom.svelte';
  import ChangesRoom from '../rooms/changes/ChangesRoom.svelte';
  import KnowledgeRoom from '../rooms/knowledge/KnowledgeRoom.svelte';
  import TerminalRoom from '../rooms/terminal/TerminalRoom.svelte';
  import Sheet from '../sheet/Sheet.svelte';
  import OfficeLayer from './OfficeLayer.svelte';
  import Rail from './Rail.svelte';
  import TopBar from './TopBar.svelte';

  interface Props {
    theme: 'dark' | 'light';
  }
  let { theme }: Props = $props();
  const tauri = isTauriApp();

  const SHEET = 404;
  const frameW = $derived(ui.width - (ui.railW + 28) - 14 - (ui.sheetOpen ? SHEET + 14 : 0));
  const narrow = $derived(frameW < 960);

  // Keep diffstats warm for finished work: narration, cards and Changes read the same numbers.
  $effect(() => {
    void store.rev;
    for (const t of store.tasks) {
      const marks = store.taskMarks[t.id] ?? [];
      if (t.status === 'done' || marks.some((m) => m.kind === 'committed')) diffstats.ensureTask(t.id, `${t.status}:${marks.length}`);
    }
    for (const p of store.plans) {
      const tasks = store.tasks.filter((t) => t.plan_id === p.id);
      const done = tasks.filter((t) => t.status === 'done').length;
      if (done) diffstats.ensurePlan(p.id, `${p.status}:${done}`);
    }
  });
</script>

<div class="shell mode-{ui.mode} room-{ui.room}" class:focus={ui.focus} class:sheet={ui.sheetOpen} class:tauri style:--rail-w="{ui.railW}px">
  <div class="world-wrap" class:hidden={ui.mode === 'work'}><OfficeLayer {theme} /></div>
  <div class="veil" aria-hidden="true"></div>
  <Rail />
  <div class="frame">
    <TopBar {narrow} />
    <main class="stage">
      {#if ui.room === 'home'}
        <Home />
      {:else}
        {#key ui.room}
          <section class="room glass" aria-label={ui.room}>
            <svelte:boundary>
              {#if ui.room === 'plan'}<PlanRoom />
              {:else if ui.room === 'changes'}<ChangesRoom />
              {:else if ui.room === 'knowledge'}<KnowledgeRoom />
              {:else}<TerminalRoom />{/if}
              {#snippet failed(error)}<p style="padding:28px;color:var(--bad)">This room failed to open: {error instanceof Error ? error.message : String(error)}</p>{/snippet}
            </svelte:boundary>
          </section>
        {/key}
      {/if}
    </main>
  </div>
  <Sheet />
</div>

<style>
  .shell { position: fixed; inset: 0; background: radial-gradient(120% 80% at 50% 120%, var(--night-glow), var(--night) 60%); }
  .world-wrap { position: absolute; inset: 0; z-index: var(--z-world); transition: filter var(--d-5) var(--ease-out), opacity var(--d-5) var(--ease-out); }
  .world-wrap.hidden { visibility: hidden; opacity: 0; }
  .veil { position: absolute; inset: 0; z-index: 1; pointer-events: none; background: transparent; transition: background-color var(--d-5) var(--ease-out); }
  .focus .world-wrap { filter: blur(7px) saturate(0.7); }
  .focus .veil { background: var(--world-focus); }
  .mode-work .veil { background: var(--night); }
  .mode-office .world-wrap { filter: none; }
  .mode-office .veil { background: transparent; }
  /* rooms sit on a quieter office than Home */
  .shell:not(.room-home) .veil { background: color-mix(in srgb, var(--night) 35%, transparent); }
  .shell.mode-work .veil { background: var(--night); }

  .frame { position: absolute; z-index: var(--z-ui); top: 0; bottom: 0; left: calc(var(--rail-w) + var(--gutter) * 2); right: var(--gutter); display: flex; flex-direction: column; pointer-events: none; transition: right var(--d-4) var(--spring), left var(--d-4) var(--spring); }
  .sheet .frame { right: calc(var(--sheet-w) + var(--gutter) * 2); }
  .frame > :global(*) { pointer-events: auto; }
  .tauri .frame :global(.top) { padding-left: 10px; }
  .stage { position: relative; flex: 1; min-height: 0; padding-bottom: var(--gutter); display: flex; flex-direction: column; pointer-events: none; }
  .stage > :global(.home), .stage > :global(.room) { pointer-events: auto; }
  .room { flex: 1; min-height: 0; display: flex; flex-direction: column; border-radius: var(--r-xl); overflow: hidden; animation: room-in var(--d-4) var(--spring); background: var(--glass-strong); }
  .mode-work .room { background: var(--glass-solid); }
  @keyframes room-in { from { opacity: 0; transform: translateY(10px) scale(0.995); } to { opacity: 1; transform: none; } }
</style>
