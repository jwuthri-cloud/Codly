<!--
  How much you trust this agent: what it may do without asking, and whether its work skips the
  review. A safety setting you choose here at any time, whatever its rank; the core enforces it
  (the approval broker and the gate) and records each change as a decision
  (orchestrator/careers.rs `set_trust`). Closed, the section's line still names the level.
-->
<script lang="ts">
  import type { Agent } from '../lib/gen/Agent';
  import type { Rank } from '../lib/gen/Rank';
  import { setTrust } from '../lib/state/actions';
  import { TRUST, trustOf } from '../lib/util/career';
  import Section from '../components/Section.svelte';

  interface Props {
    agent: Agent;
  }
  let { agent }: Props = $props();

  const level = $derived(agent.career.trust);
  let saving = $state(false);

  async function pick(next: Rank) {
    if (saving || next === level) return;
    saving = true;
    await setTrust(agent.id, next);
    saving = false;
  }
</script>

<Section id="trust" title="Trust" summary={trustOf(level).label}>
  <div class="levels" role="radiogroup" aria-label="How much you trust {agent.name}">
    {#each TRUST as t (t.level)}
      <button class="level" class:on={t.level === level} role="radio" aria-checked={t.level === level} disabled={saving} onclick={() => pick(t.level)}>
        <span class="dot" aria-hidden="true"></span>
        <span class="txt"><b>{t.label}</b><span class="sub">{t.allows}</span></span>
      </button>
    {/each}
  </div>
  <p class="sub note">Your choice for {agent.name} in this repo. Its rank and XP never change it, and every change is in its History.</p>
</Section>

<style>
  .levels { display: flex; flex-direction: column; gap: 2px; margin: 0 -8px; }
  .level { display: flex; align-items: flex-start; gap: 10px; padding: 7px 8px; border-radius: var(--r-md); text-align: left; }
  .level:hover { background: var(--surface-1); }
  .level:focus-visible { outline: 2px solid var(--live); outline-offset: 1px; }
  .level.on { background: var(--surface-1); }
  .dot { flex: none; width: 14px; height: 14px; margin-top: 2px; border-radius: 50%; box-shadow: inset 0 0 0 1.5px var(--line-3); }
  .level.on .dot { box-shadow: inset 0 0 0 4px var(--live); }
  .txt { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .txt b { font-weight: var(--w-med); font-size: var(--t-md); color: var(--ink-1); }
  .sub { font-size: var(--t-xs); line-height: 16px; color: var(--ink-3); }
  .note { margin-top: 8px; }
</style>
