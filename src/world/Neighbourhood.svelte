<!--
  The neighbourhood (PROGRESSION §4, "The map"): every lot of yours and the ones for sale, on flat
  ground. Each repo's office stands on its lot, seen from outside with a sign (its name and crew);
  the open repo is live and lit, and so is any other whose agents are on tasks (every office with
  work keeps working); idle ones say "Closed". Click a lot: enter its office, buy
  it (empty or ready-made), move an office onto it, or sell it. The wallet sits in the corner.
  The scene is scene.ts; what a lot offers and what it costs is map.ts (pure, tested).
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import type { Lot } from '../lib/gen/Lot';
  import type { LotSlot } from '../lib/gen/LotSlot';
  import type { Office } from '../lib/gen/Office';
  import type { Wallet } from '../lib/gen/Wallet';
  import { openWorkspace } from '../lib/state/actions';
  import { offices as tabs } from '../lib/state/offices.svelte';
  import { store } from '../lib/state/store.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { coins } from '../lib/util/format';
  import { buyOptions, signOf, sizeLabel, slotActions, slotKind, type ActionId } from './map';
  import type { Away } from './home';
  import { NeighbourhoodScene } from './scene';

  let el: HTMLDivElement | undefined = $state();
  let scene: NeighbourhoodScene | null = null;

  let slots = $state.raw<LotSlot[]>([]);
  let lots = $state.raw(new Map<string, Lot>());
  let wallet = $state.raw<Wallet | null>(null);
  let offices = $state.raw<Office[]>([]);
  let loaded = $state(false);
  let busy = $state(false);

  /** The lot whose panel is open, where it opened, and which step of it shows. */
  let selected = $state<string | null>(null);
  let anchor = $state({ x: 0, y: 0 });
  let step = $state<'main' | 'move-here' | 'move-away' | 'sell'>('main');

  const ws = $derived(store.workspace);
  const crewHere = $derived(store.agents.filter((a) => a.role !== 'lead' && !a.archived).length);
  const working = $derived(store.agents.filter((a) => a.role !== 'lead' && (a.status === 'working' || a.status === 'thinking')).length);
  /** Who is at home, by office: the open one's from its agents, the others' from the core. The
   *  scene keeps the ones whose stay is still ahead. */
  const home = $derived.by(() => {
    const m = new Map<string, Away[]>();
    for (const o of offices) if (o.id !== ws?.id && o.home.length) m.set(o.id, o.home.map((a) => ({ id: a.id, name: a.name, color: a.color, since: a.since, until: a.until })));
    const here = store.agents.filter((a) => a.role !== 'lead' && !a.archived && a.away_until != null);
    if (ws && here.length) m.set(ws.id, here.map((a) => ({ id: a.id, name: a.name, color: a.color, since: a.away_since ?? null, until: a.away_until! })));
    return m;
  });
  /** Every office with a tab is open (it works whenever it has work), unless paused. */
  const paused = $derived(new Set([...repoIds()].filter((id) => tabs.paused(id))));
  const open = $derived(new Set(tabs.tabs.map((t) => t.id).filter((id) => !paused.has(id))));
  function repoIds() {
    return new Set([...(ws ? [ws.id] : []), ...offices.map((o) => o.id)]);
  }
  const slot = $derived(slots.find((s) => s.id === selected) ?? null);
  const balance = $derived(wallet?.balance ?? 0);

  /** Your repos: the open one and the others (api.offices leaves the open one out). */
  const repos = $derived<{ id: string; name: string; path: string | null }[]>([
    ...(ws ? [{ id: ws.id, name: ws.name, path: ws.repo_path }] : []),
    ...offices.filter((o) => o.id !== ws?.id).map((o) => ({ id: o.id, name: o.name, path: o.repo_path })),
  ]);
  const freeLots = $derived(slots.filter((s) => s.owned && !s.workspace_id && s.id !== selected));
  const movable = $derived(repos.filter((r) => r.id !== slot?.workspace_id));
  const repoLot = (id: string) => slots.find((s) => s.workspace_id === id);

  const fail = (title: string) => (e: unknown) => toast({ kind: 'error', title, body: String(e instanceof Error ? e.message : e) });

  async function load() {
    const [o, w] = await Promise.all([api.offices().catch(() => [] as Office[]), api.wallet().catch(() => null)]);
    offices = o;
    wallet = w;
    await setSlots(await api.lots());
    loaded = true;
  }

  /** New slots, then the buildings on the owned ones (drawn from their lots). */
  async function setSlots(next: LotSlot[]) {
    const built = next.filter((s) => s.owned && (s.template || s.workspace_id));
    const pairs = await Promise.all(built.map(async (s) => [s.id, await api.lotOf(s.id).catch(() => null)] as const));
    lots = new Map(pairs.filter((p): p is readonly [string, Lot] => !!p[1]));
    slots = [...next];
  }

  onMount(() => {
    if (!el) return;
    scene = new NeighbourhoodScene(el, {
      onpick: (id, at) => {
        selected = id;
        step = 'main';
        anchor = at;
      },
    });
    // Dev-only handle for browser tests (`__nb.setHour(13)`), like `__codly`.
    if (import.meta.env.DEV) (window as unknown as { __nb: unknown }).__nb = scene;
    load().catch(fail('Could not load the neighbourhood'));
    el.focus({ preventScroll: true });
    // The other offices keep working: their lights follow.
    const poll = setInterval(() => api.offices().then((o) => (offices = o), () => {}), 10_000);
    return () => {
      clearInterval(poll);
      scene?.dispose();
      scene = null;
    };
  });

  // The rail and the header sit over the map: frame the lots in the rest.
  $effect(() => {
    scene?.setInsets({ top: 64, right: 0, bottom: 0, left: ui.railW + 20 });
  });

  $effect(() => {
    scene?.set({ slots, lots, live: ws?.id ?? null, crew: crewHere, working, away: new Map(offices.map((o) => [o.id, o.working])), open, paused, selected, home });
  });

  // ───────────── actions ─────────────

  async function run(what: string, f: () => Promise<void>) {
    if (busy) return;
    busy = true;
    try {
      await f();
    } catch (e) {
      fail(`Could not ${what}`)(e);
    } finally {
      busy = false;
    }
  }

  const close = () => ((selected = null), (step = 'main'));

  function buy(template: string | null, label: string, price: number) {
    const s = slot;
    if (!s) return;
    void run('buy it', async () => {
      wallet = await api.buyLot(s.id, template);
      await setSlots(await api.lots());
      toast({ kind: 'success', title: template ? `A ${label} is yours` : 'The lot is yours', body: `${coins(price)} spent. ${template ? 'Move an office in, or build on it.' : 'Build on it, or move an office here.'}` });
      step = 'main';
    });
  }

  function sell() {
    const s = slot;
    if (!s) return;
    void run('sell it', async () => {
      wallet = await api.sellLot(s.id);
      await setSlots(await api.lots());
      toast({ kind: 'success', title: 'Sold', body: `${coins(s.sells_for)} back in the wallet.` });
      close();
    });
  }

  function move(workspaceId: string, to: LotSlot) {
    void run('move the office', async () => {
      await setSlots(await api.moveOffice(workspaceId, to.id));
      const name = repos.find((r) => r.id === workspaceId)?.name ?? 'The office';
      toast({ kind: 'success', title: `${name} moved`, body: 'The crew carried the boxes over.' });
      selected = to.id;
      step = 'main';
    });
  }

  /** Enter a lot's office: open its repo if it isn't the open one, then the office view. */
  function enter(s: LotSlot) {
    void run('open that office', async () => {
      if (s.workspace_id !== ws?.id) {
        const path = repos.find((r) => r.id === s.workspace_id)?.path;
        if (!path) throw new Error('that repo is not among your recent ones');
        await openWorkspace(path);
      }
      ui.selection = null;
      ui.room = 'home';
      ui.mode = 'office';
    });
  }

  function act(id: ActionId) {
    const s = slot;
    if (!s) return;
    if (id === 'enter') enter(s);
    else step = id;
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === 'Escape' && selected && !ui.paletteOpen && !ui.settingsOpen && !ui.shortcutsOpen) {
      e.preventDefault();
      e.stopPropagation();
      if (step !== 'main') step = 'main';
      else close();
    }
  }

  // The panel opens beside the lot's sign, kept on screen.
  const PANEL_W = 290;
  const panelPos = $derived.by(() => {
    const W = el?.clientWidth ?? 1200;
    const H = el?.clientHeight ?? 800;
    const at = (selected && scene?.screenOf(selected)) || anchor;
    const x = Math.min(Math.max(16, at.x + 24), W - PANEL_W - 16);
    const y = Math.min(Math.max(70, at.y - 40), H - 300);
    return { x, y };
  });
</script>

<!-- Esc closes the lot's panel before it leaves the map (the app's keys listen on window). -->
<svelte:window onkeydowncapture={onkey} />

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div class="nb" bind:this={el} tabindex="0" role="application" aria-label="Neighbourhood: your lots and the ones for sale">
  <header class="nb-head">
    <div class="where glass">
      <span class="name serif">Neighbourhood</span>
      <span class="hint">{slots.filter((s) => s.owned).length} lots of yours · drag to pan, scroll to zoom, Q/E to turn</span>
    </div>
    <div class="wallet glass" data-tip="Your wallet: every office earns into it">
      {#if wallet}{coins(wallet.balance)}{:else}🪙 …{/if}
    </div>
  </header>

  {#if loaded && !slots.length}
    <p class="nb-empty glass">No lots on the map yet.</p>
  {/if}

  {#if slot}
    {@const live = slot.workspace_id === ws?.id}
    {@const p = paused.has(slot.workspace_id ?? '')}
    {@const busy = live ? working : (offices.find((o) => o.id === slot.workspace_id)?.working ?? 0)}
    {@const sign = signOf(live ? { ...slot, crew: crewHere } : slot, (live || open.has(slot.workspace_id ?? '')) && !p, busy, p)}
    {@const kind = slotKind(slot)}
    <div class="panel glass" role="dialog" aria-label={sign.title} style:left="{panelPos.x}px" style:top="{panelPos.y}px" style:width="{PANEL_W}px">
      <div class="p-head">
        <div class="p-title">{sign.title}</div>
        <button class="icon-btn sm" aria-label="Close" onclick={close}>×</button>
      </div>
      <div class="p-sub">{kind === 'sale' ? sizeLabel(slot.size) : (sign.sub ?? sizeLabel(slot.size))}</div>

      {#if kind === 'sale'}
        <ul class="opts">
          {#each buyOptions(slot, balance) as o (o.template ?? '')}
            <li>
              <button class="opt" disabled={o.short || busy} onclick={() => buy(o.template, o.label, o.price)} data-tip={o.short ? `You have ${coins(balance)}` : undefined}>
                <span class="o-label">{o.template ? `With a ${o.label}` : 'Buy it empty'}</span>
                <span class="o-price" class:short={o.short}>{coins(o.price)}</span>
              </button>
            </li>
          {/each}
        </ul>
        <p class="p-note">You have {coins(balance)}. A ready-made building is the lot plus everything in it.</p>
      {:else if step === 'main'}
        <ul class="opts">
          {#each slotActions(slot, { movable: movable.length, freeLots: freeLots.length }) as a (a.id)}
            <li>
              <button class="opt" class:primary={a.id === 'enter'} disabled={!!a.disabled || busy} data-tip={a.disabled ?? undefined} onclick={() => act(a.id)}>
                <span class="o-label">{a.label}</span>
                {#if a.disabled}<span class="o-note">{a.disabled}</span>{:else if a.note}<span class="o-note">{a.note}</span>{/if}
              </button>
            </li>
          {/each}
        </ul>
      {:else if step === 'move-here'}
        <p class="p-note">Which office moves here? Its crew carries the boxes over.</p>
        <ul class="opts">
          {#each movable as r (r.id)}
            {@const from = repoLot(r.id)}
            <li>
              <button class="opt" disabled={busy} onclick={() => slot && move(r.id, slot)}>
                <span class="o-label">{r.name}</span>
                <span class="o-note">{from ? `now on a ${sizeLabel(from.size).split(' · ')[0].toLowerCase()} lot` : 'no lot yet'}</span>
              </button>
            </li>
          {/each}
        </ul>
        <button class="btn ghost sm back" onclick={() => (step = 'main')}>Back</button>
      {:else if step === 'move-away'}
        <p class="p-note">Move {slot.name} to which of your lots?</p>
        <ul class="opts">
          {#each freeLots as f (f.id)}
            <li>
              <button class="opt" disabled={busy} onclick={() => slot?.workspace_id && move(slot.workspace_id, f)}>
                <span class="o-label">{signOf(f).title.split(' · ')[0]}</span>
                <span class="o-note">{sizeLabel(f.size)}</span>
              </button>
            </li>
          {/each}
        </ul>
        <button class="btn ghost sm back" onclick={() => (step = 'main')}>Back</button>
      {:else if step === 'sell'}
        <p class="p-note">Sell this lot{slot.template ? ' and its building' : ''} for {coins(slot.sells_for)}?{slot.gift ? ' It came as a gift, so it sells for nothing.' : ''}</p>
        <div class="confirm">
          <button class="btn ghost sm" onclick={() => (step = 'main')}>Keep it</button>
          <button class="btn danger sm" disabled={busy} onclick={sell}>Sell for {coins(slot.sells_for)}</button>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .nb { position: absolute; inset: 0; overflow: hidden; outline: none; contain: strict; isolation: isolate; user-select: none; -webkit-user-select: none; background: var(--night); color: var(--ink-1); }
  /* scene.ts sets [data-theme] for the hour (as the office does), so what floats over the map
     reads as day or night with it */
  .nb :global(.nb-canvas) { position: absolute; inset: 0; display: block; touch-action: none; }

  .nb-head { position: absolute; z-index: 3; top: 0; left: calc(var(--rail-w) + var(--gutter) * 2); right: var(--gutter); height: var(--top-h); display: flex; align-items: center; justify-content: space-between; gap: 12px; pointer-events: none; }
  .nb-head > * { pointer-events: auto; }
  .where { display: flex; align-items: baseline; gap: 12px; min-width: 0; height: 34px; padding: 0 16px; border-radius: var(--r-full); }
  .name { font-size: 22px; line-height: 34px; font-style: italic; color: var(--ink-1); white-space: nowrap; }
  .hint { font-size: var(--t-sm); color: var(--ink-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .wallet { height: 34px; padding: 0 14px; border-radius: var(--r-full); display: inline-flex; align-items: center; font-size: var(--t-md); font-weight: var(--w-semi); color: var(--ink-1); font-variant-numeric: tabular-nums; }
  .nb-empty { position: absolute; z-index: 3; left: 50%; top: 45%; transform: translateX(-50%); padding: 12px 18px; border-radius: var(--r-xl); color: var(--ink-2); }

  /* signs (created by scene.ts) */
  .nb :global(.nb-signs) { position: absolute; inset: 0; z-index: 2; pointer-events: none; overflow: hidden; font-family: var(--font-ui); -webkit-font-smoothing: antialiased; }
  .nb :global(.nb-sign) { position: absolute; left: 0; top: 0; display: flex; flex-direction: column; align-items: center; gap: 1px; padding: 4px 10px 5px; border-radius: 9px; white-space: nowrap; will-change: transform; pointer-events: auto; cursor: pointer; background: rgba(24, 20, 30, 0.82); color: #f3ede4; box-shadow: 0 0 0 1px rgba(255, 240, 220, 0.08), 0 6px 18px -8px rgba(0, 0, 0, 0.7); transition: box-shadow 160ms ease, opacity 160ms ease; }
  .nb :global(.nb-sign::after) { content: ''; position: absolute; left: 50%; bottom: -3px; width: 7px; height: 7px; margin-left: -3.5px; background: inherit; transform: rotate(45deg); border-radius: 0 0 2px 0; }
  .nb :global(.nb-sign.hot) { box-shadow: 0 0 0 1.5px #ffb547, 0 6px 18px -8px rgba(0, 0, 0, 0.7); }
  .nb :global(.nb-title) { font-size: 12px; line-height: 16px; font-weight: 650; }
  .nb :global(.nb-sub) { display: inline-flex; align-items: center; gap: 5px; font-size: 10.5px; line-height: 13px; color: rgba(243, 237, 228, 0.62); }
  .nb :global(.nb-dot) { width: 6px; height: 6px; border-radius: 50%; background: rgba(243, 237, 228, 0.35); }
  .nb :global(.nb-sign[data-state='open'] .nb-dot) { background: #6fe39b; box-shadow: 0 0 0 3px rgba(111, 227, 155, 0.18); }
  .nb :global(.nb-sign.busy .nb-dot) { animation: nb-pulse 1.6s ease-in-out infinite; }
  /* an agent at home: a tag over its house, and a smaller one over it while it walks */
  .nb :global(.nb-sign.nb-home), .nb :global(.nb-sign.nb-walker) { pointer-events: none; cursor: default; }
  .nb :global(.nb-sign.nb-home .nb-title) { font-size: 11px; line-height: 15px; font-weight: 600; }
  .nb :global(.nb-sign.nb-walker) { padding: 2px 7px 3px; border-radius: 7px; font-size: 10.5px; line-height: 13px; font-weight: 600; }
  .nb :global(.nb-sign[data-kind='sale']) { background: rgba(24, 20, 30, 0.6); }
  .nb :global(.nb-sign[data-kind='sale'] .nb-title), .nb :global(.nb-sign[data-kind='empty'] .nb-title) { font-weight: 600; }
  :global(.nb[data-theme='light'] .nb-sign) { background: rgba(255, 252, 246, 0.94); color: #2a2420; box-shadow: 0 0 0 1px rgba(60, 40, 20, 0.09), 0 6px 18px -10px rgba(80, 56, 24, 0.4); }
  :global(.nb[data-theme='light'] .nb-sign.hot) { box-shadow: 0 0 0 1.5px #d68c14, 0 6px 18px -10px rgba(80, 56, 24, 0.4); }
  :global(.nb[data-theme='light'] .nb-sub) { color: rgba(50, 40, 34, 0.6); }
  :global(.nb[data-theme='light'] .nb-dot) { background: rgba(50, 40, 34, 0.3); }
  :global(.nb[data-theme='light'] .nb-sign[data-state='open'] .nb-dot) { background: #1f9a55; box-shadow: 0 0 0 3px rgba(31, 154, 85, 0.16); }
  :global(.nb[data-theme='light'] .nb-sign[data-kind='sale']) { background: rgba(255, 252, 246, 0.78); }
  @keyframes nb-pulse { 50% { opacity: 0.35; } }

  /* the lot's panel */
  .panel { position: absolute; z-index: 4; padding: 12px; border-radius: var(--r-xl); background: var(--glass-strong); animation: p-in var(--d-3, 180ms) var(--spring); }
  @keyframes p-in { from { opacity: 0; transform: translateY(6px) scale(0.98); } to { opacity: 1; transform: none; } }
  .p-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .p-title { font-size: var(--t-md); font-weight: var(--w-semi); color: var(--ink-1); }
  .p-sub { margin: 2px 0 10px; font-size: var(--t-sm); color: var(--ink-3); }
  .p-note { margin: 8px 2px 0; font-size: var(--t-sm); line-height: 1.4; color: var(--ink-3); }
  .opts { display: flex; flex-direction: column; gap: 4px; list-style: none; margin: 0; padding: 0; }
  .opt { width: 100%; display: flex; align-items: center; justify-content: space-between; gap: 10px; min-height: 36px; padding: 6px 10px; border-radius: 10px; background: var(--surface-2); text-align: left; transition: background-color var(--d-2) var(--ease-out); }
  .opt:hover { background: var(--surface-3); }
  .opt.primary { background: var(--aurora); color: var(--on-aurora); }
  .opt.primary .o-note { color: inherit; opacity: 0.75; }
  .opt:disabled { opacity: 1; pointer-events: auto; cursor: not-allowed; background: transparent; box-shadow: inset 0 0 0 1px var(--line); }
  .opt:disabled .o-label { color: var(--ink-3); }
  .opt:disabled:hover { background: transparent; }
  .o-label { font-size: var(--t-md); font-weight: var(--w-med); color: inherit; }
  .o-price { font-size: var(--t-md); font-weight: var(--w-semi); font-variant-numeric: tabular-nums; white-space: nowrap; }
  .o-price.short { color: var(--bad); }
  .o-note { font-size: var(--t-xs); color: var(--ink-3); text-align: right; }
  .back { margin-top: 8px; }
  .confirm { display: flex; justify-content: flex-end; gap: 6px; margin-top: 10px; }
</style>
