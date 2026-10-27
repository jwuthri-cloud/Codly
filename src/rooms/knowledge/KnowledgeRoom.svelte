<!-- Knowledge room: team memory, the knowledge graph, and code search with citations. -->
<script lang="ts">
  import { store } from '../../lib/state/store.svelte';
  import { ui, type KnowledgeLens } from '../../lib/state/ui.svelte';
  import Icon from '../../components/Icon.svelte';
  import RoomHead from '../RoomHead.svelte';
  import GraphLens from './GraphLens.svelte';
  import MemoryLens from './MemoryLens.svelte';
  import SearchLens from './SearchLens.svelte';
  import FilesLens from './FilesLens.svelte';
  import FindLens from './FindLens.svelte';
  import MessagesLens from './MessagesLens.svelte';

  const LENSES: { id: KnowledgeLens; label: string; icon: string }[] = [
    { id: 'memory', label: 'Memory', icon: 'memory' },
    { id: 'graph', label: 'Graph', icon: 'graph' },
    { id: 'search', label: 'Search', icon: 'search' },
    { id: 'find', label: 'Find', icon: 'search' },
    { id: 'messages', label: 'Messages', icon: 'chat' },
    { id: 'files', label: 'Files', icon: 'file' },
  ];
  const TITLE: Record<KnowledgeLens, string> = {
    memory: 'What the team remembers',
    graph: 'What Codly knows about the code',
    search: 'Search the code',
    find: 'Find what was written down',
    messages: 'Who talks to whom',
    files: 'Memory files, one per agent',
  };
  const shared = $derived(store.memories.filter((m) => m.scope === 'project' || m.scope === 'global').length);
</script>

<div class="kn">
  <RoomHead eyebrow="Knowledge" title={TITLE[ui.knowledgeLens]}>
    {#snippet sub()}
      {#if ui.knowledgeLens === 'memory'}<span class="num">{shared} shared memories go into every agent’s brief</span>
      {:else if ui.knowledgeLens === 'graph'}<span class="num">{store.triples.filter((t) => t.valid_to == null).length} facts, most observed from real events</span>
      {:else if ui.knowledgeLens === 'find'}<span>Exact words or meaning, across the whiteboard, tasks and memory</span>
      {:else if ui.knowledgeLens === 'messages'}<span>Agents and the requests, questions and notes between them</span>
      {:else if ui.knowledgeLens === 'files'}<span>Edit what an agent should always know; it joins its brief at task start</span>
      {:else}<span>Paths, symbols and code, with the evidence for each hit</span>{/if}
    {/snippet}
    {#snippet right()}
      <div class="lenses" role="radiogroup" aria-label="Lens">
        {#each LENSES as l (l.id)}<button role="radio" aria-checked={ui.knowledgeLens === l.id} onclick={() => (ui.knowledgeLens = l.id)}><Icon name={l.icon} size={13} />{l.label}</button>{/each}
      </div>
    {/snippet}
  </RoomHead>
  {#if ui.knowledgeLens === 'memory'}<MemoryLens />{:else if ui.knowledgeLens === 'graph'}<GraphLens />{:else if ui.knowledgeLens === 'find'}<FindLens />{:else if ui.knowledgeLens === 'messages'}<MessagesLens />{:else if ui.knowledgeLens === 'files'}<FilesLens />{:else}<SearchLens />{/if}
</div>

<style>
  .kn { flex: 1; min-height: 0; display: flex; flex-direction: column; }
</style>
