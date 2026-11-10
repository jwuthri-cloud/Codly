<!--
  Ghosts: agents let go (consultants after their task) keep their career and float by the
  memory archive. The tag over the archive opens them: their record and scrapbook, and Bring
  back, which makes one a regular worker again (same name, look, career and memory).
-->
<script lang="ts">
  import type { Agent } from '../lib/gen/Agent';
  import { rankLabel } from '../lib/util/career';
  import { modal } from '../lib/util/modal';
  import type { OfficeEngine } from './engine';
  import { agentPortrait } from './portrait';
  import Scrapbook from './Scrapbook.svelte';

  interface Props {
    ghosts: Agent[];
    engine: OfficeEngine;
    insets: { top: number; right: number; bottom: number; left: number };
    onbringback?: (agentId: string) => void;
  }
  let { ghosts, engine, insets, onbringback }: Props = $props();

  let at = $state<{ x: number; y: number } | null>(null);
  let open = $state(false);
  let picked = $state<string | null>(null);
  $effect(() => {
    if (!ghosts.length) return;
    const t = setInterval(() => (at = engine.archiveScreen()), 250);
    at = engine.archiveScreen();
    return () => clearInterval(t);
  });
  const ghost = $derived(ghosts.find((g) => g.id === picked) ?? ghosts[0] ?? null);
</script>

{#if ghosts.length && at}
  <button class="tag" style:left="{at.x}px" style:top="{Math.max(insets.top + 8, at.y)}px" onclick={() => (open = true)} aria-label="{ghosts.length} ghosts haunt the memory archive">
    👻 {ghosts.length} {ghosts.length === 1 ? 'ghost' : 'ghosts'}
  </button>
{/if}

{#if open && ghost}
  <div class="scrim" role="presentation" onclick={() => (open = false)}></div>
  <div class="dlg" role="dialog" aria-modal="true" aria-label="Ghosts of the memory archive" use:modal={() => (open = false)}>
    <header>
      <div>
        <div class="kicker">Memory archive · let go, never forgotten</div>
        <h3>Ghosts</h3>
      </div>
      <button class="x" aria-label="Close" onclick={() => (open = false)}>✕</button>
    </header>
    <div class="body">
      <ul class="list">
        {#each ghosts as g (g.id)}
          <li>
            <button class:on={g.id === ghost.id} onclick={() => (picked = g.id)}>
              <img src={agentPortrait({ id: g.id, color: g.color, look_seed: g.look_seed, provider: g.provider, role: g.role }, 40)} alt="" width="40" height="40" />
              <span><b>{g.name}</b><small>{rankLabel(g.career.rank)} · {g.career.xp} XP</small></span>
            </button>
          </li>
        {/each}
      </ul>
      <section>
        <h4>{ghost.name} <small>{ghost.title ?? 'Worker'} · {ghost.provider}</small></h4>
        <p class="stats"><b>{ghost.career.stats.merged}</b> merged · <b>{ghost.career.stats.checks_passed}</b> checks passed · <b>{ghost.career.stats.approved}</b> approved</p>
        <Scrapbook milestones={ghost.career.milestones} empty="No milestones yet." />
        {#if onbringback}
          <button class="back" onclick={() => { onbringback(ghost.id); open = false; picked = null; }}>Bring {ghost.name} back</button>
          <p class="note">Back as a regular worker: the Lead's next tasks on {ghost.provider} can go to it, and you can assign it work.</p>
        {/if}
      </section>
    </div>
  </div>
{/if}

<style>
  .tag { position: absolute; z-index: 400; transform: translateX(-50%); padding: 5px 12px; border: 0; border-radius: 999px; background: rgba(150, 190, 255, 0.2); box-shadow: inset 0 0 0 1px rgba(170, 205, 255, 0.45), 0 0 18px rgba(140, 180, 255, 0.35); color: #e4eeff; font: 600 12px/1.2 'Inter Variable', Inter, system-ui, sans-serif; cursor: pointer; backdrop-filter: blur(4px); }
  .tag:hover { background: rgba(150, 190, 255, 0.32); }
  .scrim { position: absolute; inset: 0; z-index: 900; background: rgba(8, 10, 16, 0.45); }
  .dlg { position: absolute; z-index: 901; left: 50%; top: 50%; transform: translate(-50%, -50%); width: min(600px, calc(100% - 32px)); padding: 16px; border-radius: 18px; background: rgba(22, 26, 36, 0.96); color: #eef1f7; box-shadow: 0 30px 80px -30px rgba(0, 0, 0, 0.8), inset 0 0 0 1px rgba(170, 205, 255, 0.18); font: 500 13px 'Inter Variable', Inter, system-ui, sans-serif; }
  header { display: flex; justify-content: space-between; align-items: start; margin-bottom: 12px; }
  .kicker { font: 700 10px/1 ui-monospace, monospace; letter-spacing: 0.08em; text-transform: uppercase; opacity: 0.6; }
  h3 { margin: 4px 0 0; font-size: 18px; }
  .x { border: 0; background: none; color: #9aa2b6; font-size: 16px; cursor: pointer; }
  .body { display: grid; grid-template-columns: minmax(150px, 190px) 1fr; gap: 14px; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .list button { width: 100%; display: flex; gap: 8px; align-items: center; padding: 6px; border: 0; border-radius: 10px; background: none; color: inherit; font: inherit; text-align: left; cursor: pointer; }
  .list button.on, .list button:hover { background: rgba(255, 255, 255, 0.06); }
  .list img { border-radius: 10px; opacity: 0.75; filter: saturate(0.6) hue-rotate(10deg); }
  .list span { display: flex; flex-direction: column; }
  small { font-size: 11px; opacity: 0.6; font-weight: 500; }
  h4 { margin: 0 0 4px; font-size: 15px; }
  .stats { margin: 0 0 8px; font-size: 12px; opacity: 0.8; }
  .back { margin-top: 10px; padding: 7px 14px; border: 0; border-radius: 999px; background: #9cc3ff; color: #10203a; font: 600 12.5px 'Inter Variable', Inter, system-ui, sans-serif; cursor: pointer; }
  .note { margin: 6px 0 0; font-size: 11.5px; opacity: 0.6; }
  @media (max-width: 520px) { .body { grid-template-columns: 1fr; } }
</style>
