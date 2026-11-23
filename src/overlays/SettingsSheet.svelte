<!--
  Settings sheet. Changes apply immediately (optimistic, reverted with a toast if the core
  refuses). Tabs by topic; every option says what it does in a sentence.
-->
<script lang="ts">
  import type { ApprovalMode } from '../lib/gen/ApprovalMode';
  import type { Provider } from '../lib/gen/Provider';
  import type { ResumeOnLaunch } from '../lib/gen/ResumeOnLaunch';
  import type { Settings } from '../lib/gen/Settings';
  import { redetect, refreshAccounts, updateSettings } from '../lib/state/actions';
  import { accounts } from '../lib/state/accounts.svelte';
  import { ModelCatalogs } from '../lib/state/modelCatalogs.svelte';
  import { usable as accountUsable } from '../lib/util/accounts';
  import { skillsUi } from '../lib/state/skills.svelte';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { COMPANY_LABEL, PROVIDER_LABEL } from '../lib/util/labels';
  import { defaultEffortLabel, effortOptions, keptEffort, optionLabel, troubleNote } from '../lib/util/models';
  import { HOST_ALLOW_LASTS } from '../lib/util/sandbox';
  import { joinModel, splitModel } from '../lib/util/team';
  import AccountsSection from './AccountsSection.svelte';
  import ApiKeysSection from './ApiKeysSection.svelte';
  import ApprovalRulesSection from './ApprovalRulesSection.svelte';
  import FreeFlowSection from './FreeFlowSection.svelte';
  import BreakerSection from './BreakerSection.svelte';
  import WatchdogSection from './WatchdogSection.svelte';
  import InboundSection from './InboundSection.svelte';
  import NotificationsSection from './NotificationsSection.svelte';
  import ConnectionsSection from './ConnectionsSection.svelte';
  import DeveloperSection from './DeveloperSection.svelte';
  import HousekeepingSection from './HousekeepingSection.svelte';
  import StorageSection from './StorageSection.svelte';
  import TeamSection from './TeamSection.svelte';
  import TriggersSection from './TriggersSection.svelte';
  import WorktreeSection from './WorktreeSection.svelte';
  import RequiredChecksSection from './RequiredChecksSection.svelte';
  import TrustSection from './TrustSection.svelte';
  import BillingBadge from '../components/BillingBadge.svelte';
  import DiagnosticsReport from '../components/DiagnosticsReport.svelte';
  import Dialog from '../components/Dialog.svelte';
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
  // The names and the efforts each model takes are its CLI's catalog; any other name can be typed.
  const leadModel = $derived(splitModel(s.lead_model));
  const catalogs = new ModelCatalogs();
  const leadCatalog = $derived(catalogs.of(leadCli));
  const leadEfforts = $derived(effortOptions(leadCatalog, leadCli, leadModel.name, leadModel.effort));
  const leadListTrouble = $derived(troubleNote(leadCatalog, catalogs.failure(leadCli), PROVIDER_LABEL[leadCli]));
  $effect(() => catalogs.ask(leadCli));
  const setLeadModel = (name: string, effort: string) => set({ lead_model: joinModel(name, effort) });
  /** Another model keeps the effort only if it takes that level. */
  const pickLeadModel = (name: string) => setLeadModel(name, keptEffort(leadCatalog, leadCli, name.trim(), leadModel.effort));
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
    { id: 'strict', title: 'Ask first', body: 'Every command and edit the CLI would ask about comes to you.' },
    { id: 'autonomous', title: 'Autonomous', body: 'Commands and edits run without asking unless Codly sees a risk in them (rm -rf, git push, network, paths outside the worktree). This is a list of risky commands, not a sandbox: Codly reads the command, not what a script or program does once it runs. Codex agents run in their own OS sandbox. Claude agents have one only with “Sandbox Claude’s shell” on (below). Gemini, OpenCode, Cursor and Copilot agents have none.' },
    { id: 'full', title: 'Full access', body: 'Workers never wait on you for commands or edits, risky ones included. Questions, permission changes, changes to an agent’s own CLI settings, Codly’s private data and read-only agents still ask. So does a command that can lose work in your project folder itself: from an agent hired with Git isolation off, or from one that points the command there from its worktree.' },
  ];
  let confirmFull = $state(false);
  function pickMode(m: ApprovalMode) {
    confirmFull = m === 'full' && s.approval_mode !== 'full';
    if (!confirmFull) set({ approval_mode: m });
  }
  /** Claude is asked who is signed in again at once: with your shell's sign-in, or without it. */
  async function setShellAuth(on: boolean) {
    await updateSettings({ claude_shell_auth: on });
    await redetect();
    await refreshAccounts();
  }

  const TABS = [
    { id: 'general', label: 'General', icon: 'settings' },
    { id: 'team', label: 'Team', icon: 'users' },
    { id: 'safety', label: 'Safety', icon: 'lock' },
    { id: 'automation', label: 'Automation', icon: 'bolt' },
    { id: 'developer', label: 'Developer', icon: 'code' },
  ];
  let tab = $state(ui.settingsTab ?? 'general');
  ui.settingsTab = null;
  // Another tab starts at its top, not where the last one was scrolled to.
  let panel: HTMLDivElement | undefined = $state();
  $effect(() => {
    void tab;
    if (panel) panel.scrollTop = 0;
  });
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
    ['use_my_claude_setup', 'Use my own CLI setup', 'Claude agents also load your own MCP servers, plugins and skills, and the repo’s .claude settings once you trusted the folder (Safety). Codex agents also start the MCP servers of your Codex config, in every folder, trusted or not: they are yours, not the repo’s. Off: agents get Codly’s tools and your Connections only. Applies to agents started after the change; approvals still go through Codly.'],
  ];
  const LONG_RUN_TOGGLES = [
    ['keep_awake', 'Keep the Mac awake while agents work', 'The Mac does not go to sleep by itself while an agent works or a plan runs. It may sleep when all work waits for you. Closing the lid still puts it to sleep.'],
    ['confirm_quit', 'Ask before quitting while agents work', 'Agents stop when Codly quits. Their work can go on when you open Codly again.'],
  ];
  const RESUME_CHOICES: [ResumeOnLaunch, string][] = [['ask', 'Ask'], ['always', 'Always'], ['never', 'Never']];
  const SANDBOX_TOGGLES = [['codex_network_access', 'Let Codex agents use the network', 'Commands a Codex agent runs in its sandbox can reach the internet (package installs, downloads) without asking you. Codly’s approval rules (git push, curl | sh, package downloads) are not asked for these commands either. Off: each one asks first. Applies to agents started after the change.']];
  const OFFICE_TOGGLES = [
    ['game_rules', 'Office rules affect work', 'Agents can be at home or tired, and new hires wait for a job offer.'],
    ['world_ambient', 'Keep the office moving behind the chat', 'Idle agents keep moving and the break-room games play while the office shows behind the chat. Off: there the office is drawn only when something happens, which saves battery. With your system’s “Reduce motion” on, it never moves by itself.'],
  ];
  const REVIEW_TOGGLES = [['review_gate', 'Cross-vendor review before merge', 'Another provider reviews each task’s diff before it merges.']];
  // Seconds; the value in use is always one of the choices.
  const checkLimits = $derived([...new Set([300, 600, 1200, 1800, 3600, 7200, s.verify_timeout_secs])].sort((a, b) => a - b));
</script>

<Dialog label="Settings" width={820} style="height: min(760px, calc(100vh - 64px))" onclose={() => (ui.settingsOpen = false)}>
  <div class="main">
    <nav class="tabs" role="tablist" aria-label="Settings sections">
      {#each TABS as t (t.id)}
        <button role="tab" aria-selected={tab === t.id} class:on={tab === t.id} onclick={() => (tab = t.id)}><Icon name={t.icon} size={15} />{t.label}</button>
      {/each}
    </nav>
    <div class="body" role="tabpanel" bind:this={panel}>
    {#snippet toggles(list: string[][])}
      {#each list as [k, title, body] (k)}
        <label class="row toggle">
          <div class="r-text"><span class="r-title">{title}</span><span class="desc">{body}</span></div>
          <input type="checkbox" class="switch" checked={s[k as 'auto_run_plans']} aria-label={title} onchange={(e) => set({ [k]: (e.currentTarget as HTMLInputElement).checked })} />
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
        <input class="field" list="lead-models" value={leadModel.name} placeholder="Default" spellcheck="false" autocomplete="off" aria-label="Lead model" onchange={(e) => pickLeadModel(e.currentTarget.value)} />
        <datalist id="lead-models">{#each leadCatalog?.models ?? [] as m, i (i)}<option value={m.id}>{optionLabel(m)}</option>{/each}</datalist>
      </div>
      {#if leadListTrouble}<p class="desc warn" role="status">{leadListTrouble} <button class="link" onclick={() => catalogs.ask(leadCli, true)}>Try again</button></p>{/if}
      {#if leadEfforts.length}
        <div class="row">
          <div class="r-text">
            <span class="r-title">Thinking effort</span>
            <span class="desc">How long it reasons before answering. Higher plans better, slower, and uses more of your quota.</span>
          </div>
          <select class="field" value={leadModel.effort} aria-label="Lead thinking effort" onchange={(e) => setLeadModel(leadModel.name, e.currentTarget.value)}>
            <option value="">{defaultEffortLabel(leadCatalog, leadModel.name)}</option>
            {#each leadEfforts as e (e.id)}<option value={e.id}>{e.label}</option>{/each}
          </select>
        </div>
      {/if}
    </section>

    <section>
      <h3 class="eyebrow">Plans</h3>
      {@render toggles(PLAN_TOGGLES)}
    </section>

    <section>
      <h3 class="eyebrow">Long runs</h3>
      {@render toggles(LONG_RUN_TOGGLES)}
      <div class="row">
        <div class="r-text">
          <span class="r-title">Resume work when Codly opens</span>
          <span class="desc">For tasks and plans that were running when Codly last closed. Ask: one question, and nothing starts before your answer. Always: the work goes on at once. Never: it stays stopped until you press Resume work.</span>
        </div>
        <select class="field" value={s.resume_on_launch} aria-label="Resume work when Codly opens" onchange={(e) => set({ resume_on_launch: e.currentTarget.value as ResumeOnLaunch })}>
          {#each RESUME_CHOICES as [id, label] (id)}<option value={id}>{label}</option>{/each}
        </select>
      </div>
    </section>

    <section>
      <h3 class="eyebrow">Office</h3>
      {@render toggles(OFFICE_TOGGLES)}
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
        <input type="checkbox" class="switch" checked={soundOn} aria-label="Office sounds" onchange={(e) => sound.setMuted(!(e.currentTarget as HTMLInputElement).checked)} />
      </label>
      <div class="row">
        <div class="r-text"><span class="r-title">Volume</span><span class="desc">{Math.round(volume * 100)}%</span></div>
        <input type="range" class="volume" min="0" max="100" step="5" value={Math.round(volume * 100)} disabled={!soundOn} aria-label="Office sound volume" oninput={(e) => sound.setVolume(+(e.currentTarget as HTMLInputElement).value / 100)} />
      </div>
    </section>

    <NotificationsSection />

    <HousekeepingSection />
    <StorageSection />

    <section>
      <h3 class="eyebrow">Troubleshooting</h3>
      <p class="desc">For a bug report: the Codly version, your system, your agent CLIs and the last log lines. Tokens, keys, email addresses and paths under your home folder are taken out. Repository and agent names can remain: read it before you share it.</p>
      <DiagnosticsReport preview />
    </section>
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

    <ApiKeysSection />
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
      <h3 class="eyebrow">Claude Code</h3>
      <!-- Not <label>s: only the switch itself changes these, never a click on the text. -->
      <div class="row">
        <div class="r-text">
          <span class="r-title">Sandbox Claude’s shell</span>
          <span class="desc">For Claude Code agents that have a shell: workers and 1:1 agents. Off: a shell command runs with your account’s rights, and Codly’s approval rules are the only check. On: Claude Code runs each shell command in its own OS sandbox. A sandboxed command can write only in the agent’s folder, a temporary folder and the repository’s git data. It can still read your other files. It can reach only the hosts that your Claude Code settings allow: for another host Codly asks you, in every mode except Full access. {HOST_ALLOW_LASTS} Commands still go through Codly’s approval rules, as they do with the switch off: the sandbox is a second check under the rules, not a way around them. So a command can ask you once for itself and once more for a host. A command that Claude wants to run outside the sandbox asks you first, in every mode except Full access. If Claude Code does not turn its sandbox on, the agent does not start, and Codly tells you why. File edits and tool servers are not sandboxed. Codex agents always run in Codex’s own sandbox. Gemini, OpenCode, Cursor and Copilot agents have no sandbox. Applies to agents started after the change.</span>
        </div>
        <input type="checkbox" class="switch" checked={s.claude_sandbox} aria-label="Sandbox Claude’s shell" onchange={(e) => set({ claude_sandbox: e.currentTarget.checked })} />
      </div>
      <div class="row">
        <div class="r-text">
          <span class="r-title">Use my shell’s Claude sign-in</span>
          <span class="desc">Off: Codly takes <code>CLAUDE_CODE_USE_BEDROCK</code>, <code>CLAUDE_CODE_USE_VERTEX</code> and <code>CLAUDE_CODE_OAUTH_TOKEN</code> out of an agent’s environment, so Claude agents use only the sign-in of <code>claude auth login</code>. On: Claude agents keep these variables from your login shell, as Claude Code does in your terminal, and that account pays: Amazon Bedrock, Google Vertex AI, or the subscription of the token. Codly does not store the token. For a token from <code>claude setup-token</code>, check Anthropic’s terms first: they may not allow another app to use it. Applies to agents started after the change.</span>
        </div>
        <input type="checkbox" class="switch" checked={s.claude_shell_auth} aria-label="Use my shell’s Claude sign-in" onchange={(e) => setShellAuth(e.currentTarget.checked)} />
      </div>
    </section>

    <ApprovalRulesSection />

    <section>
      <h3 class="eyebrow">Sandbox</h3>
      {@render toggles(SANDBOX_TOGGLES)}
    </section>

    <section>
      <h3 class="eyebrow">Review</h3>
      {@render toggles(REVIEW_TOGGLES)}
      <div class="row">
        <div class="r-text">
          <span class="r-title">Time limit for checks</span>
          <span class="desc">How long one verify command may run before Codly stops it. A check that runs out of time blocks its task for you; it does not go back to the worker.</span>
        </div>
        <select class="field" value={String(s.verify_timeout_secs)} aria-label="Time limit for checks" onchange={(e) => set({ verify_timeout_secs: Number(e.currentTarget.value) })}>
          {#each checkLimits as secs (secs)}<option value={String(secs)}>{secs % 60 ? `${secs} s` : `${secs / 60} min`}</option>{/each}
        </select>
      </div>
    </section>

    <RequiredChecksSection />
    <TrustSection />

    <WorktreeSection />

    <BreakerSection />

    <WatchdogSection />
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
</Dialog>

<style>
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
  .warn { color: var(--amber-ink); }
  .link { color: var(--ink-1); font-weight: var(--w-med); text-decoration: underline; text-decoration-color: var(--line-3); text-underline-offset: 3px; }
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
  .volume { width: 160px; accent-color: var(--aurora-m); }
</style>
