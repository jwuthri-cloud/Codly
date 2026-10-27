<!--
  First run as a quest: five levels on a map, 100 XP each, all skippable, reopenable from Settings.
  The crew level completes itself the moment one CLI is ready; the others when you press Continue.
-->
<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { api, isTauriApp } from '../lib/api';
  import { redetect } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import Icon from '../components/Icon.svelte';
  import Logo from '../components/Logo.svelte';
  import { blip } from './sound';
  import { LEVELS, XP_PER_LEVEL, complete, crewReady, levelIndex, nextLevel, progress, rank, xp, type LevelId } from './quest';
  import LevelBrains from './LevelBrains.svelte';
  import LevelCreative from './LevelCreative.svelte';
  import LevelCrew from './LevelCrew.svelte';
  import LevelLead from './LevelLead.svelte';
  import LevelMission from './LevelMission.svelte';

  let current = $state<LevelId>(untrack(() => nextLevel(ui.quest)));
  const idx = $derived(levelIndex(current));
  const points = $derived(xp(ui.quest));
  const sound = $derived(ui.questSound && !ui.reducedMotion);
  const level = $derived(LEVELS[idx]);

  // Someone who already has repositories is not new here: don't walk them through it uninvited.
  onMount(() => {
    if (ui.onboarded || ui.onboardingOpen || new URLSearchParams(location.search).get('onboard')) return;
    api.recentWorkspaces().then((r) => r.length && (ui.onboarded = true)).catch(() => {});
  });

  function award(id: LevelId) {
    if (ui.quest.includes(id)) return;
    ui.quest = complete(ui.quest, id);
    const l = LEVELS[levelIndex(id)];
    toast({ kind: 'success', title: `+${XP_PER_LEVEL} XP · ${l.reward}`, body: ui.quest.length === LEVELS.length ? undefined : `Next: ${LEVELS[levelIndex(nextLevel(ui.quest))].title}` });
    blip('level', sound);
  }
  // Level 1 finishes itself: one working CLI is all a crew needs to start.
  $effect(() => {
    if (crewReady(store.clis) && !untrack(() => ui.quest.includes('crew'))) award('crew');
  });
  function next() {
    award(current);
    blip('tap', sound);
    if (idx < LEVELS.length - 1) current = LEVELS[idx + 1].id;
  }
  function back() {
    blip('tap', sound);
    if (idx > 0) current = LEVELS[idx - 1].id;
  }
  function close() {
    ui.onboarded = true;
    ui.onboardingOpen = false;
  }
  function finish() {
    award('mission');
    blip('done', sound);
    close();
  }
  function onkey(e: KeyboardEvent) {
    if (e.key === 'Escape' && ui.onboarded) close();
  }
  async function again() {
    await redetect();
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="onb" role="dialog" aria-modal="true" aria-label="Welcome to Codly">
  <div class="sky" aria-hidden="true"></div>
  <div class="drag" data-tauri-drag-region class:tauri={isTauriApp()}></div>

  <header class="top">
    <p class="brand"><Logo size={22} /><span>Codly</span></p>
    <div class="xp" role="progressbar" aria-label="Quest progress" aria-valuemin="0" aria-valuemax={LEVELS.length * XP_PER_LEVEL} aria-valuenow={points}>
      <span class="rank">{rank(points)}</span>
      <span class="bar"><i style:width="{progress(ui.quest) * 100}%"></i></span>
      <span class="pts num">{points} XP</span>
    </div>
    <div class="right">
      <button class="icon-btn" aria-pressed={ui.questSound} aria-label={ui.questSound ? 'Mute sounds' : 'Turn sounds on'} data-tip={ui.questSound ? 'Mute sounds' : 'Turn sounds on'} onclick={() => (ui.questSound = !ui.questSound)} disabled={ui.reducedMotion}>
        <Icon name={ui.questSound && !ui.reducedMotion ? 'bolt' : 'slash'} size={15} />
      </button>
      <button class="btn ghost sm" onclick={close}>{ui.onboarded ? 'Close' : 'Skip for now'}</button>
    </div>
  </header>

  <nav class="map" aria-label="Quest map">
    {#each LEVELS as l, i (l.id)}
      {@const done = ui.quest.includes(l.id)}
      <button class="node" class:done class:on={l.id === current} aria-current={l.id === current ? 'step' : undefined} onclick={() => (current = l.id)}>
        <span class="dot"><Icon name={done ? 'check' : l.icon} size={16} /></span>
        <span class="n-title">{l.title}</span>
      </button>
      {#if i < LEVELS.length - 1}<span class="path" class:done aria-hidden="true"></span>{/if}
    {/each}
  </nav>

  <main class="stage">
    {#key current}
      <section class="panel glass" aria-labelledby="lvl-title">
        <p class="eyebrow">Level {idx + 1} of {LEVELS.length}</p>
        <h1 id="lvl-title" class="serif">{level.title}</h1>
        <p class="blurb">{level.blurb}</p>
        <div class="body">
          {#if current === 'crew'}<LevelCrew {again} />
          {:else if current === 'lead'}<LevelLead />
          {:else if current === 'brains'}<LevelBrains />
          {:else if current === 'creative'}<LevelCreative />
          {:else}<LevelMission {finish} />{/if}
        </div>
      </section>
    {/key}
  </main>

  <footer class="foot">
    <button class="btn ghost" onclick={back} disabled={idx === 0}><Icon name="arrow-left" size={14} />Back</button>
    <span class="hint">{ui.quest.includes(current) ? 'Done · +' + XP_PER_LEVEL + ' XP' : 'Everything here is optional.'}</span>
    {#if current !== 'mission'}
      <button class="btn primary" onclick={next}>{ui.quest.includes(current) ? 'Next level' : 'Continue'}<Icon name="arrow-right" size={14} /></button>
    {/if}
  </footer>
</div>

<style>
  .onb { position: fixed; inset: 0; z-index: calc(var(--z-ui) + 20); display: flex; flex-direction: column; background: var(--night); overflow: hidden; }
  .sky { position: absolute; inset: 0; pointer-events: none; background: radial-gradient(120% 80% at 50% 125%, var(--night-glow), transparent 70%), radial-gradient(60% 40% at 80% 0%, color-mix(in srgb, var(--aurora-b) 14%, transparent), transparent 70%); }
  .drag { position: relative; height: 28px; flex-shrink: 0; }
  .top { position: relative; width: min(1040px, calc(100vw - 48px)); margin: 0 auto; display: flex; align-items: center; gap: 24px; padding: 6px 0 4px; }
  .brand { display: inline-flex; align-items: center; gap: 9px; font-weight: var(--w-semi); font-size: 14px; }
  .xp { flex: 1; display: flex; align-items: center; gap: 12px; min-width: 0; }
  .rank { font-family: var(--font-mono); font-size: 11px; letter-spacing: 0.08em; text-transform: uppercase; color: var(--amber); white-space: nowrap; }
  .bar { flex: 1; height: 8px; border-radius: 4px; background: var(--surface-2); overflow: hidden; box-shadow: inset 0 0 0 1px var(--line); }
  .bar i { display: block; height: 100%; border-radius: 4px; background: linear-gradient(90deg, var(--aurora-a), var(--aurora-m), var(--aurora-b)); transition: width var(--d-5) var(--spring); }
  .pts { font-family: var(--font-mono); font-size: 12px; color: var(--ink-2); white-space: nowrap; }
  .right { display: flex; align-items: center; gap: 8px; }
  .map { position: relative; width: min(1040px, calc(100vw - 48px)); margin: 18px auto 0; display: flex; align-items: flex-start; justify-content: space-between; }
  .node { display: flex; flex-direction: column; align-items: center; gap: 8px; width: 110px; color: var(--ink-3); transition: color var(--d-2); }
  .node:hover { color: var(--ink-1); }
  .dot { width: 44px; height: 44px; border-radius: 14px; display: grid; place-items: center; background: var(--surface-1); box-shadow: inset 0 0 0 1px var(--line-2); transition: transform var(--d-3) var(--spring), box-shadow var(--d-3), background-color var(--d-3); }
  .node.on .dot { transform: translateY(-3px) scale(1.06); color: var(--ink-1); box-shadow: inset 0 0 0 1.5px var(--aurora-m), 0 8px 24px color-mix(in srgb, var(--aurora-m) 35%, transparent); animation: pulse 2.4s var(--ease-out) infinite; }
  .node.done .dot { background: color-mix(in srgb, var(--ok) 18%, var(--surface-1)); color: var(--ok); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--ok) 50%, transparent); }
  .n-title { font-size: var(--t-sm); font-weight: var(--w-med); text-align: center; line-height: 1.25; }
  .node.on .n-title { color: var(--ink-1); }
  .path { flex: 1; height: 2px; margin-top: 21px; border-radius: 1px; background: repeating-linear-gradient(90deg, var(--line-3) 0 6px, transparent 6px 12px); }
  .path.done { background: var(--ok); opacity: 0.6; }
  .stage { position: relative; flex: 1; min-height: 0; overflow-y: auto; display: flex; justify-content: center; padding: 22px 0 8px; }
  .panel { width: min(760px, calc(100vw - 48px)); height: fit-content; padding: 28px 32px 30px; border-radius: var(--r-xl); display: flex; flex-direction: column; gap: 6px; animation: rise-in var(--d-4) var(--spring) both; }
  h1 { font-size: 34px; line-height: 1.08; letter-spacing: -0.01em; }
  .blurb { font-size: 15px; line-height: 23px; color: var(--ink-2); margin-bottom: 14px; }
  .body { display: flex; flex-direction: column; gap: 10px; }
  .foot { position: relative; width: min(760px, calc(100vw - 48px)); margin: 0 auto; display: flex; align-items: center; gap: 14px; padding: 12px 0 22px; }
  .hint { flex: 1; text-align: center; font-size: var(--t-sm); color: var(--ink-3); }
  @keyframes pulse { 0%, 100% { box-shadow: inset 0 0 0 1.5px var(--aurora-m), 0 8px 24px color-mix(in srgb, var(--aurora-m) 30%, transparent); } 50% { box-shadow: inset 0 0 0 1.5px var(--aurora-m), 0 10px 32px color-mix(in srgb, var(--aurora-m) 55%, transparent); } }
  @media (prefers-reduced-motion: reduce) { .node.on .dot { animation: none; } }
  @media (max-width: 760px) { .n-title { display: none; } .node { width: 44px; } }
</style>
