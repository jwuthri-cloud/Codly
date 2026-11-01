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
  import { api } from '../lib/api';
  import { setRequestPrefs } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { PROVIDER_LABEL } from '../lib/util/labels';
  import { EFFORTS, joinModel, splitModel } from '../lib/util/team';
  import Icon from '../components/Icon.svelte';
  import Menu, { type MenuItem } from '../components/Menu.svelte';

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

  let models = $state<string[]>([]);
  $effect(() => {
    const p = cli;
    models = [];
    api.listModels(p).then((l) => p === cli && (models = l)).catch(() => {});
  });

  let open = $state<'mode' | 'model' | null>(null);
  let modeBtn: HTMLButtonElement | undefined = $state();
  let modelBtn: HTMLButtonElement | undefined = $state();

  const modeItems = $derived<MenuItem[]>([
    { kind: 'header', label: 'Mode' },
    ...MODES.map((m) => ({ label: m.label, hint: m.hint, icon: m.icon, checked: m.id === mode, run: () => setRequestPrefs(thread, { mode: m.id }) })),
    { kind: 'header', label: 'Permissions' },
    ...PERMISSIONS.map((p) => ({ label: p.label, hint: p.hint, checked: p.id === perm, run: () => setRequestPrefs(thread, { approval_mode: p.id }) })),
  ]);
  const efforts = $derived(EFFORTS[cli] ?? []);
  const setModel = (name: string, effort: string) => setRequestPrefs(thread, { model: joinModel(name, effort) });
  // A model belongs to its CLI: switching CLI goes back to that one's default model.
  const setCli = (p: Provider | null) => setRequestPrefs(thread, { provider: p, model: null });
  const modelItems = $derived<MenuItem[]>([
    ...(clis.length > 1
      ? [
          { kind: 'header' as const, label: 'Lead CLI' },
          { label: 'Default', hint: PROVIDER_LABEL[usualCli], checked: !prefs.provider, run: () => setCli(null) },
          ...clis.map((p) => ({ label: PROVIDER_LABEL[p], checked: prefs.provider === p, run: () => setCli(p) })),
        ]
      : []),
    { kind: 'header', label: `${PROVIDER_LABEL[cli]} model` },
    { label: 'Default', checked: !model.name, run: () => setModel('', model.effort) },
    ...[...new Set([...models, ...(model.name ? [model.name] : [])])].map((m) => ({ label: m, checked: m === model.name, run: () => setModel(m, model.effort) })),
    ...(efforts.length
      ? [
          { kind: 'header' as const, label: 'Thinking' },
          { label: 'Default', checked: !model.effort, run: () => setModel(model.name, '') },
          ...efforts.map((e) => ({ label: e === 'xhigh' ? 'Extra high' : e[0].toUpperCase() + e.slice(1), checked: e === model.effort, run: () => setModel(model.name, e) })),
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
</div>
{#if open === 'mode' && modeBtn}<Menu anchor={modeBtn} items={modeItems} onclose={() => (open = null)} width={340} />{/if}
{#if open === 'model' && modelBtn}<Menu anchor={modelBtn} items={modelItems} onclose={() => (open = null)} width={240} />{/if}

<style>
  .controls { display: flex; gap: 4px; padding: 6px 10px 0; }
  .chip { display: inline-flex; align-items: center; gap: 5px; height: 26px; padding: 0 8px 0 9px; border-radius: var(--r-full); font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-2); }
  .chip:hover, .chip[aria-expanded='true'] { background: var(--surface-2); color: var(--ink-1); }
  .chip.direct { color: var(--amber-ink); }
  .sub { color: var(--ink-3); font-weight: var(--w-reg); }
</style>
