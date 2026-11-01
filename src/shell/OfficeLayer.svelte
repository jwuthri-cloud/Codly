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
  import type { Agent } from '../lib/gen/Agent';
  import type { Artifact } from '../lib/gen/Artifact';
  import type { Office } from '../lib/gen/Office';
  import type { Placed } from '../lib/gen/Placed';
  import type { Rank } from '../lib/gen/Rank';
  import { atLeast, PERKS, rankLabel } from '../lib/util/career';
  import { editionDue } from '../office/daily';
  import type { Shop } from '../lib/gen/Shop';
  import type { Lot } from '../lib/gen/Lot';
  import type { EventRow } from '../lib/gen/EventRow';
  import { acquireArtifactUrl, releaseArtifactUrl } from '../lib/state/artifacts';
  import { giveModel, interruptAgent, setAgentLook, stopAgent } from '../lib/state/actions';
  import { accounts } from '../lib/state/accounts.svelte';
  import { narrate } from '../lib/state/narration';
  import type { RepoFile } from '../lib/gen/RepoFile';
  import { offices } from '../lib/state/offices.svelte';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { toast } from '../lib/state/toasts.svelte';


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

  // The repo's lot (its walls, floors and furniture; created on the first read: a Studio for a
  // repo with history, a Garage for a new one). The office waits for it, so it never shows the
  // wrong building first.
  let lot = $state.raw<Lot | null>(null);
  let lotFor = $state<string | null>(null);
  // Read again on the way back from the map, where offices move between plots.
  const onMap = $derived(ui.mode === 'map');
  $effect(() => {
    const ws = store.workspace?.id ?? null;
    void store.epoch;
    void onMap;
    if (!ws) return void ((lot = null), (lotFor = null));
    api.officeLot().then(
      (l) => store.workspace?.id === ws && ((lot = l), (lotFor = ws)),
      () => store.workspace?.id === ws && (lotFor = ws),
    );
  });
  const lotReady = $derived(!store.workspace || lotFor === store.workspace.id);

  // Ghosts (agents let go) and the Build/Buy wallet: both move with agent changes (XP, let go),
  // and with the developer panel's cheats.
  let ghosts = $state.raw<Agent[]>([]);
  let shop = $state.raw<Shop | null>(null);
  function refetchOffice() {
    api.ghosts().then((g) => (ghosts = g)).catch(() => {});
    api.officeShop().then((s) => (shop = s)).catch(() => {});
  }
  $effect(() => {
    void store.workspace?.id;
    void store.epoch;
    if (store.workspace) refetchOffice();
    else (ghosts = []), (shop = null);
  });
  $effect(() => {
    let t: ReturnType<typeof setTimeout> | undefined;
    const off = store.onEvent((ev) => {
      if (ev.kind.type !== 'agent_changed' && !(ev.kind.type === 'decision' && ev.kind.decision_type === 'developer')) return;
      clearTimeout(t);
      t = setTimeout(refetchOffice, 1500);
    });
    return () => {
      off();
      clearTimeout(t);
    };
  });
  const fail = (title: string) => (e: unknown) => toast({ kind: 'error', title, body: String(e) });
  function bringBack(id: string) {
    api.bringBack(id).then((a) => {
      toast({ kind: 'success', title: `${a.name} is back`, body: 'A regular worker again, with the same career and memory.' });
      refetchOffice();
    }, fail('Could not bring them back'));
  }
  // On a failure, the office goes back to the furniture the core has (it shows a drop at once).
  const undo = (title: string) => (e: unknown) => {
    shop = shop && { ...shop };
    fail(title)(e);
  };
  // The lot too: what came with the office (desks, the server) is drawn from it.
  const relot = () => api.officeLot().then((l) => (lot = l), () => {});
  const place = (p: Placed) => api.officePlace(p).then((s) => ((shop = s), relot()), undo('Could not put it there'));
  const sell = (uid: string) => api.officeSell(uid).then((s) => ((shop = s), relot()), undo('Could not sell it'));
  // Build mode: the core checks the lot and charges the wallet for the change (a refusal throws:
  // build mode puts the office back).
  const saveLot = async (l: Lot) => {
    const saved = await api.officeSaveLot(l);
    lot = saved.lot;
    if (shop) shop = { ...shop, wallet: saved.wallet };
    return saved;
  };

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
  const insets = $derived({ top: 64, left: ui.railW + 28, right: ui.sheetOpen ? 404 + 28 : 14, bottom: 14 });

  const subscribe = (cb: (ev: EventRow) => void) => store.onEvent(cb);
  const onselect = (sel: Sel | null) => {
    if (!sel) ui.select(null);
    else if (sel.agentId) ui.selectAgent(sel.agentId);
    else if (sel.taskId) ui.selectTask(sel.taskId);
    else if (sel.path) ui.selectFile(sel.path);
    else ui.select(null);
  };
  const onhover = (sel: Sel | null) => ui.setHover(sel);
  // The office's click menu (The Sims' pie menu): each item is a real command.
  const onaction = (id: string, action: 'chat' | 'task' | 'model' | 'stop' | 'interrupt' | 'details' | 'export' | 'trust' | 'untrust' | 'home' | 'callin') => {
    const a = store.agentMap.get(id);
    if (!a) return;
    if (action === 'chat') ui.openThread(`agent:${id}`);
    else if (action === 'task') {
      if (a.task_id) ui.selectTask(a.task_id);
    } else if (action === 'details') ui.selectAgent(id);
    else if (action === 'model') {
      // 1:1 agents switch in place; the Lead's model lives in Settings; workers follow the plan.
      if (a.role === 'solo') ui.modelDialog = { provider: a.provider, agentId: id };
      else ui.settingsOpen = true;
    } else if (action === 'interrupt') interruptAgent(id);
    else if (action === 'stop') stopAgent(id);
    else if (action === 'export') exportAgent(id, a.name);
    else if (action === 'trust' || action === 'untrust') trustAgent(id, a.name, action === 'trust' ? a.career.rank : 'intern');
  };
  /** Grant (up to its rank) or take back the perks a rank earns (career::PERKS). */
  async function trustAgent(id: string, name: string, rank: Rank) {
    try {
      const a = await api.trustAgent(id, rank);
      const perks = PERKS.filter((p) => atLeast(a.career.trust, p.rank)).map((p) => p.perk.toLowerCase());
      toast(perks.length ? { kind: 'success', title: `You trust ${name} as ${rankLabel(a.career.trust)}`, body: `${perks.join('; ')}.` } : { kind: 'info', title: `${name} asks you again`, body: 'Your approval settings apply to it like everyone else.' });
    } catch (e) {
      toast({ kind: 'error', title: 'Could not change trust', body: String(e) });
    }
  }
  /** Move an agent to another of your offices: once the core has it, it walks out of this one. */
  async function transferAgent(id: string, office: Office) {
    const name = store.agentMap.get(id)?.name ?? 'It';
    try {
      await api.transferAgent(id, office.id);
      if (ui.selectedAgentId === id) ui.select(null);
      toast({ kind: 'success', title: `${name} moved to ${office.name}`, body: 'Its career, traits and memory went along; trust and friendships stay here.' });
    } catch (e) {
      toast({ kind: 'error', title: `${name} can't move yet`, body: String(e) });
    }
  }
  /** Save an agent (career, traits, memory) as a file to import into another repo. */
  async function exportAgent(id: string, name: string) {
    try {
      const json = await api.exportAgent(id);
      const url = URL.createObjectURL(new Blob([json], { type: 'application/json' }));
      const a = Object.assign(document.createElement('a'), { href: url, download: `${name.toLowerCase().replace(/[^a-z0-9]+/g, '-')}.codly-agent.json` });
      a.click();
      URL.revokeObjectURL(url);
      toast({ kind: 'success', title: `${name} exported`, body: `Import ${a.download} from the add-agent dialog in any repo.` });
    } catch (e) {
      toast({ kind: 'error', title: 'Export failed', body: String(e) });
    }
  }
  /** Save the office (its lot, and its crew's names, looks, ranks and public stats; no code,
   *  paths or prompts) as a `codly/office@1` file. */
  async function exportOffice() {
    try {
      const json = await api.exportOffice();
      const name = (store.workspace?.name ?? 'office').toLowerCase().replace(/[^a-z0-9]+/g, '-');
      const url = URL.createObjectURL(new Blob([json], { type: 'application/json' }));
      const a = Object.assign(document.createElement('a'), { href: url, download: `${name}.codly-office.json` });
      a.click();
      URL.revokeObjectURL(url);
      toast({ kind: 'success', title: 'Office exported', body: `${a.download}: the lot and the crew, without code, paths or prompts.` });
    } catch (e) {
      toast({ kind: 'error', title: 'Export failed', body: String(e) });
    }
  }
  /** The morning edition, once a day per repo: read once you close it (kept in this browser). */
  const editionKey = () => `codly.edition.${store.workspace?.id ?? 'demo'}`;
  async function digest() {
    let last: number | null = null;
    try {
      last = Number(localStorage.getItem(editionKey())) || null;
    } catch {}
    const due = editionDue(last, Date.now());
    return due ? api.officeDigest(due.since) : null;
  }
  // Pause is this office's: the others keep working.
  const onpause = (paused: boolean) => store.workspace && offices.pause(store.workspace.id, paused);
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
    {#if lotReady}
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
            accounts={accounts.list}
            {onaction}
            onlook={(id, seed) => {
              const a = store.agentMap.get(id);
              if (a) setAgentLook(a, seed);
            }}
            paused={!!store.workspace && offices.paused(store.workspace.id)}
            {onpause}
            onpickapplicant={(offerId, i) => api.pickApplicant(offerId, i).catch(fail('Could not pick'))}
            {ghosts}
            onbringback={bringBack}
            {lot}
            {shop}
            onplace={place}
            onsell={sell}
            onexport={exportOffice}
            onsavelot={saveLot}
            {digest}
            simsKey="codly.sims.{store.workspace?.id ?? 'demo'}"
            onhome={(id, until) => api.setAgentHome(id, until).catch(() => {})}
            onrest={(id, sick) => api.noteRest(id, sick).catch(() => {})}
            ontired={(id, tired) => api.setAgentTired(id, tired).catch(() => {})}
            onhangout={(a, b) => api.noteHangout(a, b).catch(() => {})}
            biggerModel={(id) => api.biggerModel(id)}
            ongivemodel={(id, model) => {
              const a = store.agentMap.get(id);
              if (a) giveModel(a, model);
            }}
            offices={() => api.offices()}
            ontransfer={transferAgent}
            oneditionread={() => {
              try {
                localStorage.setItem(editionKey(), String(Date.now()));
              } catch {}
            }}
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
    {/if}
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
