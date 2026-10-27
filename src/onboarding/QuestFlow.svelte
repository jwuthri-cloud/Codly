<!--
  First run (no workspace, onboarding neither finished nor skipped): a quest line that recruits
  your CLI crew, picks a difficulty (approval mode), sets up macOS folder access and opens the
  first repository. The last two quests (first order, first ship) carry on in the shell tracker.
-->
<script lang="ts">
  import type { ApprovalMode } from '../lib/gen/ApprovalMode';
  import type { Provider } from '../lib/gen/Provider';
  import { isTauriApp } from '../lib/api';
  import { DIFFICULTIES, FOLDER_ACCESS_URL, QUESTS, cliReady, type QuestId } from '../lib/onboarding';
  import { openWorkspace, redetect, updateSettings } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { PROVIDER_LABEL } from '../lib/util/labels';
  import Icon from '../components/Icon.svelte';
  import Logo from '../components/Logo.svelte';
  import Portrait from '../components/Portrait.svelte';
  import StateMark from '../components/StateMark.svelte';
  import FullAccessConfirm from './FullAccessConfirm.svelte';
  import { quest } from './quest.svelte';

  const STEPS: QuestId[] = ['crew', 'rules', 'vault', 'base'];
  let step = $state(Math.max(0, STEPS.findIndex((id) => !quest.isDone(id))));
  const current = $derived(QUESTS.find((q) => q.id === STEPS[step])!);

  const HUE: Record<Provider, number> = { claude: 1, codex: 4, gemini: 5, opencode: 6, cursor: 7, copilot: 3 };
  const drone = (p: Provider) => ({ id: `cli-${p}`, name: PROVIDER_LABEL[p], color: HUE[p], look_seed: p, provider: p, role: 'worker' as const });
  const crew = $derived([...store.clis].sort((a, b) => +cliReady(b) - +cliReady(a)));

  let detecting = $state(false);
  async function again() {
    detecting = true;
    await redetect();
    detecting = false;
  }

  let confirmFull = $state(false);
  function pick(m: ApprovalMode) {
    if (m === 'full' && store.settings.approval_mode !== 'full') {
      confirmFull = true;
      return;
    }
    apply(m);
  }
  function apply(m: ApprovalMode) {
    confirmFull = false;
    updateSettings({ approval_mode: m });
    quest.flag('mode');
  }

  const mac = typeof navigator !== 'undefined' && /Mac/i.test(navigator.userAgent);
  async function openFolderAccess() {
    if (isTauriApp()) {
      try {
        const { openUrl } = await import('@tauri-apps/plugin-opener');
        await openUrl(FOLDER_ACCESS_URL);
      } catch {
        /* the button still counts: the user can open the pane by hand */
      }
    }
    quest.flag('vault');
  }

  let opening = $state(false);
  async function open() {
    opening = true;
    await openWorkspace();
    opening = false;
  }
</script>

<div class="flow">
  <div class="sky" aria-hidden="true"></div>
  <div class="drag" data-tauri-drag-region class:tauri={isTauriApp()}></div>
  <header class="hud">
    <p class="brand"><Logo size={22} /><span>Codly</span></p>
    <div class="stats">
      <span class="badge" aria-label="Level {quest.level.index + 1}, {quest.level.name}"><span class="lv num">{quest.level.index + 1}</span>{quest.level.name}</span>
      <div class="bar" role="progressbar" aria-label="Experience" aria-valuemin={quest.level.floor} aria-valuemax={quest.level.next ?? quest.xp} aria-valuenow={quest.xp}>
        <span style="width:{Math.round(quest.level.progress * 100)}%"></span>
      </div>
      <span class="xp num">{quest.xp} XP</span>
    </div>
    <button class="btn ghost sm" onclick={() => quest.dismiss()}>Skip onboarding</button>
  </header>

  <div class="page">
    <nav class="line" aria-label="Quests">
      <p class="eyebrow">Quest line</p>
      <ol>
        {#each QUESTS as q, i (q.id)}
          {@const idx = STEPS.indexOf(q.id)}
          <li>
            <button class="q" class:on={idx === step} class:done={quest.isDone(q.id)} disabled={idx < 0} onclick={() => (step = idx)} aria-current={idx === step ? 'step' : undefined}>
              <span class="dot">{#if quest.isDone(q.id)}<Icon name="check" size={12} />{:else}{i + 1}{/if}</span>
              <span class="q-title">{q.title}</span>
              <span class="q-xp num">{idx < 0 ? 'in your base' : `+${q.xp}`}</span>
            </button>
          </li>
        {/each}
      </ol>
    </nav>

    {#key step}
      <section class="card glass" aria-labelledby="quest-title">
        <p class="eyebrow">Quest {step + 1} of {QUESTS.length} · +{current.xp} XP</p>
        <h1 class="serif" id="quest-title">{current.title}{#if quest.isDone(current.id)} <span class="tick"><Icon name="check-circle" size={22} /></span>{/if}</h1>
        <p class="flavor">{current.flavor}</p>

        {#if current.id === 'crew'}
          <ul class="crew">
            {#each crew as c, i (c.provider)}
              <li class="recruit" class:off={!cliReady(c)} style="animation-delay:{i * 60}ms">
                <Portrait agent={drone(c.provider)} size={44} dim={!cliReady(c)} />
                <b>{PROVIDER_LABEL[c.provider]}</b>
                <span class="r-state"><StateMark tone={cliReady(c) ? 'done' : c.path ? 'wait' : 'idle'} size={8} />{cliReady(c) ? 'Recruited' : c.path ? 'Sign in' : 'Not installed'}</span>
              </li>
            {/each}
          </ul>
          <p class="hint">{quest.readyClis ? `${quest.readyClis} recruited${quest.readyClis > 1 ? ` · +${(quest.readyClis - 1) * 25} bonus XP` : ''}` : 'Install and sign in to any agent CLI (Claude Code, Codex, Gemini CLI…), then check again.'}</p>
          <div class="actions">
            <button class="btn ghost" onclick={again} disabled={detecting}><Icon name="refresh" size={13} />{detecting ? 'Checking…' : 'Check again'}</button>
          </div>
        {:else if current.id === 'rules'}
          <div class="modes" role="radiogroup" aria-label="Difficulty">
            {#each DIFFICULTIES as d (d.id)}
              {@const on = quest.progress.flags.mode && store.settings.approval_mode === d.id}
              <button class="mode" class:on class:full={d.id === 'full'} role="radio" aria-checked={on} onclick={() => pick(d.id)}>
                <span class="m-top"><b>{d.title}</b><span class="tag" class:rec={d.id === 'balanced'}>{d.tag}</span></span>
                <span class="m-body">{d.body}</span>
              </button>
            {/each}
          </div>
          {#if confirmFull}<FullAccessConfirm onconfirm={() => apply('full')} oncancel={() => (confirmFull = false)} />{/if}
          <p class="hint">You can change this any time in Settings.</p>
        {:else if current.id === 'vault'}
          <p class="body">
            {mac ? 'macOS asks before an app reads Desktop, Documents or Downloads. Give Codly access in' : 'On macOS, give Codly access in'}
            <b>System Settings › Privacy &amp; Security › Files and Folders</b> so agents are not interrupted by a system prompt mid-task.
          </p>
          <div class="actions">
            <button class="btn primary" onclick={openFolderAccess}><Icon name="lock" size={14} />Open System Settings</button>
            <button class="btn ghost" onclick={() => quest.flag('vault')}>Already done</button>
          </div>
        {:else}
          <p class="body">Pick a git repository. Codly opens it as your base: the Lead reads it, workers get their own worktrees.</p>
          <div class="actions">
            <button class="btn primary lg" onclick={open} disabled={opening}><Icon name="folder" size={15} />{opening ? 'Opening…' : 'Open a repository'}</button>
          </div>
        {/if}

        <footer class="nav">
          <button class="btn ghost sm" onclick={() => step--} disabled={step === 0}><Icon name="arrow-left" size={13} />Back</button>
          {#if step < STEPS.length - 1}
            <button class="btn sm" onclick={() => step++}>{quest.isDone(current.id) ? 'Next quest' : 'Skip for now'}<Icon name="arrow-right" size={13} /></button>
          {/if}
        </footer>
      </section>
    {/key}
  </div>
  <footer class="foot">
    <button class="btn ghost sm" onclick={() => (ui.settingsOpen = true)}><Icon name="settings" size={13} />Settings</button>
  </footer>
</div>

<style>
  .flow { position: fixed; inset: 0; display: flex; flex-direction: column; background: var(--night); overflow: hidden; }
  .sky { position: absolute; inset: 0; pointer-events: none; background: radial-gradient(120% 80% at 50% 125%, var(--night-glow), transparent 70%); }
  .drag { position: relative; height: 40px; flex-shrink: 0; }
  .hud { position: relative; width: min(1080px, calc(100vw - 64px)); margin: 0 auto; display: flex; align-items: center; gap: 24px; }
  .brand { display: inline-flex; align-items: center; gap: 9px; font-weight: var(--w-semi); font-size: 15px; }
  .stats { flex: 1; display: flex; align-items: center; gap: 12px; justify-content: flex-end; }
  .badge { display: inline-flex; align-items: center; gap: 8px; height: 28px; padding: 0 12px 0 4px; border-radius: var(--r-full); background: var(--surface-2); font-weight: var(--w-semi); font-size: var(--t-sm); }
  .lv { display: grid; place-items: center; width: 20px; height: 20px; border-radius: 50%; background: var(--aurora); color: var(--on-aurora); font-size: var(--t-xs); }
  .bar { width: 180px; height: 6px; border-radius: 3px; background: var(--surface-2); overflow: hidden; }
  .bar span { display: block; height: 100%; background: var(--aurora); border-radius: inherit; transition: width var(--d-5) var(--spring); }
  .xp { color: var(--live); font-weight: var(--w-semi); font-size: var(--t-sm); min-width: 56px; }
  .page { position: relative; flex: 1; overflow-y: auto; width: min(1080px, calc(100vw - 64px)); margin: 0 auto; padding: 5vh 0 24px; display: grid; grid-template-columns: 260px minmax(0, 1fr); gap: 48px; align-items: start; }
  .line { display: flex; flex-direction: column; gap: 10px; animation: rise-in var(--d-5) var(--spring) both; }
  .line ol { display: flex; flex-direction: column; gap: 2px; }
  .q { display: flex; align-items: center; gap: 10px; width: 100%; padding: 9px 10px; border-radius: var(--r-md); text-align: left; color: var(--ink-3); }
  .q:hover { background: var(--surface-1); color: var(--ink-1); }
  .q:disabled { opacity: 0.6; }
  .q.on { background: var(--surface-2); color: var(--ink-1); }
  .q.done { color: var(--ink-2); }
  .dot { display: grid; place-items: center; width: 22px; height: 22px; border-radius: 50%; flex-shrink: 0; box-shadow: inset 0 0 0 1.5px var(--line-3); font-size: var(--t-xs); font-weight: var(--w-semi); }
  .done .dot { background: var(--ok-soft); color: var(--ok); box-shadow: none; }
  .q-title { flex: 1; font-weight: var(--w-med); }
  .q-xp { font-size: var(--t-xs); color: var(--ink-3); }
  .card { display: flex; flex-direction: column; gap: 16px; padding: 28px 30px 20px; border-radius: var(--r-xl); animation: rise-in var(--d-5) var(--spring) both; }
  h1 { font-size: clamp(32px, 4vw, 46px); line-height: 1.05; display: flex; align-items: center; gap: 12px; }
  .tick { color: var(--ok); display: inline-flex; animation: pop-in var(--d-4) var(--spring) both; }
  .flavor { font-size: 15px; line-height: 24px; color: var(--ink-2); max-width: 56ch; }
  .body { line-height: 22px; color: var(--ink-2); max-width: 60ch; }
  .hint { font-size: var(--t-sm); color: var(--ink-3); }
  .actions { display: flex; gap: 10px; flex-wrap: wrap; }
  .crew { display: grid; grid-template-columns: repeat(auto-fill, minmax(130px, 1fr)); gap: 8px; }
  .recruit { display: flex; flex-direction: column; align-items: center; gap: 6px; padding: 16px 10px 12px; border-radius: var(--r-lg); background: var(--surface-1); text-align: center; animation: rise-in var(--d-4) var(--spring) both; }
  .recruit:not(.off) { box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--ok) 35%, transparent); }
  .recruit b { font-size: var(--t-md); }
  .recruit.off b { color: var(--ink-3); }
  .r-state { display: inline-flex; align-items: center; gap: 6px; font-size: var(--t-xs); color: var(--ink-3); }
  .modes { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; }
  .mode { display: flex; flex-direction: column; gap: 6px; padding: 14px 16px; border-radius: var(--r-lg); text-align: left; background: var(--surface-1); transition: background-color var(--d-2), transform var(--d-2) var(--spring); }
  .mode:hover { background: var(--surface-2); transform: translateY(-1px); }
  .mode.on { background: var(--surface-3); box-shadow: inset 0 0 0 1.5px var(--ink-2); }
  .mode.full.on { box-shadow: inset 0 0 0 1.5px var(--amber); }
  .m-top { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .tag { font-size: var(--t-xs); padding: 2px 8px; border-radius: var(--r-full); background: var(--surface-2); color: var(--ink-3); }
  .tag.rec { background: var(--live-soft); color: var(--live); }
  .full .tag { background: var(--amber-soft); color: var(--amber-ink); }
  .m-body { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .nav { display: flex; justify-content: space-between; padding-top: 12px; margin-top: 4px; box-shadow: inset 0 1px 0 var(--line); }
  .foot { position: relative; width: min(1080px, calc(100vw - 64px)); margin: 0 auto; display: flex; justify-content: flex-end; padding: 12px 0 22px; }
  @media (max-width: 900px) { .page { grid-template-columns: 1fr; gap: 20px; } .modes { grid-template-columns: 1fr; } .bar { width: 100px; } }
</style>
