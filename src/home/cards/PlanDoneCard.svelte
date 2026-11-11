<!--
  The plan landed on its integration branch: what changed in total, what each task did, what
  the agents made, and the decision that is yours — merge into your branch, or keep the branch.
-->
<script lang="ts">
  import { keepBranch, mergePlan } from '../../lib/state/actions';
  import { diffstats } from '../../lib/state/diffstats.svelte';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { shortSha } from '../../lib/util/format';
  import { idVar, GLOSSARY } from '../../lib/util/labels';
  import ArtifactThumb from '../../components/ArtifactThumb.svelte';
  import Icon from '../../components/Icon.svelte';

  interface Props {
    planId: string;
  }
  let { planId }: Props = $props();
  const plan = $derived(store.planMap.get(planId) ?? null);
  const tasks = $derived(store.tasks.filter((t) => t.plan_id === planId).sort((a, b) => a.key.localeCompare(b.key, undefined, { numeric: true })));
  const stat = $derived(diffstats.plan[planId] ?? null);
  const arts = $derived(store.artifacts.filter((a) => a.task_id && tasks.some((t) => t.id === a.task_id)));
  const merged = $derived.by(() => {
    void store.rev;
    return store.planMerges[planId] ?? null;
  });
  const kept = $derived(ui.keptPlans.includes(planId) || !!store.planMap.get(planId)?.kept);
  const target = $derived(store.workspace?.current_branch ?? store.workspace?.default_branch ?? 'main');
  let busy = $state(false);
  async function merge() {
    busy = true;
    await mergePlan(planId);
    busy = false;
  }
</script>

{#if plan}
  <article class="card" class:open={!merged && !kept}>
    <p class="eyebrow">{merged ? `Merged into ${target}` : kept ? 'Branch kept' : 'Plan finished · ready to merge'}</p>
    <h3 class="serif">{plan.summary}</h3>
    <p class="stats num">
      {#if stat}<span class="add">+{stat.added}</span> <span class="del">−{stat.removed}</span> across {stat.files} files ·{/if}
      {tasks.filter((t) => t.status === 'done').length} of {tasks.length} tasks merged into <code data-tip={GLOSSARY.integration}>{plan.integration_branch ?? 'the integration branch'}</code>
    </p>
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
        <button class="btn primary" disabled={busy || kept} onclick={merge}><Icon name="merge" size={14} />Merge into {target}</button>
        <button class="btn" disabled={busy || kept} onclick={() => keepBranch(planId)}>Keep the branch</button>
        <button class="btn ghost" onclick={() => ui.openDiff({ kind: 'plan', id: planId })}>View changes</button>
      {/if}
    </footer>
  </article>
{/if}

<style>
  .card { position: relative; display: flex; flex-direction: column; gap: 12px; padding: 20px 22px 18px; border-radius: var(--r-lg); background: var(--surface-1); }
  .card.open::before { content: ''; position: absolute; left: 18px; right: 18px; top: 0; height: 1px; background: var(--aurora); }
  .eyebrow { margin-bottom: -6px; }
  h3 { font-size: var(--d-md); line-height: 1.12; }
  .stats { font-size: var(--t-md); color: var(--ink-2); }
  .stats code { font-size: 11.5px; color: var(--ink-2); }
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
