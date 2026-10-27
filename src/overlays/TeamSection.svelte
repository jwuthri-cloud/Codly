<!--
  The standing team: members the Lead delegates to by name. "All repos" edits the global team
  (Settings); "This repo" switches global members off here and adds repo-only members. Every
  edit saves at once (optimistic, reverted with a toast if the core refuses it).
-->
<script lang="ts">
  import type { MemberKind } from '../lib/gen/MemberKind';
  import type { Provider } from '../lib/gen/Provider';
  import type { TeamMember } from '../lib/gen/TeamMember';
  import { api } from '../lib/api';
  import { saveWorkspaceTeam, updateSettings } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import { moveItem } from '../lib/util/accounts';
  import { PROVIDER_LABEL } from '../lib/util/labels';
  import { EFFORTS, PRESETS, ROLE_TITLES, fromPreset, joinModel, memberTitle, splitModel, teamProblem } from '../lib/util/team';
  import Icon from '../components/Icon.svelte';

  const HARNESSES: Provider[] = ['claude', 'codex', 'gemini', 'opencode'];
  const usable = $derived(HARNESSES.filter((p) => store.clis.some((c) => c.provider === p && c.path && !c.problem && c.logged_in !== false)));
  const installed = $derived(HARNESSES.filter((p) => store.clis.some((c) => c.provider === p && c.path)));

  let scope = $state<'all' | 'repo'>('all');
  const repoScope = $derived(scope === 'repo' && !!store.workspace);
  const global = $derived(store.settings.team);
  const repo = $derived(store.workspaceTeam);
  /** The list this view edits. */
  const list = $derived(repoScope ? repo.members : global);

  function save(next: TeamMember[]) {
    const problem = teamProblem(next, repoScope ? global : repo.members);
    if (problem) {
      toast({ kind: 'error', title: 'Couldn’t save the team', body: problem });
      return;
    }
    if (repoScope) saveWorkspaceTeam({ ...repo, members: next });
    else updateSettings({ team: next });
  }
  function patch(i: number, p: Partial<TeamMember>) {
    save(list.map((m, j) => (j === i ? { ...m, ...p } : m)));
  }
  function toggleHere(id: string, on: boolean) {
    saveWorkspaceTeam({ ...repo, off: on ? repo.off.filter((x) => x !== id) : [...repo.off, id] });
  }
  function add(preset = PRESETS[0]) {
    const taken = [...global, ...repo.members].map((m) => m.name);
    const m = fromPreset(preset, usable, taken);
    if (!m) return toast({ kind: 'error', title: 'No agent CLI is signed in', body: 'Members run on one: sign in, then add them.' });
    save([...list, m]);
  }

  /** Tasks waiting for a member (in this repo): removing them sends those to consultants. */
  const queued = (id: string) => {
    const agent = store.agents.find((a) => a.member === id);
    return agent ? store.tasks.filter((t) => t.assignee === agent.id && (t.status === 'pending' || t.status === 'ready')).length : 0;
  };
  let confirming = $state<string | null>(null);
  function remove(i: number) {
    const m = list[i];
    if (queued(m.id) && confirming !== m.id) {
      confirming = m.id;
      return;
    }
    confirming = null;
    save(list.filter((_, j) => j !== i));
  }

  // Model names per CLI, for the model field's suggestions (typing any name still works).
  let models = $state<Partial<Record<Provider, string[]>>>({});
  $effect(() => {
    for (const p of new Set(list.map((m) => m.provider)))
      if (!(p in models)) {
        models[p] = [];
        api.listModels(p).then((l) => (models[p] = l)).catch(() => {});
      }
  });
  const agentOf = (id: string) => store.agents.find((a) => a.member === id);
</script>

<section>
  <h3 class="eyebrow">Team</h3>
  <p class="desc">
    Standing members the Lead delegates to by name. Each keeps its own conversation, memory and worktree in every repo, and only runs
    when it has work. For anything else the Lead hires a consultant for one task and lets it go after.
  </p>
  {#if store.workspace}
    <div class="seg" role="radiogroup" aria-label="Team scope">
      <button role="radio" aria-checked={scope === 'all'} class:on={scope === 'all'} onclick={() => (scope = 'all')}>All repos</button>
      <button role="radio" aria-checked={scope === 'repo'} class:on={scope === 'repo'} onclick={() => (scope = 'repo')}>In {store.workspace.name}</button>
    </div>
  {/if}

  {#if repoScope && global.length}
    <ul class="members">
      {#each global as m (m.id)}
        {@const on = !repo.off.includes(m.id)}
        <li class="member compact" class:off={!on || !m.enabled}>
          <div class="m-top">
            <span class="m-name">{m.name}</span>
            <span class="muted small">{memberTitle(m)} · {PROVIDER_LABEL[m.provider]}{m.model ? ` ${m.model}` : ''}</span>
            <span class="grow"></span>
            {#if !m.enabled}<span class="muted small">off in all repos</span>{/if}
            <input type="checkbox" class="switch" checked={on} disabled={!m.enabled} aria-label={`${m.name} in this repo`} onchange={(e) => toggleHere(m.id, e.currentTarget.checked)} />
          </div>
        </li>
      {/each}
    </ul>
    <p class="r-title">Only in {store.workspace?.name}</p>
  {/if}

  {#if list.length}
    <ul class="members">
      {#each list as m, i (m.id || i)}
        {@const model = splitModel(m.model)}
        {@const agent = agentOf(m.id)}
        <li class="member" class:off={!m.enabled}>
          <div class="m-top">
            <input class="m-name-in" value={m.name} aria-label="Name" spellcheck="false" maxlength="24" onchange={(e) => patch(i, { name: e.currentTarget.value.trim() })} />
            <span class="badge">{memberTitle(m)}</span>
            {#if agent && agent.status !== 'idle' && agent.status !== 'offline'}<span class="muted small">{agent.status.replace('_', ' ')}</span>{/if}
            <span class="grow"></span>
            <button class="icon-btn sm" aria-label="Move up" disabled={i === 0} onclick={() => save(moveItem(list, i, i - 1))}><Icon name="chevron-up" size={14} /></button>
            <button class="icon-btn sm" aria-label="Move down" disabled={i === list.length - 1} onclick={() => save(moveItem(list, i, i + 1))}><Icon name="chevron-down" size={14} /></button>
            <input type="checkbox" class="switch" checked={m.enabled} aria-label={`${m.name} on`} onchange={(e) => patch(i, { enabled: e.currentTarget.checked })} />
          </div>
          <div class="m-grid">
            <label class="field">
              <span class="f-label">Title</span>
              <select class="input" value={memberTitle(m)} onchange={(e) => patch(i, { title: e.currentTarget.value })}>
                {#if !ROLE_TITLES.includes(memberTitle(m))}<option value="">{memberTitle(m)}</option>{/if}
                {#each ROLE_TITLES as t (t)}<option value={t}>{t}</option>{/each}
              </select>
            </label>
            <label class="field">
              <span class="f-label">Harness</span>
              <select class="input" value={m.provider} onchange={(e) => patch(i, { provider: e.currentTarget.value as Provider, model: null })}>
                {#each installed.includes(m.provider) ? installed : [m.provider, ...installed] as p (p)}
                  <option value={p} disabled={!usable.includes(p) && p !== m.provider}>{PROVIDER_LABEL[p]}{usable.includes(p) ? '' : ' (unavailable)'}</option>
                {/each}
              </select>
            </label>
            <label class="field">
              <span class="f-label">Model</span>
              <input class="input" list={`models-${i}`} value={model.name} placeholder="Default" spellcheck="false" autocomplete="off" onchange={(e) => patch(i, { model: joinModel(e.currentTarget.value, model.effort) })} />
              <datalist id={`models-${i}`}>{#each models[m.provider] ?? [] as x (x)}<option value={x}></option>{/each}</datalist>
            </label>
            {#if EFFORTS[m.provider]}
              <label class="field">
                <span class="f-label">Effort</span>
                <select class="input" value={model.effort} onchange={(e) => patch(i, { model: joinModel(model.name, e.currentTarget.value) })}>
                  <option value="">Default</option>
                  {#each EFFORTS[m.provider] ?? [] as x (x)}<option value={x}>{x === 'xhigh' ? 'Extra high' : x[0].toUpperCase() + x.slice(1)}</option>{/each}
                </select>
              </label>
            {/if}
            <label class="field">
              <span class="f-label">Does</span>
              <select class="input" value={m.kind} onchange={(e) => patch(i, { kind: e.currentTarget.value as MemberKind })}>
                <option value="worker">Tasks</option>
                <option value="reviewer">Reviews</option>
              </select>
            </label>
          </div>
          <label class="field">
            <span class="f-label">What they do <span class="muted">(added to their instructions)</span></span>
            <textarea class="input role" rows="2" maxlength="400" value={m.role} placeholder="Owns crates/, prefers small PRs" onchange={(e) => patch(i, { role: e.currentTarget.value })}></textarea>
          </label>
          <div class="m-foot">
            {#if agent?.usage.context_used && agent.usage.context_window}
              <span class="muted small">Conversation at {Math.round((agent.usage.context_used * 100) / agent.usage.context_window)}% of its context</span>
            {/if}
            <span class="grow"></span>
            {#if confirming === m.id}
              <span class="small warn">{queued(m.id)} queued task(s) will go to consultants.</span>
              <button class="btn sm danger" onclick={() => remove(i)}>Remove</button>
              <button class="btn sm ghost" onclick={() => (confirming = null)}>Keep</button>
            {:else}
              <button class="btn sm ghost rm" onclick={() => remove(i)}><Icon name="trash" size={13} />Remove</button>
            {/if}
          </div>
        </li>
      {/each}
    </ul>
  {:else}
    <p class="desc">{repoScope ? 'No members only for this repo.' : 'No team yet: the Lead hires a consultant for each task.'} Start from one:</p>
  {/if}

  <div class="presets">
    {#each PRESETS as p (p.role)}
      <button class="btn sm ghost" onclick={() => add(p)}><Icon name="plus" size={13} />{memberTitle(p)}</button>
    {/each}
  </div>
</section>

<style>
  section { padding: 20px 0; box-shadow: inset 0 -1px 0 var(--line); display: flex; flex-direction: column; gap: 12px; }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .muted { color: var(--ink-3); font-weight: var(--w-reg); }
  .small { font-size: var(--t-sm); }
  .warn { color: var(--amber-ink); }
  .grow { flex: 1; }
  .r-title { font-weight: var(--w-med); }
  .seg { display: inline-flex; align-self: flex-start; gap: 2px; padding: 2px; border-radius: var(--r-full); background: var(--surface-1); }
  .seg button { height: 26px; padding: 0 12px; border-radius: var(--r-full); font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-2); }
  .seg button.on { background: var(--surface-3); color: var(--ink-1); }
  .members { display: flex; flex-direction: column; border-radius: var(--r-md); background: var(--surface-1); }
  .member { display: flex; flex-direction: column; gap: 10px; padding: 12px 14px; }
  .member.compact { padding: 8px 14px; }
  .member + .member { box-shadow: inset 0 1px 0 var(--line); }
  .member.off :is(.m-grid, .field, .m-name, .muted) { opacity: 0.55; }
  .m-top { display: flex; align-items: center; gap: 8px; min-height: 26px; min-width: 0; }
  .m-name { font-weight: var(--w-semi); }
  .m-name-in { box-shadow: none; min-width: 4ch; max-width: 22ch; field-sizing: content; height: 26px; padding: 0 6px; margin-left: -6px; border: 0; border-radius: var(--r-sm); background: transparent; font-weight: var(--w-semi); font-size: var(--t-md); outline: none; }
  .m-name-in:hover { background: var(--surface-2); }
  .m-name-in:focus { background: var(--surface-2); box-shadow: inset 0 0 0 1px var(--focus); }
  .badge { font-size: var(--t-xs); padding: 1px 7px; border-radius: var(--r-full); background: var(--surface-3); color: var(--ink-2); white-space: nowrap; }
  .m-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(120px, 1fr)); gap: 8px; }
  .field { display: flex; flex-direction: column; gap: 5px; min-width: 0; }
  .f-label { font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-2); }
  .role { resize: vertical; min-height: 44px; line-height: var(--lh-sm); padding-top: 6px; padding-bottom: 6px; }
  .m-foot { display: flex; align-items: center; gap: 6px; min-height: 26px; }
  .rm { color: var(--ink-3); }
  .rm:hover { color: var(--bad); background: var(--bad-soft); }
  .presets { display: flex; flex-wrap: wrap; gap: 4px; }
</style>
