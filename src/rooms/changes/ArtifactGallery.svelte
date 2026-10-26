<!-- Everything the agents made — images (incl. generated), pages, drawings, documents, tables. -->
<script lang="ts">
  import type { ArtifactKind } from '../../lib/gen/ArtifactKind';
  import { KIND_LABEL } from '../../lib/state/artifacts';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { ago } from '../../lib/util/format';
  import { idVar } from '../../lib/util/labels';
  import ArtifactThumb from '../../components/ArtifactThumb.svelte';
  import EmptyState from '../../components/EmptyState.svelte';
  import Portrait from '../../components/Portrait.svelte';
  import ArtifactViewer from './ArtifactViewer.svelte';

  let kind = $state<ArtifactKind | 'all'>('all');
  const all = $derived([...store.artifacts].sort((a, b) => b.created_at - a.created_at));
  const kinds = $derived([...new Set(all.map((a) => a.kind))]);
  const list = $derived(kind === 'all' ? all : all.filter((a) => a.kind === kind));
  const open = $derived(ui.artifactOpen ? (store.artifactMap.get(ui.artifactOpen) ?? null) : null);
</script>

{#if !all.length}
  <EmptyState title="Nothing made yet." body="Images (generated ones too), pages, drawings and documents your agents produce show up here, next to who made them." />
{:else}
  <div class="gal">
    {#if kinds.length > 1}
      <div class="filters lenses" role="radiogroup" aria-label="Kind">
        <button role="radio" aria-checked={kind === 'all'} onclick={() => (kind = 'all')}>All <span class="n">{all.length}</span></button>
        {#each kinds as k (k)}<button role="radio" aria-checked={kind === k} onclick={() => (kind = k)}>{KIND_LABEL[k]}</button>{/each}
      </div>
    {/if}
    <ul class="grid">
      {#each list as a (a.id)}
        {@const ag = a.agent_id ? store.agentMap.get(a.agent_id) : null}
        {@const t = a.task_id ? store.taskMap.get(a.task_id) : null}
        <li style:--hue={idVar(ag?.color ?? 0)}>
          <button class="card" onclick={() => (ui.artifactOpen = a.id)} onpointerenter={() => ui.setHover({ agentId: a.agent_id, taskId: a.task_id, path: a.path })} onpointerleave={() => ui.setHover(null)}>
            <span class="tb"><ArtifactThumb artifact={a} /><span class="kind">{KIND_LABEL[a.kind]}</span></span>
            <span class="title truncate">{a.title}</span>
            <span class="by">{#if ag}<Portrait agent={ag} size={16} />{ag.name}{/if}{#if t}<span class="t">· {t.key}</span>{/if}<span class="when num">{ago(a.created_at, ui.now)}</span></span>
          </button>
        </li>
      {/each}
    </ul>
  </div>
  {#if open}<ArtifactViewer artifact={open} {list} />{/if}
{/if}

<style>
  .gal { flex: 1; min-height: 0; overflow-y: auto; padding: 4px 28px 28px; }
  .filters { margin-bottom: 18px; }
  .n { margin-left: 2px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(230px, 1fr)); gap: 22px 18px; }
  .card { display: flex; flex-direction: column; gap: 8px; width: 100%; text-align: left; }
  .tb { position: relative; display: block; aspect-ratio: 16 / 10; border-radius: var(--r-lg); overflow: hidden; box-shadow: inset 0 0 0 1px var(--line), 0 14px 30px -22px rgba(0, 0, 0, 0.7); transition: transform var(--d-3) var(--spring), box-shadow var(--d-3); }
  .card:hover .tb { transform: translateY(-3px); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--hue) 60%, transparent), 0 20px 36px -22px rgba(0, 0, 0, 0.7); }
  .kind { position: absolute; left: 8px; bottom: 8px; height: 20px; padding: 0 8px; border-radius: var(--r-full); background: rgba(10, 13, 20, 0.66); color: #e9ecf4; font-size: var(--t-xs); line-height: 20px; -webkit-backdrop-filter: blur(8px); backdrop-filter: blur(8px); }
  .title { font-size: var(--t-md); font-weight: var(--w-semi); color: var(--ink-1); }
  .by { display: flex; align-items: center; gap: 6px; font-size: var(--t-sm); color: var(--ink-2); margin-top: -4px; }
  .t { color: var(--ink-3); }
  .when { margin-left: auto; color: var(--ink-3); font-size: var(--t-xs); }
</style>
