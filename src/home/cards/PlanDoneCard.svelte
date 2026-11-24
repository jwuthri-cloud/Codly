<!--
  The plan landed on its integration branch: what changed in total, what each task did, what
  the agents made, and the decision that is yours — merge into your branch, or keep the branch.
  A merge that would conflict with your branch says so here, with the two ways out: until the
  conflict is resolved the card does not say "ready to merge", and the one primary button is
  the step's own "Have an agent resolve it". The same after a merge refused for checks that
  failed on the merge result ("Have an agent fix it" is the one primary button) or that could
  not run there (no primary button): Merge is a plain one. While a pull request for the plan is
  open and its checks are not read yet, still run or failed, the primary button is to look at the pull
  request; Merge stays. Both keep their places while the checks change: only which one is the
  primary button does.
-->
<script lang="ts">
  import { keepBranch, mergePlan } from '../../lib/state/actions';
  import { diffstats } from '../../lib/state/diffstats.svelte';
  import { conflictsWith, mergeBlock, mergeKey, mergeUi } from '../../lib/state/mergeState.svelte';
  import { prKey, prUi } from '../../lib/state/pullRequest.svelte';
  import { planSpent, spentVersion } from '../../lib/state/spent.svelte';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { PLAN_SPENT_TIP, spentLine } from '../../lib/util/cost';
  import { plural, shortSha } from '../../lib/util/format';
  import { idVar, GLOSSARY } from '../../lib/util/labels';
  import { openExternal } from '../../lib/util/markdown';
  import ArtifactThumb from '../../components/ArtifactThumb.svelte';
  import Icon from '../../components/Icon.svelte';
  import MergeConflictStep from '../../components/MergeConflictStep.svelte';
  import RepoSyncBar from '../../components/RepoSyncBar.svelte';
  import PullRequestStep from '../../components/PullRequestStep.svelte';
  import PreviewStep from '../../components/PreviewStep.svelte';
  import { pullRequestFirst, pullRequestTip } from '../mergeChoice';

  interface Props {
    planId: string;
  }
  let { planId }: Props = $props();
  const plan = $derived(store.planMap.get(planId) ?? null);
  const tasks = $derived(store.tasks.filter((t) => t.plan_id === planId).sort((a, b) => a.key.localeCompare(b.key, undefined, { numeric: true })));
  const stat = $derived(diffstats.plan[planId] ?? null);
  // What the plan's tasks used in all (its runs are over: asked once, and again if a task changes state).
  const spent = $derived(planSpent.plan[planId] ?? null);
  $effect(() => {
    if (plan) planSpent.ensure(planId, spentVersion(tasks.map((t) => t.status), false, 0));
  });
  const arts = $derived(store.artifacts.filter((a) => a.task_id && tasks.some((t) => t.id === a.task_id)));
  const merged = $derived.by(() => {
    void store.rev;
    return store.mergeOf(planId);
  });
  const kept = $derived(ui.keptPlans.includes(planId) || !!store.planMap.get(planId)?.kept);
  const target = $derived(store.workspace?.current_branch ?? store.workspace?.default_branch ?? 'main');
  /** The branch it conflicts with: nothing can merge until that is resolved, and the step below has the ways out. */
  const conflict = $derived(merged || kept ? null : conflictsWith({ kind: 'plan', id: planId }));
  /** A conflict or checks that refused the merge: the way out below is the main button, not Merge. */
  const blocked = $derived(merged || kept ? null : mergeBlock(mergeUi.states[mergeKey({ kind: 'plan', id: planId })]));
  /** A pull request for it is open and its checks are not read, still run or failed: look at those before you merge here. */
  const pr = $derived(prUi.opened[prKey({ kind: 'plan', id: planId })] ?? null);
  const prChecking = $derived(pullRequestFirst(pr));
  let working = $state(false);
  /** A merge or keep from here, or a merge of this plan from elsewhere (the chip at the thread's end), is under way. */
  const busy = $derived(working || mergeUi.merging.includes(planId));
  // `unchecked`: after the plan's checks failed on the merge result, you merge anyway.
  async function merge(unchecked = false) {
    working = true;
    await mergePlan(planId, null, unchecked);
    working = false;
  }
  async function keep() {
    working = true;
    await keepBranch(planId);
    working = false;
  }
</script>

{#if plan}
  <article class="card" class:open={!merged && !kept}>
    <p class="eyebrow">{merged ? `Merged into ${target}` : kept ? 'Branch kept' : conflict ? `Plan finished · conflicts with ${conflict}` : 'Plan finished · ready to merge'}</p>
    <h3 class="serif">{plan.summary}</h3>
    <p class="stats num">
      {#if stat}<span class="add">+{stat.added}</span> <span class="del">−{stat.removed}</span> across {plural(stat.files, 'file')} ·{/if}
      {tasks.filter((t) => t.status === 'done').length} of {tasks.length} tasks merged into <code data-tip={GLOSSARY.integration}>{plan.integration_branch ?? 'the integration branch'}</code>
    </p>
    {#if spent?.tokens}<p class="used num"><span data-tip={PLAN_SPENT_TIP}>Used {spentLine(spent)}</span></p>{/if}
    <ul class="tasks">
      {#each tasks as t (t.id)}
        {@const a = t.assignee ? store.agentMap.get(t.assignee) : null}
        <li style:--hue={idVar(a?.color ?? 0)}>
          <button onclick={() => ui.selectTask(t.id)}>
            <span class="dot"></span><span class="k">{t.key}</span><span class="t truncate">{t.title}</span><span class="a">{a?.name ?? ''}</span>
          </button>
        </li>
      {/each}
    </ul>
    {#if arts.length}
      <div class="arts">
        {#each arts as a (a.id)}
          <button class="art" onclick={() => ui.openArtifact(a.id)}>
            <span class="tb"><ArtifactThumb artifact={a} /></span>
            <span class="cap truncate">{a.title}</span>
          </button>
        {/each}
      </div>
    {/if}
    <footer>
      {#if merged}
        <span class="done"><Icon name="check" size={13} stroke={2} />Merged into <code>{target}</code> at <code>{shortSha(merged.sha)}</code></span>
        <button class="btn ghost sm" onclick={() => ui.openDiff({ kind: 'plan', id: planId })}>View changes</button>
      {:else}
        <!-- Both stay where they are while the checks change: only which one is the main button does. -->
        {#if pr?.state === 'open' && !conflict}
          {@const url = pr.url}
          <button class="btn" class:primary={prChecking && !blocked} onclick={() => openExternal(url)} data-tip={prChecking ? pullRequestTip(pr) : 'Open it in your browser'}><Icon name="link" size={14} />View the pull request</button>
        {/if}
        <button class="btn" class:primary={!blocked && !prChecking} disabled={busy || kept} onclick={() => merge()}><Icon name="merge" size={14} />Merge into {target}</button>
        <button class="btn" disabled={busy || kept} onclick={keep}>Keep the branch</button>
        <button class="btn ghost" onclick={() => ui.openDiff({ kind: 'plan', id: planId })}>View changes</button>
      {/if}
    </footer>
    <!-- Until the merge: Codly removes the plan's worktree then. -->
    {#if plan.integration_worktree && !merged}<PreviewStep source={{ kind: 'plan', id: planId }} worktree={plan.integration_worktree} />{/if}
    {#if !merged && !kept}<MergeConflictStep source={{ kind: 'plan', id: planId }} refresh={busy} onunchecked={() => merge(true)} />{/if}
    {#if !kept}<RepoSyncBar refresh={merged?.sha ?? busy} {planId} />{/if}
    {#if plan.integration_branch}<PullRequestStep source={{ kind: 'plan', id: planId }} canPush={!merged} mergedInto={merged ? target : null} />{/if}
  </article>
{/if}

<style>
  .card { position: relative; display: flex; flex-direction: column; gap: 12px; padding: 20px 22px 18px; border-radius: var(--r-lg); background: var(--surface-1); }
  .card.open::before { content: ''; position: absolute; left: 18px; right: 18px; top: 0; height: 1px; background: var(--aurora); }
  .eyebrow { margin-bottom: -6px; }
  h3 { font-size: var(--d-md); line-height: 1.12; }
  .stats { font-size: var(--t-md); color: var(--ink-2); }
  .stats code { font-size: 11.5px; color: var(--ink-2); }
  .used { margin-top: -8px; font-size: var(--t-sm); color: var(--ink-3); }
  .add { color: var(--ok); }
  .del { color: var(--bad); }
  .tasks { display: flex; flex-direction: column; }
  .tasks button { display: flex; align-items: center; gap: 10px; width: 100%; height: 32px; padding: 0 8px; margin: 0 -8px; border-radius: var(--r-sm); text-align: left; font-size: var(--t-md); }
  .tasks button:hover { background: var(--surface-1); }
  .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--hue); flex-shrink: 0; }
  .k { font-family: var(--font-mono); font-size: 11px; color: var(--ink-3); }
  .t { flex: 1; color: var(--ink-1); }
  .a { font-size: var(--t-sm); color: var(--ink-3); }
  .arts { display: grid; grid-template-columns: repeat(auto-fill, minmax(128px, 1fr)); gap: 10px; }
  .art { display: flex; flex-direction: column; gap: 6px; text-align: left; }
  .tb { display: block; height: 82px; border-radius: var(--r-md); overflow: hidden; box-shadow: inset 0 0 0 1px var(--line); transition: transform var(--d-2) var(--spring); }
  .art:hover .tb { transform: translateY(-2px); }
  .cap { font-size: var(--t-xs); color: var(--ink-3); }
  footer { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; margin-top: 2px; }
  .done { display: inline-flex; align-items: center; gap: 6px; font-size: var(--t-md); color: var(--ok); margin-right: 8px; }
  .done code { font-size: 11.5px; color: var(--ink-2); }
</style>
