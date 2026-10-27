<!-- Level 4: which model for images, voice and video. All text comes from catalog.json. -->
<script lang="ts">
  import { toast } from '../lib/state/toasts.svelte';
  import { PROVIDER_LABEL } from '../lib/util/labels';
  import Icon from '../components/Icon.svelte';
  import { catalog, type Tier } from './catalog';

  let tab = $state(catalog.capabilities[0].id);
  const cap = $derived(catalog.capabilities.find((c) => c.id === tab) ?? catalog.capabilities[0]);
  const TIER: Record<Tier, string> = { best: 'Best', fast: 'Fastest', good: 'Good', none: 'Not for this' };

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      toast({ kind: 'info', title: 'Prompt copied', duration: 2500 });
    } catch {
      /* clipboard blocked */
    }
  }
</script>

<div class="tabs" role="tablist" aria-label="Capability">
  {#each catalog.capabilities as c (c.id)}
    <button role="tab" aria-selected={tab === c.id} class:on={tab === c.id} onclick={() => (tab = c.id)}><Icon name={c.icon} size={14} />{c.title}</button>
  {/each}
</div>
<p class="blurb">{cap.blurb}</p>
<ul class="picks">
  {#each cap.picks as p (p.name)}
    <li class="pick t-{p.tier}">
      <span class="tier">{TIER[p.tier]}</span>
      <div class="p-body">
        <p class="p-name"><b>{p.name}</b><span class="via">{PROVIDER_LABEL[p.provider]}</span>{#if p.verify}<span class="vf" data-tip="Named from memory: double-check the current model name before relying on it.">verify</span>{/if}</p>
        <p class="why">{p.why}{#if p.needs}<span class="needs"> Needs: {p.needs}.</span>{/if}</p>
        {#if p.tip}
          <div class="tip"><code class="selectable">{p.tip}</code><button class="btn sm ghost" onclick={() => copy(p.tip!)}><Icon name="copy" size={12} />Copy</button></div>
        {/if}
      </div>
    </li>
  {/each}
</ul>

<style>
  .tabs { display: inline-flex; gap: 2px; padding: 3px; border-radius: var(--r-full); background: var(--surface-1); width: fit-content; }
  .tabs button { display: inline-flex; align-items: center; gap: 6px; height: 28px; padding: 0 14px; border-radius: var(--r-full); font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-3); transition: background-color var(--d-2), color var(--d-2); }
  .tabs button.on { background: var(--surface-3); color: var(--ink-1); }
  .blurb { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-2); }
  .picks { display: flex; flex-direction: column; gap: 8px; }
  .pick { display: flex; gap: 12px; padding: 12px 14px; border-radius: var(--r-lg); background: var(--surface-1); box-shadow: inset 0 0 0 1px var(--line); }
  .pick.t-none { opacity: 0.7; }
  .tier { flex: none; width: 74px; height: 22px; display: grid; place-items: center; border-radius: 7px; font-family: var(--font-mono); font-size: 10px; letter-spacing: 0.05em; text-transform: uppercase; background: var(--surface-2); color: var(--ink-3); }
  .t-best .tier { background: color-mix(in srgb, var(--amber) 24%, transparent); color: var(--amber-ink); }
  .t-fast .tier { background: color-mix(in srgb, var(--aurora-m) 24%, transparent); color: var(--ink-1); }
  .t-good .tier { background: color-mix(in srgb, var(--ok) 20%, transparent); color: var(--ink-1); }
  .p-body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .p-name { display: flex; align-items: baseline; gap: 8px; flex-wrap: wrap; }
  .p-name b { font-size: 14px; font-weight: var(--w-semi); }
  .via { font-size: var(--t-xs); color: var(--ink-3); font-family: var(--font-mono); }
  .vf { font-size: 10px; font-family: var(--font-mono); padding: 1px 6px; border-radius: 6px; background: color-mix(in srgb, var(--amber) 18%, transparent); color: var(--amber-ink); }
  .why { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-2); }
  .needs { color: var(--ink-3); }
  .tip { display: flex; align-items: center; gap: 8px; margin-top: 4px; padding: 6px 6px 6px 10px; border-radius: var(--r-md); background: var(--surface-2); }
  .tip code { flex: 1; font-size: 11.5px; line-height: 1.4; color: var(--ink-1); white-space: normal; }
</style>
