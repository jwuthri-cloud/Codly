<!--
  The Sims' click menu on a piece of furniture: what it can be used for, and which free agent to
  send. Only agents between tasks can go; real work calls them back. Cosmetic: using furniture
  never changes how agents work.
-->
<script lang="ts">
  import type { Agent } from '../lib/gen/Agent';
  import type { FurnitureMenu, OfficeEngine } from './engine';
  import { agentPortrait } from './portrait';

  interface Props {
    engine: OfficeEngine;
    menu: FurnitureMenu;
    agents: Agent[];
    onclose: () => void;
  }
  let { engine, menu, agents, onclose }: Props = $props();

  const free = $derived.by(() => {
    const ids = new Set(engine.freeAgents());
    return agents.filter((a) => ids.has(a.id));
  });

  function send(a: Agent, intent: FurnitureMenu['actions'][number]['intent']) {
    engine.sendTo(a.id, intent);
    onclose();
  }
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && onclose()} />

<div class="menu" style:left="{menu.x}px" style:top="{menu.y}px" role="dialog" aria-label={menu.label}>
  <header>
    <b>{menu.label}</b>
    <button class="x" aria-label="Close" onclick={onclose}>✕</button>
  </header>
  {#if !menu.actions.length}
    <p class="note">Just for looks.</p>
  {:else if !free.length}
    <p class="note">Everyone is busy. Agents take breaks between tasks.</p>
  {:else}
    {#each menu.actions as act (act.label)}
      <div class="act">
        <span class="lbl">{act.label}</span>
        <span class="who">
          {#each free as a (a.id)}
            <button onclick={() => send(a, act.intent)} title="Send {a.name}">
              <img src={agentPortrait(a, 28)} alt="" width="22" height="22" />{a.name}
            </button>
          {/each}
        </span>
      </div>
    {/each}
  {/if}
</div>

<style>
  .menu { position: absolute; z-index: 402; transform: translate(-50%, calc(-100% - 12px)); min-width: 200px; max-width: 340px; padding: 10px 12px; border-radius: 14px; background: rgba(22, 26, 36, 0.95); color: #eef1f7; box-shadow: 0 18px 50px -20px rgba(0, 0, 0, 0.8), inset 0 0 0 1px rgba(255, 255, 255, 0.08); font: 500 12.5px/1.35 'Inter Variable', Inter, system-ui, sans-serif; }
  header { display: flex; justify-content: space-between; align-items: center; gap: 10px; margin-bottom: 6px; }
  .x { border: 0; background: none; color: #9aa2b6; font-size: 13px; cursor: pointer; }
  .act { display: flex; flex-direction: column; gap: 4px; padding: 6px 0; border-top: 1px solid rgba(255, 255, 255, 0.07); }
  .lbl { font-weight: 600; color: #8ff0b0; }
  .who { display: flex; flex-wrap: wrap; gap: 4px; }
  .who button { display: inline-flex; align-items: center; gap: 5px; padding: 3px 9px 3px 3px; border: 0; border-radius: 999px; background: rgba(255, 255, 255, 0.08); color: inherit; font: inherit; cursor: pointer; }
  .who button:hover { background: rgba(126, 224, 161, 0.2); }
  img { border-radius: 50%; }
  .note { margin: 2px 0 0; font-size: 12px; opacity: 0.7; }
</style>
