<!--
  Settings sheet. Changes apply immediately (optimistic, reverted with a toast if the core
  refuses). Tabs by topic; every option says what it does in a sentence.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { ApprovalMode } from '../lib/gen/ApprovalMode';
  import type { Provider } from '../lib/gen/Provider';
  import type { Settings } from '../lib/gen/Settings';
  import { api } from '../lib/api';
  import { updateSettings } from '../lib/state/actions';
  import { accounts } from '../lib/state/accounts.svelte';
  import { usable as accountUsable } from '../lib/util/accounts';
  import { skillsUi } from '../lib/state/skills.svelte';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { COMPANY_LABEL, PROVIDER_LABEL } from '../lib/util/labels';
  import AccountsSection from './AccountsSection.svelte';
  import FreeFlowSection from './FreeFlowSection.svelte';
  import BreakerSection from './BreakerSection.svelte';
  import InboundSection from './InboundSection.svelte';
  import ConnectionsSection from './ConnectionsSection.svelte';
  import DeveloperSection from './DeveloperSection.svelte';
  import HousekeepingSection from './HousekeepingSection.svelte';
  import TeamSection from './TeamSection.svelte';
  import TriggersSection from './TriggersSection.svelte';
  import BillingBadge from '../components/BillingBadge.svelte';
  import Icon from '../components/Icon.svelte';
  import FullAccessConfirm from '../onboarding/FullAccessConfirm.svelte';
  import { sound } from '../office/sound';

  const s = $derived(store.settings);
  const installed = $derived(store.clis.filter((c) => c.path));
  const usable = (p: Provider) => {
    const c = store.clis.find((x) => x.provider === p);
    return !!c?.path && !c.problem && c.logged_in !== false;
  };
  /** The Lead's CLI: its pinned sign-in's when that one is usable (the core's rule), else your pick. */
  const leadCli = $derived.by(() => {
    const a = s.lead_account ? accounts.byId.get(s.lead_account) : null;
    return a && accountUsable(a) ? a.provider : (s.lead_provider ?? 'claude');
  });
  const ALL: Provider[] = ['claude', 'codex', 'gemini', 'opencode', 'cursor', 'copilot'];

  function set(patch: Partial<Settings>) {
    updateSettings(patch);
  }
  /** A Lead pinned to a sign-in decides its CLI, so picking another CLI unpins it. Its model
   *  belongs to the old CLI, so it goes back to the new one's default. */
  function pickLead(p: Provider) {
    const pinned = s.lead_account ? accounts.byId.get(s.lead_account)?.provider ?? s.lead_account.split(':')[0] : null;
    const model = p === leadCli ? {} : { lead_model: null };
    set(pinned && pinned !== p ? { lead_provider: p, lead_account: null, ...model } : { lead_provider: p, ...model });
  }

  // The Lead's model and thinking effort, stored as one string: "opus:high" (the core's format).
  // ponytail: each CLI's levels today, by hand; Gemini and OpenCode take none through Codly.
  const EFFORTS: Partial<Record<Provider, string[]>> = {
    claude: ['low', 'medium', 'high', 'xhigh', 'max'],
    codex: ['low', 'medium', 'high', 'xhigh'],
  };
  const ALL_EFFORTS = ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra'];
  const leadModel = $derived.by(() => {
    const m = s.lead_model ?? '';
    const i = m.lastIndexOf(':');
    return i >= 0 && ALL_EFFORTS.includes(m.slice(i + 1)) ? { name: m.slice(0, i), effort: m.slice(i + 1) } : { name: m, effort: '' };
  });
  function setLeadModel(name: string, effort: string) {
    const n = name.trim();
    set({ lead_model: n || effort ? `${n}${effort ? `:${effort}` : ''}` : null });
  }
  let leadModels = $state<string[]>([]);
  $effect(() => {
    const p = leadCli;
    leadModels = [];
    api.listModels(p).then((l) => p === leadCli && (leadModels = l)).catch(() => {}); // typing a name still works
  });
  function move(p: Provider, d: number) {
    const list = [...s.worker_providers];
    const i = list.indexOf(p);
    const j = i + d;
    if (i < 0 || j < 0 || j >= list.length) return;
    [list[i], list[j]] = [list[j], list[i]];
    set({ worker_providers: list });
  }
  function toggleWorker(p: Provider) {
    const list = s.worker_providers.includes(p) ? s.worker_providers.filter((x) => x !== p) : [...s.worker_providers, p];
    set({ worker_providers: list });
  }

  const MODES: { id: ApprovalMode; title: string; body: string }[] = [
    { id: 'balanced', title: 'Balanced', body: 'Codly’s rules settle the obvious cases; anything risky (rm -rf, git push, outside the worktree, network installs) asks you.' },
    { id: 'strict', title: 'Strict', body: 'Every command and edit the CLI would ask about comes to you.' },
    { id: 'autonomous', title: 'Autonomous', body: 'Anything that stays inside the agent’s worktree is allowed. Risky commands still ask.' },
    { id: 'full', title: 'Full access', body: 'Workers never wait on you for commands or edits, risky ones included. Questions, permission changes, Codly’s private data and read-only agents still ask.' },
  ];
  let confirmFull = $state(false);
  function pickMode(m: ApprovalMode) {
    confirmFull = m === 'full' && s.approval_mode !== 'full';
    if (!confirmFull) set({ approval_mode: m });
  }

  const TABS = [
    { id: 'general', label: 'General', icon: 'settings' },
    { id: 'team', label: 'Team', icon: 'users' },
    { id: 'safety', label: 'Safety', icon: 'lock' },
    { id: 'automation', label: 'Automation', icon: 'bolt' },
    { id: 'developer', label: 'Developer', icon: 'code' },
  ];
  let tab = $state('general');
  // Office sound (kept on this machine, not in the core's settings).
  let soundOn = $state(!sound.muted);
  let volume = $state(sound.volume);
  $effect(() =>
    sound.onChange(() => {
      soundOn = !sound.muted;
      volume = sound.volume;
    }),
  );
  const PLAN_TOGGLES = [
    ['auto_run_plans', 'Run plans without asking', 'New requests start in Auto instead of Plan (switch per request under the composer, or ⇧⇥). You can still cancel at any time.'],
    ['lead_followups', 'Lead follow-ups', 'The Lead hears about task and plan progress and reports back in chat.'],
    ['use_my_claude_setup', 'Use my Claude Code setup', 'Claude agents also load your own MCP servers, plugins and skills, and the repo’s .claude settings. Applies to agents started after the change; approvals still go through Codly.'],
  ];
  const REVIEW_TOGGLES = [['review_gate', 'Cross-vendor review before merge', 'Another provider reviews each task’s diff before it merges.']];

  let sheet: HTMLDivElement | undefined = $state();
  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      ui.settingsOpen = false;
    }
  }
  onMount(() => sheet?.focus());
</script>

<div class="scrim" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && (ui.settingsOpen = false)}>
  <div class="sheet" role="dialog" aria-modal="true" aria-label="Settings" tabindex="-1" bind:this={sheet} onkeydown={key}>
    <header>
      <h2>Settings</h2>
      <button class="icon-btn" aria-label="Close settings" onclick={() => (ui.settingsOpen = false)}><Icon name="x" /></button>
    </header>
    <div class="main">
      <nav class="tabs" role="tablist" aria-label="Settings sections">
        {#each TABS as t (t.id)}
          <button role="tab" aria-selected={tab === t.id} class:on={tab === t.id} onclick={() => (tab = t.id)}><Icon name={t.icon} size={15} />{t.label}</button>
        {/each}
      </nav>
      <div class="body" role="tabpanel">
      {#snippet toggles(list: string[][])}
        {#each list as [k, title, body] (k)}
          <label class="row toggle">
            <div class="r-text"><span class="r-title">{title}</span><span class="desc">{body}</span></div>
            <input type="checkbox" class="switch" checked={s[k as 'auto_run_plans']} onchange={(e) => set({ [k]: (e.currentTarget as HTMLInputElement).checked })} />
          </label>
        {/each}
      {/snippet}

      {#if tab === 'general'}
      <section>
        <h3 class="eyebrow">The Lead</h3>
        <p class="desc">The supervisor you chat with. It reads the repo, proposes plans, routes tasks, and never edits files.</p>
        <div class="choices">
          {#each ALL.filter((p) => installed.some((c) => c.provider === p)) as p (p)}
            <button class="choice" class:on={leadCli === p} disabled={!usable(p)} onclick={() => pickLead(p)}>
              <span class="radio"></span>{PROVIDER_LABEL[p]} <span class="muted small">{COMPANY_LABEL[p]}</span>
              {#if !usable(p)}<span class="muted small">unavailable</span>{/if}
            </button>
          {/each}
        </div>
        <div class="row">
          <div class="r-text">
            <span class="r-title">Model</span>
            <span class="desc">Empty uses {PROVIDER_LABEL[leadCli]}’s default. A request can pick its own under the composer. Leads switch on their next message and keep their conversation.</span>
          </div>
          <input class="field" list="lead-models" value={leadModel.name} placeholder="Default" spellcheck="false" autocomplete="off" aria-label="Lead model" onchange={(e) => setLeadModel(e.currentTarget.value, leadModel.effort)} />
          <datalist id="lead-models">{#each leadModels as m (m)}<option value={m}></option>{/each}</datalist>
        </div>
        {#if EFFORTS[leadCli]}
          <div class="row">
            <div class="r-text">
              <span class="r-title">Thinking effort</span>
              <span class="desc">How long it reasons before answering. Higher plans better, slower, and uses more of your quota.</span>
            </div>
            <select class="field" value={leadModel.effort} aria-label="Lead thinking effort" onchange={(e) => setLeadModel(leadModel.name, e.currentTarget.value)}>
              <option value="">Default</option>
              {#each EFFORTS[leadCli] ?? [] as e (e)}<option value={e}>{e === 'xhigh' ? 'Extra high' : e[0].toUpperCase() + e.slice(1)}</option>{/each}
            </select>
          </div>
        {/if}
      </section>

      <section>
        <h3 class="eyebrow">Plans</h3>
        {@render toggles(PLAN_TOGGLES)}
      </section>

      <section>
        <h3 class="eyebrow">Skills</h3>
        <div class="row">
          <div class="r-text"><span class="r-title">Agent skills</span><span class="desc">See what Claude Code and Codex can load, and install more from a catalog.</span></div>
          <button class="btn sm" onclick={() => { ui.settingsOpen = false; skillsUi.show(); }}>Manage skills</button>
        </div>
      </section>

      <section>
        <h3 class="eyebrow">Getting started</h3>
        <div class="row">
          <div class="r-text"><span class="r-title">The quest</span><span class="desc">Install and sign in to your agents, add more models, and learn which model to use for images, voice and video.</span></div>
          <button class="btn sm" onclick={() => { ui.settingsOpen = false; ui.onboardingOpen = true; }}>Replay</button>
        </div>
      </section>

      <section>
        <h3 class="eyebrow">Appearance</h3>
        <div class="row">
          <div class="r-text"><span class="r-title">Theme</span><span class="desc">Night is deep blue-black glass over the office; day is warm paper.</span></div>
          <div class="seg lenses" role="radiogroup" aria-label="Theme">
            {#each [['system', 'System', 'monitor'], ['light', 'Day', 'sun'], ['dark', 'Night', 'moon']] as [id, label, icon] (id)}
              <button role="radio" aria-checked={s.theme === id} onclick={() => set({ theme: id })}><Icon name={icon} size={13} />{label}</button>
            {/each}
          </div>
        </div>
        <p class="desc">Motion follows your system’s “Reduce motion” setting.</p>
      </section>

      <section>
        <h3 class="eyebrow">Sound</h3>
        <label class="row toggle">
          <div class="r-text"><span class="r-title">Office sounds</span><span class="desc">Pool balls, the PS5 and the TV match, typing, the kitchen, jingles for merges and promotions, and music in Build/Buy. Off by default; quiet when the window is hidden.</span></div>
          <input type="checkbox" class="switch" checked={soundOn} onchange={(e) => sound.setMuted(!(e.currentTarget as HTMLInputElement).checked)} />
        </label>
        <div class="row">
          <div class="r-text"><span class="r-title">Volume</span><span class="desc">{Math.round(volume * 100)}%</span></div>
          <input type="range" class="volume" min="0" max="100" step="5" value={Math.round(volume * 100)} disabled={!soundOn} aria-label="Office sound volume" oninput={(e) => sound.setVolume(+(e.currentTarget as HTMLInputElement).value / 100)} />
        </div>
      </section>

      <HousekeepingSection />
      {:else if tab === 'team'}
      <TeamSection />

      <section>
        <h3 class="eyebrow">Consultants</h3>
        <p class="desc">Workers hired for one task when no team member takes it. They use these providers in preference order, unless the Lead picks one.</p>
        <ul class="order">
          {#each [...s.worker_providers, ...ALL.filter((p) => !s.worker_providers.includes(p))] as p (p)}
            {@const on = s.worker_providers.includes(p)}
            {@const cli = store.clis.find((c) => c.provider === p)}
            <li class:off={!on}>
              <label class="check">
                <input type="checkbox" checked={on} disabled={!usable(p) && !on} onchange={() => toggleWorker(p)} />
                <span class="pname">{PROVIDER_LABEL[p]}</span> <span class="muted small">{COMPANY_LABEL[p]}</span>
              </label>
              {#if cli?.path}<BillingBadge billing={cli.billing} plan={cli.plan} />{:else}<span class="muted small">not installed</span>{/if}
              <span class="grow"></span>
              {#if on}
                <button class="icon-btn sm" aria-label="Move up" onclick={() => move(p, -1)} disabled={s.worker_providers.indexOf(p) === 0}><Icon name="chevron-up" size={14} /></button>
                <button class="icon-btn sm" aria-label="Move down" onclick={() => move(p, 1)} disabled={s.worker_providers.indexOf(p) === s.worker_providers.length - 1}><Icon name="chevron-down" size={14} /></button>
              {/if}
            </li>
          {/each}
        </ul>
        <div class="row">
          <div class="r-text"><span class="r-title">Agents at once</span><span class="desc">How many workers may run in parallel.</span></div>
          <div class="stepper">
            <button class="icon-btn sm" aria-label="Fewer" onclick={() => set({ max_parallel: Math.max(1, s.max_parallel - 1) })}><Icon name="minus" size={14} /></button>
            <span class="num">{s.max_parallel}</span>
            <button class="icon-btn sm" aria-label="More" onclick={() => set({ max_parallel: Math.min(8, s.max_parallel + 1) })}><Icon name="plus" size={14} /></button>
          </div>
        </div>
      </section>

      <AccountsSection />
      {:else if tab === 'safety'}
      <section>
        <h3 class="eyebrow">Approvals</h3>
        <p class="desc">The default; a request can choose its own under the composer.</p>
        <div class="modes">
          {#each MODES as m (m.id)}
            <button class="mode" class:on={s.approval_mode === m.id} onclick={() => pickMode(m.id)} aria-pressed={s.approval_mode === m.id}>
              <span class="m-title"><span class="radio"></span>{m.title}</span>
              <span class="desc">{m.body}</span>
            </button>
          {/each}
        </div>
        {#if confirmFull}<FullAccessConfirm onconfirm={() => ((confirmFull = false), set({ approval_mode: 'full' }))} oncancel={() => (confirmFull = false)} />{/if}
      </section>

      <section>
        <h3 class="eyebrow">Review</h3>
        {@render toggles(REVIEW_TOGGLES)}
      </section>

      <BreakerSection />
      {:else if tab === 'automation'}
      <FreeFlowSection />
      <TriggersSection />
      <InboundSection />
      <ConnectionsSection />
      {:else if tab === 'developer'}
      <DeveloperSection />
      {/if}
      </div>
    </div>
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; z-index: var(--z-modal); background: var(--scrim); animation: fade-in var(--d-3) var(--ease-out); display: grid; place-items: center; }
  .sheet { width: min(820px, calc(100vw - 32px)); height: min(760px, calc(100vh - 64px)); display: flex; flex-direction: column; border-radius: var(--r-xl); background: var(--pop); box-shadow: var(--pop-shadow), 0 40px 100px -30px rgba(0, 0, 0, 0.6); outline: none; animation: pop-in var(--d-4) var(--spring); }
  header { display: flex; align-items: center; justify-content: space-between; height: 64px; flex-shrink: 0; padding: 0 14px 0 26px; box-shadow: inset 0 -1px 0 var(--line); }
  h2 { font-family: var(--font-display); font-weight: 400; font-size: var(--d-sm); }
  .main { flex: 1; min-height: 0; display: flex; }
  .tabs { width: 168px; flex-shrink: 0; display: flex; flex-direction: column; gap: 2px; padding: 14px 10px; box-shadow: inset -1px 0 var(--line); }
  .tabs button { display: flex; align-items: center; gap: 10px; height: 36px; padding: 0 12px; border-radius: var(--r-md); color: var(--ink-3); font-weight: var(--w-med); text-align: left; }
  .tabs button:hover { background: var(--surface-1); color: var(--ink-1); }
  .tabs button.on { background: var(--surface-3); color: var(--ink-1); }
  .body { flex: 1; min-width: 0; overflow-y: auto; padding: 4px 26px 24px; }
  @media (max-width: 640px) { .main { flex-direction: column; } .tabs { width: auto; flex-direction: row; overflow-x: auto; box-shadow: inset 0 -1px 0 var(--line); } }
  section { padding: 20px 0; box-shadow: inset 0 -1px 0 var(--line); display: flex; flex-direction: column; gap: 12px; }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .small { font-size: var(--t-sm); }
  .choices { display: flex; flex-wrap: wrap; gap: 6px; }
  .choice { display: inline-flex; align-items: center; gap: 8px; height: 34px; padding: 0 14px 0 12px; border-radius: var(--r-full); background: var(--surface-1); font-weight: var(--w-med); }
  .choice:hover { background: var(--surface-2); }
  .choice.on { background: var(--surface-3); box-shadow: inset 0 0 0 1.5px var(--ink-2); }
  .radio { width: 13px; height: 13px; border-radius: 50%; box-shadow: inset 0 0 0 1.5px var(--line-3); flex-shrink: 0; }
  .on .radio { box-shadow: inset 0 0 0 4px var(--ink-1); }
  .order { display: flex; flex-direction: column; border-radius: var(--r-md); background: var(--surface-1); }
  .order li { display: flex; align-items: center; gap: 10px; height: 42px; padding: 0 8px 0 14px; }
  .order li + li { box-shadow: inset 0 1px 0 var(--line); }
  .order li.off .pname { color: var(--ink-3); }
  .check { display: inline-flex; align-items: center; gap: 9px; }
  .pname { font-weight: var(--w-med); }
  .grow { flex: 1; }
  .row { display: flex; align-items: center; gap: 16px; min-height: 42px; }
  .r-text { flex: 1; display: flex; flex-direction: column; gap: 2px; }
  .r-title { font-weight: var(--w-med); }
  .field { width: 180px; font-size: var(--t-sm); }
  .stepper { display: inline-flex; align-items: center; gap: 4px; padding: 2px; border-radius: var(--r-full); background: var(--surface-1); }
  .stepper .num { min-width: 22px; text-align: center; font-weight: var(--w-semi); }
  .modes { display: flex; flex-direction: column; gap: 6px; }
  .mode { display: flex; flex-direction: column; gap: 3px; padding: 12px 14px; border-radius: var(--r-md); text-align: left; background: var(--surface-1); }
  .mode:hover { background: var(--surface-2); }
  .mode.on { background: var(--surface-3); box-shadow: inset 0 0 0 1.5px var(--ink-2); }
  .m-title { display: inline-flex; align-items: center; gap: 8px; font-weight: var(--w-semi); }
  .mode .desc { padding-left: 21px; }
  input[type='checkbox']:not(.switch) { accent-color: var(--aurora-m); }
  .volume { width: 160px; accent-color: var(--accent, #7ee0a1); }
</style>
