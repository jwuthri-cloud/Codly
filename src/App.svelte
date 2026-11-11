<script lang="ts">
  import { onMount } from 'svelte';
  import Shell from './shell/Shell.svelte';
  import Onboarding from './onboarding/Onboarding.svelte';
  import Welcome from './overlays/Welcome.svelte';
  import Palette from './overlays/Palette.svelte';
  import SettingsSheet from './overlays/SettingsSheet.svelte';
  import ShortcutsSheet from './overlays/ShortcutsSheet.svelte';
  import AddAgentWizard from './overlays/AddAgentWizard.svelte';
  import ModelDialog from './overlays/ModelDialog.svelte';
  import RepoPicker from './overlays/RepoPicker.svelte';
  import SkillsManager from './overlays/SkillsManager.svelte';
  import { skillsUi } from './lib/state/skills.svelte';
  import IdeOverlay from './ide/IdeOverlay.svelte';
  import EmptyState from './components/EmptyState.svelte';
  import Toasts from './components/Toasts.svelte';
  import TooltipHost from './components/TooltipHost.svelte';
  import DragGhost from './components/DragGhost.svelte';
  import { store, installVisibility } from './lib/state/store.svelte';
  import { ui } from './lib/state/ui.svelte';
  import { installKeys } from './lib/state/keys';
  import { installNotifications } from './lib/state/notify';
  import { installTooltips } from './lib/state/tooltip.svelte';

  // Dev-only handle for debugging and browser tests (stripped from production builds).
  if (import.meta.env.DEV) (window as unknown as { __codly: unknown }).__codly = { store, ui };

  onMount(() => {
    store.connect();
    const off = [installKeys(), installTooltips(), installVisibility(), installNotifications()];
    return () => off.forEach((f) => f());
  });

  // Theme: always resolved onto [data-theme] ("system" follows the OS live).
  const theme = $derived<'dark' | 'light'>(store.settings.theme === 'dark' || store.settings.theme === 'light' ? store.settings.theme : ui.systemDark ? 'dark' : 'light');
  $effect(() => {
    document.documentElement.setAttribute('data-theme', theme);
    try {
      localStorage.setItem('codly.theme', store.settings.theme);
    } catch {
      /* ignore */
    }
  });

  // Persist small UI preferences.
  $effect(() => {
    void [ui.room, ui.mode, ui.focus, ui.railOpen, ui.onboarded, ui.quest, ui.questSound, ui.sections, ui.planLens, ui.changesLens, ui.knowledgeLens, ui.timelineZoom, ui.pinned, ui.keptPlans];
    ui.persist();
  });
</script>

{#if !store.ready}
  <div class="boot" data-tauri-drag-region></div>
{:else if store.workspace}
  <Shell />
{:else if store.error}
  <!-- The first snapshot never arrived: say why, instead of an empty Welcome. -->
  <div class="boot fail" data-tauri-drag-region>
    <EmptyState title="Codly couldn’t start." body={store.error}>
      <button class="btn primary" onclick={() => store.connect()}>Try again</button>
    </EmptyState>
  </div>
{:else if ui.onboarded}
  <Welcome />
{:else}
  <Onboarding />
{/if}
{#if ui.onboardingOpen && (store.workspace || ui.onboarded)}<Onboarding />{/if}

{#if ui.paletteOpen}<Palette />{/if}
{#if ui.settingsOpen}<SettingsSheet />{/if}
{#if ui.shortcutsOpen}<ShortcutsSheet />{/if}
{#if ui.modelDialog}{#if ui.modelDialog.agentId}<ModelDialog />{:else}<AddAgentWizard />{/if}{/if}
{#if ui.repoPickerOpen}<RepoPicker />{/if}
{#if skillsUi.open}<SkillsManager />{/if}
<IdeOverlay />
<Toasts />
<TooltipHost />
<DragGhost />

<style>
  .boot { position: fixed; inset: 0; background: var(--night); }
  .boot.fail { display: grid; place-items: center; }
</style>
