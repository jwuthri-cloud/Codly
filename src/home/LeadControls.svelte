<!--
  The request's own pickers under the composer, Claude Code style: one for how the Lead works
  (Ask / Plan / Auto / Direct) and how much agents ask you, one for the Lead's CLI, model and
  thinking effort. Each request remembers its own; unset follows Settings. ⇧⇥ in the composer
  cycles the mode.
-->
<script lang="ts" module>
  import type { LeadMode } from '../lib/gen/LeadMode';
  import type { ApprovalMode } from '../lib/gen/ApprovalMode';
  import type { Provider } from '../lib/gen/Provider';

  export const MODES: { id: LeadMode; label: string; hint: string; icon: string }[] = [
    { id: 'ask', label: 'Ask', hint: 'Answers only', icon: 'chat' },
    { id: 'plan', label: 'Plan', hint: 'You click Run plan', icon: 'plan' },
    { id: 'auto', label: 'Auto', hint: 'Plans run at once', icon: 'play' },
    { id: 'direct', label: 'Direct', hint: 'No plan, starts now', icon: 'bolt' },
  ];
  export const PERMISSIONS: { id: ApprovalMode; label: string; hint: string }[] = [
    { id: 'strict', label: 'Ask first', hint: 'Every change asks you' },
    { id: 'balanced', label: 'Balanced', hint: 'Only risky ones ask' },
    { id: 'autonomous', label: 'Autonomous', hint: 'Worktree edits allowed' },
  ];
</script>

<script lang="ts">
  import { setRequestPrefs } from '../lib/state/actions';
  import { ModelCatalogs } from '../lib/state/modelCatalogs.svelte';
  import { store } from '../lib/state/store.svelte';
  import { PROVIDER_LABEL } from '../lib/util/labels';
  import { defaultEffortLabel, effortOptions, keptEffort, listTrouble } from '../lib/util/models';
  import { findModels, joinModel, splitModel } from '../lib/util/team';
  import Icon from '../components/Icon.svelte';
  import Menu, { type MenuItem } from '../components/Menu.svelte';
  import UsageChip from './UsageChip.svelte';

  interface Props {
    thread: string;
  }
  let { thread }: Props = $props();

  const prefs = $derived(store.requestPrefs[thread] ?? { mode: null, approval_mode: null, model: null, provider: null });
  const s = $derived(store.settings);
  const mode = $derived<LeadMode>(prefs.mode ?? (s.auto_run_plans ? 'auto' : 'plan'));
  const perm = $derived<ApprovalMode>(prefs.approval_mode ?? s.approval_mode);
  const usualCli = $derived(s.lead_provider ?? 'claude');
  /** The Lead's CLI: this request's pick, else its Lead's, else the Settings one. */
  const cli = $derived(prefs.provider ?? store.threadAgent(thread)?.provider ?? usualCli);
  // Another CLI than Settings runs on its own default model, not the Settings one (the core's rule).
  const model = $derived(splitModel(prefs.model ?? (prefs.provider && prefs.provider !== usualCli ? null : s.lead_model)));
  const clis = $derived(store.clis.filter((c) => c.path && !c.problem && c.logged_in !== false).map((c) => c.provider));

  // Each CLI's catalog (its models, and the thinking efforts each one takes), kept: switching
  // CLI in the open menu shows its list at once.
  const catalogs = new ModelCatalogs();
  const catalog = $derived(catalogs.of(cli));
  const models = $derived((catalog?.models ?? []).map((m) => m.id));
  /** Why the list is not the CLI's own (it could not be read, or the CLI could not be asked and
   *  Codly's built-in names are listed): the menu says so and offers to try again. */
  const trouble = $derived(listTrouble(catalog, catalogs.failure(cli)));
  $effect(() => catalogs.ask(cli));

  let open = $state<'mode' | 'model' | null>(null);
  /** Open the CLI and model menu (the composer's `/model`). */
  export function showModels() {
    open = 'model';
  }
  let modeBtn: HTMLButtonElement | undefined = $state();
  let modelBtn: HTMLButtonElement | undefined = $state();

  const modeItems = $derived<MenuItem[]>([
    { kind: 'header', label: 'Mode' },
    ...MODES.map((m) => ({ label: m.label, hint: m.hint, icon: m.icon, checked: m.id === mode, run: () => setRequestPrefs(thread, { mode: m.id }) })),
    { kind: 'header', label: 'Permissions' },
    ...PERMISSIONS.map((p) => ({ label: p.label, hint: p.hint, checked: p.id === perm, run: () => setRequestPrefs(thread, { approval_mode: p.id }) })),
  ]);
  // The levels the chosen model takes, as its CLI lists them.
  const efforts = $derived(effortOptions(catalog, cli, model.name, model.effort));
  const setModel = (name: string, effort: string) => setRequestPrefs(thread, { model: joinModel(name, effort) });
  /** Another model keeps the effort only if it takes that level. */
  const pickModel = (name: string) => setModel(name, keptEffort(catalog, cli, name, model.effort));
  // A model belongs to its CLI: switching CLI goes back to that one's default model.
  const setCli = (p: Provider | null) => setRequestPrefs(thread, { provider: p, model: null });
  // OpenRouter lists thousands: show a few, search for the rest.
  const SHOWN = 8;
  let query = $state('');
  const found = $derived(findModels(models, query, SHOWN));
  const modelItems = $derived<MenuItem[]>([
    ...(clis.length > 1
      ? [
          { kind: 'header' as const, label: 'Lead CLI' },
          { label: 'Default', hint: PROVIDER_LABEL[usualCli], checked: !prefs.provider, stay: true, run: () => setCli(null) },
          ...clis.map((p) => ({ label: PROVIDER_LABEL[p], checked: prefs.provider === p, stay: true, run: () => setCli(p) })),
        ]
      : []),
    { kind: 'header', label: `${PROVIDER_LABEL[cli]} model` },
    { label: 'Default', checked: !model.name, stay: true, run: () => pickModel('') },
    ...(trouble
      ? [
          { label: 'Couldn’t list the models', hint: 'Try again', icon: 'alert', stay: true, run: () => catalogs.ask(cli, true) },
          { label: trouble, disabled: true, run: () => {} },
        ]
      : []),
    ...[...new Set([...(model.name && !query ? [model.name] : []), ...found.shown])].map((m) => ({ label: m, checked: m === model.name, stay: true, run: () => pickModel(m) })),
    ...(found.total > found.shown.length ? [{ label: `${found.total - found.shown.length} more`, hint: 'type to search', disabled: true, run: () => {} }] : []),
    ...(query.trim() && !models.includes(query.trim()) ? [{ label: `Use “${query.trim()}”`, stay: true, run: () => pickModel(query.trim()) }] : []),
    ...(efforts.length
      ? [
          { kind: 'header' as const, label: 'Thinking' },
          { label: defaultEffortLabel(catalog, model.name), checked: !model.effort, stay: true, run: () => setModel(model.name, '') },
          ...efforts.map((e) => ({ label: e.label, checked: e.id === model.effort, stay: true, run: () => setModel(model.name, e.id) })),
        ]
      : []),
  ]);
  const current = $derived(MODES.find((m) => m.id === mode)!);
  const permLabel = $derived(perm === s.approval_mode && !prefs.approval_mode ? '' : PERMISSIONS.find((p) => p.id === perm)!.label);
</script>

<div class="controls">
  <button class="chip" class:direct={mode === 'direct'} bind:this={modeBtn} aria-haspopup="menu" aria-expanded={open === 'mode'} onclick={() => (open = open === 'mode' ? null : 'mode')} data-tip="How the Lead works on this request" data-tip-kbd="⇧⇥">
    <Icon name={current.icon} size={13} />{current.label}{#if permLabel}<span class="sub">· {permLabel}</span>{/if}<Icon name="chevron-down" size={12} />
  </button>
  <button class="chip" bind:this={modelBtn} aria-haspopup="menu" aria-expanded={open === 'model'} onclick={() => (open = open === 'model' ? null : 'model')} data-tip="The Lead's CLI, model and thinking for this request">
    <span class="sub">{PROVIDER_LABEL[cli]} ·</span>{model.name || 'default'}{#if model.effort}<span class="sub">· {model.effort === 'xhigh' ? 'extra high' : model.effort}</span>{/if}<Icon name="chevron-down" size={12} />
  </button>
  <UsageChip agent={store.threadAgent(thread)} />
</div>
{#if open === 'mode' && modeBtn}<Menu anchor={modeBtn} items={modeItems} onclose={() => (open = null)} width={340} />{/if}
{#if open === 'model' && modelBtn}<Menu anchor={modelBtn} items={modelItems} onclose={() => ((open = null), (query = ''))} width={260} bind:search={() => (models.length > SHOWN ? query : null), (v) => (query = v ?? '')} searchPlaceholder="Search models" />{/if}

<style>
  .controls { display: flex; gap: 4px; padding: 6px 10px 0; }
  .chip { display: inline-flex; align-items: center; gap: 5px; height: 26px; padding: 0 8px 0 9px; border-radius: var(--r-full); font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-2); }
  .chip:hover, .chip[aria-expanded='true'] { background: var(--surface-2); color: var(--ink-1); }
  .chip.direct { color: var(--amber-ink); }
  .sub { color: var(--ink-3); font-weight: var(--w-reg); }
</style>
