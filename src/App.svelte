<script lang="ts">
  import { onMount, untrack } from 'svelte';
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
  import PullRequestDialog from './overlays/PullRequestDialog.svelte';
  import IntakeDialog from './overlays/IntakeDialog.svelte';
  import { prUi } from './lib/state/pullRequest.svelte';
  import TrustDialog from './overlays/TrustDialog.svelte';
  import { workspaceTrust } from './lib/state/workspaceTrust.svelte';
  import { intakeUi } from './lib/state/intake.svelte';
  import IdeOverlay from './ide/IdeOverlay.svelte';
  import Logo from './components/Logo.svelte';
  import StartFailed from './components/StartFailed.svelte';
  import Toasts from './components/Toasts.svelte';
  import TooltipHost from './components/TooltipHost.svelte';
  import DragGhost from './components/DragGhost.svelte';
  import { store, installVisibility } from './lib/state/store.svelte';
  import { ui } from './lib/state/ui.svelte';
  import { installKeys } from './lib/state/keys';
  import { installNotifications } from './lib/state/notify';
  import { installNoticeOpener } from './lib/state/openNotice';
  import { installTooltips } from './lib/state/tooltip.svelte';

  // Dev-only handle for debugging and browser tests (stripped from production builds).
  if (import.meta.env.DEV) (window as unknown as { __codly: unknown }).__codly = { store, ui };

  onMount(() => {
    store.connect();
    workspaceTrust.start();
    const off = [installKeys(), installTooltips(), installVisibility(), installNotifications(), installNoticeOpener()];
    return () => off.forEach((f) => f());
  });

  // A repo that opens is asked about once: its own CLI configuration loads only when you trust it.
  const openRepo = $derived(store.workspace?.id ?? null);
  $effect(() => {
    void openRepo;
    untrack(() => void workspaceTrust.check());
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
  <!-- The core reads the login shell and asks each CLI who is signed in: seconds, with a slow shell
       start-up file. Shown after a moment, so a fast start does not flash it. -->
  <div class="boot center" data-tauri-drag-region>
    <div class="starting" role="status">
      <Logo size={40} />
      <p class="serif">Starting Codly…</p>
      <p class="what">Reading your shell setup and checking your agent CLIs.</p>
      <p class="what slow">Still working. A slow shell start-up file (like <code>.zshrc</code>) can make this take a few more seconds.</p>
    </div>
  </div>
{:else if store.workspace}
  <Shell />
{:else if store.error}
  <!-- The first snapshot never arrived: say why, instead of an empty Welcome. -->
  <div class="boot center" data-tauri-drag-region>
    <StartFailed />
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
{#if prUi.dialog}<PullRequestDialog />{/if}
{#if workspaceTrust.asking}<TrustDialog />{/if}
{#if intakeUi.dialog}<IntakeDialog />{/if}
<IdeOverlay />
<Toasts />
<TooltipHost />
<DragGhost />

<style>
  .boot { position: fixed; inset: 0; background: var(--night); }
  .boot.center { display: grid; place-items: center; }
  .starting { display: flex; flex-direction: column; align-items: center; gap: 8px; padding: 24px; text-align: center; animation: fade-in var(--d-4) var(--ease-out) 600ms both; }
  .starting .serif { margin-top: 6px; font-size: var(--d-sm); color: var(--ink-1); }
  .what { font-size: var(--t-md); line-height: var(--lh-md); color: var(--ink-3); max-width: 46ch; }
  .what.slow { animation: fade-in var(--d-4) var(--ease-out) 8s both; }
  .what code { font-size: 0.9em; color: var(--ink-2); }
</style>
