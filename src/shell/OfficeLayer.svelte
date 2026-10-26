<!--
  The office, full-bleed behind everything: the 3D world where the crew works. Mounts
  src/office/OfficeView.svelte with exactly its contract props.
  - Lazy-loaded (three.js is its own chunk) and isolated by an error boundary: an office failure
    never takes the work down.
  - Files come from api.repoMap() (optional for the office): fetched when a workspace opens and
    refetched (debounced) when commits or merges land.
  - A fresh snapshot (epoch) remounts it so it re-derives from the new truth.
-->
<script lang="ts" module>
  const importOffice = () => import('../office/OfficeView.svelte');
  let loading: ReturnType<typeof importOffice> | null = null;
  const loadOffice = () => (loading ??= importOffice());
</script>

<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api } from '../lib/api';
  import type { Artifact } from '../lib/gen/Artifact';
  import type { EventRow } from '../lib/gen/EventRow';
  import { acquireArtifactUrl, releaseArtifactUrl } from '../lib/state/artifacts';
  import { narrate } from '../lib/state/narration';
  import type { RepoFile } from '../lib/gen/RepoFile';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';

  interface Props {
    theme: 'dark' | 'light';
  }
  let { theme }: Props = $props();

  let files = $state.raw<RepoFile[]>([]);
  let seq = 0;
  function refetch() {
    const my = ++seq;
    api.repoMap().then((f) => my === seq && (files = f)).catch(() => {});
  }
  $effect(() => {
    void store.workspace?.id;
    void store.epoch;
    if (store.workspace) refetch();
    else files = [];
  });
  $effect(() => {
    let t: ReturnType<typeof setTimeout> | undefined;
    const off = store.onEvent((ev) => {
      if (ev.kind.type !== 'committed' && ev.kind.type !== 'merged') return;
      clearTimeout(t);
      t = setTimeout(refetch, 1200);
    });
    return () => {
      off();
      clearTimeout(t);
    };
  });

  type Sel = { agentId?: string | null; taskId?: string | null; path?: string | null };
  const selection = $derived.by((): Sel | null => {
    const s = ui.selection;
    if (!s) return null;
    if (s.kind === 'agent') return { agentId: s.id };
    if (s.kind === 'task') return { taskId: s.id };
    return { path: s.id };
  });
  const hover = $derived.by((): Sel | null => {
    const h = ui.hover;
    return h.agentId || h.taskId || h.path ? { agentId: h.agentId, taskId: h.taskId, path: h.path } : null;
  });
  const backdrop = $derived(!(ui.mode === 'office' && ui.room === 'home'));

  // The office labels say exactly what the crew strip says.
  const officeNarrate = (agentId: string) => narrate(agentId);

  // Images the agents made show for real: hold a blob URL per image artifact while the office lives.
  const urls = new Map<string, string>();
  let urlRev = $state(0);
  $effect(() => {
    for (const a of store.artifacts) {
      if ((a.kind !== 'image' && a.kind !== 'svg') || urls.has(a.id)) continue;
      urls.set(a.id, '');
      acquireArtifactUrl(a)
        .then((u) => {
          if (destroyed) return releaseArtifactUrl(a.id);
          urls.set(a.id, u);
          urlRev++;
        })
        .catch(() => urls.delete(a.id));
    }
  });
  let destroyed = false;
  onDestroy(() => {
    destroyed = true;
    for (const [id, u] of urls) if (u) releaseArtifactUrl(id);
    urls.clear();
  });
  const artifactUrl = (a: Artifact) => urls.get(a.id) || null;
  /** A fresh array once more URLs are ready, so the office re-places what it shows. */
  const officeArtifacts = $derived.by(() => {
    void urlRev;
    return store.artifacts.slice();
  });
  const insets = $derived({ top: 64, left: 52 + 28, right: ui.sheetOpen ? 404 + 28 : 14, bottom: 14 });

  const subscribe = (cb: (ev: EventRow) => void) => store.onEvent(cb);
  const onselect = (sel: Sel | null) => {
    if (!sel) ui.select(null);
    else if (sel.agentId) ui.selectAgent(sel.agentId);
    else if (sel.taskId) ui.selectTask(sel.taskId);
    else if (sel.path) ui.selectFile(sel.path);
    else ui.select(null);
  };
  const onhover = (sel: Sel | null) => ui.setHover(sel);
  // Read once per mount (not reactive): persisted events, oldest first.
  const historyAtMount = () => store.events.slice();

  function retry(reset: () => void) {
    loading = null;
    reset();
  }
</script>

<div class="office" aria-label="The office">
  {#await loadOffice() then mod}
    {@const OfficeView = mod.default}
    <svelte:boundary onerror={(e) => console.error('[codly] office crashed', e)}>
      {#key store.epoch}
        <OfficeView
          {files}
          agents={store.agents}
          tasks={store.tasks}
          approvals={store.approvals}
          artifacts={officeArtifacts}
          {selection}
          {hover}
          {theme}
          reducedMotion={ui.reducedMotion}
          {backdrop}
          {onselect}
          {onhover}
          {subscribe}
          history={historyAtMount()}
          narrate={officeNarrate}
          {artifactUrl}
          {insets}
          ticker={ui.mode === 'office'}
        />
      {/key}
      {#snippet failed(_error, reset)}
        <div class="fail">
          <p class="serif">The office went dark.</p>
          <p class="muted">Your agents keep working; everything is still in the panels.</p>
          <button class="btn sm" onclick={() => retry(reset)}>Reopen the office</button>
        </div>
      {/snippet}
    </svelte:boundary>
  {:catch}
    <div class="fail"><p class="serif">The office couldn’t load.</p><p class="muted">Your agents keep working; everything is still in the panels.</p></div>
  {/await}
</div>

<style>
  .office { position: absolute; inset: 0; display: flex; flex-direction: column; }
  .office > :global(*) { flex: 1; min-height: 0; }
  .fail { display: flex; flex-direction: column; align-items: center; justify-content: flex-end; gap: 6px; padding-bottom: 18vh; text-align: center; }
  .fail .serif { font-size: var(--d-sm); }
</style>
