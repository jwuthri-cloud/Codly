<!--
  The slim icon rail (labelled when expanded, via the chevron at the bottom): your office tabs, Home and four rooms, then the view controls (full office ⌘1, focus ⌘.) and
  Settings. Badges say only what needs saying: amber when something waits for you, an orbit on
  Plan while it runs, a dot on Changes when new artifacts arrive.
-->
<script lang="ts">
  import { isTauriApp } from '../lib/api';
  import { store } from '../lib/state/store.svelte';
  import { ROOMS, ui, type Room } from '../lib/state/ui.svelte';
  import Icon from '../components/Icon.svelte';
  import Logo from '../components/Logo.svelte';
  import OfficeTabs from './OfficeTabs.svelte';

  const tauri = isTauriApp();
  const waiting = $derived(store.pendingApprovals.length);
  // Every running plan, across requests, counts towards the Plan badge.
  const running = $derived(store.plans.filter((p) => p.status === 'running' || p.status === 'integrating'));
  const planTasks = $derived(store.tasks.filter((t) => running.some((p) => p.id === t.plan_id)));
  const planFrac = $derived(planTasks.length ? planTasks.filter((t) => t.status === 'done').length / planTasks.length : 0);
  const planLive = $derived(running.length > 0);

  // New artifacts since you last opened Changes.
  let seenArtifacts = $state(0);
  $effect(() => {
    if (ui.room === 'changes') seenArtifacts = store.artifacts.length;
  });
  const freshArtifacts = $derived(Math.max(0, store.artifacts.length - seenArtifacts));

  function badge(r: Room): 'wait' | 'plan' | 'dot' | null {
    if (r === 'home' && waiting && (ui.room !== 'home' || ui.mode === 'office' || ui.mode === 'map')) return 'wait';
    if (r === 'plan' && planLive) return 'plan';
    if (r === 'changes' && freshArtifacts && ui.room !== 'changes') return 'dot';
    return null;
  }
  const C = 2 * Math.PI * 7;
</script>

<nav class="rail glass" class:tauri class:open={ui.railOpen} aria-label="Rooms">
  <div class="mark" data-tauri-drag-region><Logo size={24} /></div>
  {#if store.workspace}<OfficeTabs />{/if}
  <ul class="rooms">
    {#each ROOMS as r (r.id)}
      {@const b = badge(r.id)}
      <li>
        <button
          class="room"
          class:on={ui.room === r.id && !(r.id === 'home' && (ui.mode === 'office' || ui.mode === 'map'))}
          aria-current={ui.room === r.id ? 'page' : undefined}
          aria-label={r.label}
          data-tip="{r.label} — {r.hint}"
          data-tip-kbd="G {r.key}"
          onclick={() => ui.go(r.id)}
        >
          <Icon name={r.icon} size={18} />
          {#if ui.railOpen}<span class="label">{r.label}</span>{/if}
          {#if b === 'wait'}<span class="b-wait" aria-label="{waiting} waiting for you">{waiting}</span>
          {:else if b === 'plan'}
            <svg class="b-plan" width="18" height="18" viewBox="0 0 18 18" aria-hidden="true">
              <circle cx="9" cy="9" r="7" fill="none" style="stroke: var(--line-2)" stroke-width="2" />
              <circle cx="9" cy="9" r="7" fill="none" style="stroke: var(--live)" stroke-width="2" stroke-linecap="round" stroke-dasharray="{Math.max(1.5, C * planFrac)} {C}" transform="rotate(-90 9 9)" />
            </svg>
          {:else if b === 'dot'}<span class="b-dot" aria-label="{freshArtifacts} new artifacts"></span>{/if}
        </button>
      </li>
    {/each}
  </ul>
  <div class="grow" data-tauri-drag-region></div>
  <ul class="views">
    <li>
      <button class="room" aria-pressed={ui.mode === 'office'} class:on={ui.mode === 'office'} aria-label="Full office" data-tip={ui.mode === 'office' ? 'Back to work' : 'Full office: hide the panels'} data-tip-kbd="⌘1" onclick={() => ui.toggleMode('office')}>
        <Icon name="office" size={18} />
        {#if ui.railOpen}<span class="label">Full office</span>{/if}
      </button>
    </li>
    <li>
      <button class="room" aria-pressed={ui.mode === 'map'} class:on={ui.mode === 'map'} aria-label="Neighbourhood" data-tip={ui.mode === 'map' ? 'Back to work' : 'Neighbourhood: all your lots'} data-tip-kbd="⌘3" onclick={() => ui.toggleMode('map')}>
        <Icon name="map" size={18} />
        {#if ui.railOpen}<span class="label">Neighbourhood</span>{/if}
      </button>
    </li>
    <li>
      <button class="room" aria-pressed={ui.focus} class:on={ui.focus} aria-label="Focus" data-tip={ui.focus ? 'Bring the office back' : 'Focus: dim the office'} data-tip-kbd="⌘." onclick={() => (ui.focus = !ui.focus)}>
        <Icon name="focus" size={18} />
        {#if ui.railOpen}<span class="label">Focus</span>{/if}
      </button>
    </li>
    <li>
      <button class="room" aria-label="Settings" data-tip="Settings" data-tip-kbd="⌘," onclick={() => (ui.settingsOpen = true)}>
        <Icon name="settings" size={18} />
        {#if ui.railOpen}<span class="label">Settings</span>{/if}
      </button>
    </li>
    <li>
      <button class="room" aria-label={ui.railOpen ? 'Collapse sidebar' : 'Expand sidebar'} aria-expanded={ui.railOpen} data-tip={ui.railOpen ? 'Collapse the sidebar' : 'Show labels'} onclick={() => (ui.railOpen = !ui.railOpen)}>
        <Icon name={ui.railOpen ? 'chevron-left' : 'chevron-right'} size={18} />
        {#if ui.railOpen}<span class="label">Collapse</span>{/if}
      </button>
    </li>
  </ul>
</nav>

<style>
  .rail { position: fixed; z-index: var(--z-ui); left: var(--gutter); top: var(--gutter); bottom: var(--gutter); width: var(--rail-w); display: flex; flex-direction: column; align-items: center; padding: 14px 0 10px; border-radius: var(--r-xl); }
  .rail.tauri { top: 48px; }
  .rail { transition: width var(--d-4) var(--spring); }
  .rail.open { align-items: stretch; padding-left: 7px; padding-right: 7px; }
  .rail.open .mark { width: auto; justify-content: start; padding-left: 7px; }
  .rail.open .room { width: 100%; grid-auto-flow: column; justify-content: start; justify-items: start; gap: 11px; padding: 0 10px; }
  .rail.open .room.on::before { left: -7px; }
  .label { font-size: 13px; font-weight: 500; white-space: nowrap; }
  .rail.open .b-wait { top: 50%; right: 8px; transform: translateY(-50%); }
  .rail.open .b-plan { top: 50%; right: 8px; margin-top: -7px; }
  .rail.open .b-dot { top: 50%; right: 11px; margin-top: -3px; }
  .mark { width: 36px; height: 36px; display: grid; place-items: center; margin-bottom: 14px; }
  .rooms, .views { display: flex; flex-direction: column; gap: 4px; }
  .grow { flex: 1; width: 100%; }
  .room { position: relative; display: grid; place-items: center; width: 38px; height: 38px; border-radius: 12px; color: var(--ink-3); transition: background-color var(--d-2) var(--ease-out), color var(--d-2) var(--ease-out); }
  .room:hover { color: var(--ink-1); background: var(--surface-2); }
  .room.on { color: var(--ink-1); background: var(--surface-3); }
  .room.on::before { content: ''; position: absolute; left: -9px; top: 11px; bottom: 11px; width: 3px; border-radius: 3px; background: var(--ink-1); }
  .b-wait { position: absolute; top: 3px; right: 2px; min-width: 15px; height: 15px; padding: 0 4px; border-radius: 8px; background: var(--amber); color: #241504; font-size: 10px; font-weight: 700; line-height: 15px; text-align: center; }
  .b-plan { position: absolute; top: 1px; right: 0; width: 14px; height: 14px; }
  .b-plan circle:last-child { transition: stroke-dasharray var(--d-5) var(--spring); }
  .b-dot { position: absolute; top: 7px; right: 7px; width: 7px; height: 7px; border-radius: 50%; background: var(--aurora-m); box-shadow: 0 0 0 2px var(--glass-solid); }
</style>
