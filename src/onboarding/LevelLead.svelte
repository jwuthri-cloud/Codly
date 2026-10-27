<!-- Level 2: who you talk to. Any ready CLI can lead; the choice is the same Settings → The Lead writes. -->
<script lang="ts">
  import { updateSettings } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { cliOk, drone } from '../lib/util/cli';
  import { PROVIDER_LABEL } from '../lib/util/labels';
  import Portrait from '../components/Portrait.svelte';

  const BLURB: Record<string, string> = {
    claude: 'Careful planner, great at reading a codebase. The usual pick.',
    codex: 'Fast and decisive, and it can generate images.',
    gemini: 'Huge context window, good with big repositories.',
    opencode: 'Any model you like, through OpenRouter.',
    cursor: 'Cursor’s agent, with its own models.',
    copilot: 'GitHub Copilot’s agent.',
  };
  const ready = $derived(store.clis.filter(cliOk));
  const chosen = $derived(store.settings.lead_provider ?? ready[0]?.provider ?? null);
</script>

{#if !ready.length}
  <p class="empty">No agent is ready yet. Go back one level and install or sign in to at least one, or carry on and come back later.</p>
{:else}
  <div class="grid" role="radiogroup" aria-label="Lead">
    {#each ready as c, i (c.provider)}
      <button class="card" class:on={chosen === c.provider} role="radio" aria-checked={chosen === c.provider} onclick={() => updateSettings({ lead_provider: c.provider })}>
        <Portrait agent={drone(c.provider)} size={44} />
        <b>{PROVIDER_LABEL[c.provider]}{#if i === 0 && c.provider === 'claude'}<span class="tag">Recommended</span>{/if}</b>
        <span>{BLURB[c.provider] ?? ''}</span>
      </button>
    {/each}
  </div>
  <p class="fine">You can change the Lead, its model and its thinking effort any time in Settings.</p>
{/if}

<style>
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(210px, 1fr)); gap: 10px; }
  .card { display: flex; flex-direction: column; align-items: flex-start; gap: 6px; padding: 14px; text-align: left; border-radius: var(--r-lg); background: var(--surface-1); box-shadow: inset 0 0 0 1px var(--line); transition: box-shadow var(--d-3), transform var(--d-3) var(--spring); }
  .card:hover { transform: translateY(-2px); }
  .card.on { box-shadow: inset 0 0 0 1.5px var(--aurora-m), 0 6px 20px color-mix(in srgb, var(--aurora-m) 22%, transparent); }
  .card b { font-size: 14.5px; font-weight: var(--w-semi); display: flex; align-items: center; gap: 8px; }
  .card span:not(.tag) { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .tag { font-size: 10px; font-family: var(--font-mono); text-transform: uppercase; letter-spacing: 0.06em; padding: 2px 6px; border-radius: 6px; background: color-mix(in srgb, var(--amber) 22%, transparent); color: var(--amber-ink); }
  .fine, .empty { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
</style>
