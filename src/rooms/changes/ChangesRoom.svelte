<!-- Changes room: the diffs (plan and per task) and the artifacts gallery. -->
<script lang="ts">
  import { diffstats, statLine } from '../../lib/state/diffstats.svelte';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import Icon from '../../components/Icon.svelte';
  import RoomHead from '../RoomHead.svelte';
  import ArtifactGallery from './ArtifactGallery.svelte';
  import DiffsLens from './DiffsLens.svelte';

  // The plan you look at (DiffsLens's chips), else the request's.
  const picked = $derived(ui.diffTarget && (ui.diffTarget.kind === 'plan' ? ui.diffTarget.id : store.taskMap.get(ui.diffTarget.id)?.plan_id));
  const plan = $derived(ui.diffTarget ? ((picked && store.planMap.get(picked)) || null) : (store.planOf(ui.request) ?? store.activePlan));
  const stat = $derived(plan ? (diffstats.plan[plan.id] ?? null) : null);
</script>

<div class="changes">
  <RoomHead eyebrow="Changes" title={ui.changesLens === 'artifacts' ? 'What the agents made' : (plan?.summary ?? 'What changed')}>
    {#snippet sub()}
      {#if ui.changesLens === 'diffs'}
        <span class="num">{stat ? `${statLine(stat)} on the integration branch` : 'Diffs from each task’s worktree and the plan’s integration branch'}</span>
      {:else}
        <span>{store.artifacts.length} {store.artifacts.length === 1 ? 'artifact' : 'artifacts'} · opened safely: scripts stay off unless you allow them</span>
      {/if}
    {/snippet}
    {#snippet right()}
      <div class="lenses" role="radiogroup" aria-label="Lens">
        <button role="radio" aria-checked={ui.changesLens === 'diffs'} onclick={() => (ui.changesLens = 'diffs')}><Icon name="diff" size={13} />Diffs</button>
        <button role="radio" aria-checked={ui.changesLens === 'artifacts'} onclick={() => (ui.changesLens = 'artifacts')}><Icon name="image" size={13} />Artifacts <span class="n">{store.artifacts.length}</span></button>
      </div>
    {/snippet}
  </RoomHead>
  {#if ui.changesLens === 'diffs'}<DiffsLens />{:else}<ArtifactGallery />{/if}
</div>

<style>
  .changes { position: relative; flex: 1; min-height: 0; display: flex; flex-direction: column; }
</style>
