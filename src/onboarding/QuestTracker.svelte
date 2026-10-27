<!--
  In the workspace: a compact quest tracker (level, XP bar, next quest) that collapses to a
  badge and goes away once dismissed or once every quest is done.
-->
<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import { quest } from './quest.svelte';

  let open = $state(true);
</script>

<aside class="tracker glass" class:open aria-label="Onboarding quests">
  <div class="head">
    <button class="toggle" onclick={() => (open = !open)} aria-expanded={open} aria-controls="quest-next">
      <span class="lv num" aria-hidden="true">{quest.level.index + 1}</span>
      <span class="name">{quest.level.name}</span>
      <span class="xp num">{quest.xp} XP</span>
      <Icon name={open ? 'chevron-down' : 'chevron-up'} size={13} />
    </button>
    <button class="icon-btn sm" aria-label="Dismiss quests" data-tip="Dismiss quests" onclick={() => quest.dismiss()}><Icon name="x" size={13} /></button>
  </div>
  <div class="bar" role="progressbar" aria-label="Experience" aria-valuemin={quest.level.floor} aria-valuemax={quest.level.next ?? quest.xp} aria-valuenow={quest.xp}>
    <span style="width:{Math.round(quest.level.progress * 100)}%"></span>
  </div>
  {#if open && quest.next}
    <div class="next" id="quest-next">
      <p class="eyebrow">Next quest · +{quest.next.xp} XP</p>
      <p class="title">{quest.next.title}</p>
      <p class="flavor">{quest.next.flavor}</p>
    </div>
  {/if}
</aside>

<style>
  .tracker { position: fixed; left: 88px; bottom: 16px; z-index: var(--z-pop); width: 240px; padding: 8px 8px 10px; border-radius: var(--r-lg); display: flex; flex-direction: column; gap: 8px; animation: rise-in var(--d-4) var(--spring) both; }
  .head { display: flex; align-items: center; gap: 4px; }
  .toggle { flex: 1; display: flex; align-items: center; gap: 8px; padding: 2px 6px 2px 2px; border-radius: var(--r-full); font-size: var(--t-sm); }
  .toggle:hover { background: var(--surface-2); }
  .lv { display: grid; place-items: center; width: 20px; height: 20px; border-radius: 50%; background: var(--aurora); color: var(--on-aurora); font-size: var(--t-xs); font-weight: var(--w-semi); }
  .name { font-weight: var(--w-semi); }
  .xp { margin-left: auto; color: var(--live); font-weight: var(--w-semi); }
  .bar { height: 4px; margin: 0 4px; border-radius: 2px; background: var(--surface-2); overflow: hidden; }
  .bar span { display: block; height: 100%; background: var(--aurora); transition: width var(--d-5) var(--spring); }
  .next { display: flex; flex-direction: column; gap: 3px; padding: 2px 6px 0; }
  .title { font-weight: var(--w-semi); }
  .flavor { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
</style>
