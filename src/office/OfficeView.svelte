<!--
  CONTRACT (owned by the 3D office engine): a cozy low-poly 3D office/studio. Agents are
  characters walking between rooms that mean something (desks, library, server room, meeting
  room, mail room, archive…), driven only by real events. Props mirror the previous CityView
  so the app swaps one import. Optional extras: `files` (shown as the library's file count),
  `narrate` (the app's crew-strip sentence, so labels match), `artifactUrl` (a URL for an
  artifact's bytes: image artifacts appear framed on their author's desk), `ticker` (the small
  event ticker; always mirrored to aria-live), `insets` (app UI at the edges).
-->
<script lang="ts" module>
  export type WorldSelection = { agentId?: string | null; taskId?: string | null; path?: string | null };
  /** The app's one-sentence narration of an agent (same sentence as the crew strip). */
  export type NarrateFn = (agentId: string) => { parts: { t: string; code?: boolean }[]; tone?: string } | null;
</script>

<script lang="ts">
  import { onMount } from 'svelte';
  import type { Agent } from '../lib/gen/Agent';
  import type { Approval } from '../lib/gen/Approval';
  import type { Artifact } from '../lib/gen/Artifact';
  import type { EventRow } from '../lib/gen/EventRow';
  import type { RepoFile } from '../lib/gen/RepoFile';
  import type { Task } from '../lib/gen/Task';
  import type { AccountInfo } from '../lib/gen/AccountInfo';
  import { OfficeEngine, type AgentListItem, type TickerItem } from './engine';
  import SimsHud, { type AgentAction } from './SimsHud.svelte';
  import { headline, live, plumbobOf, SimsTracker, type Busy } from './sims';
  import JobBoard from './JobBoard.svelte';
  import type { JobOffer } from '../lib/gen/JobOffer';
  import type { Placed } from '../lib/gen/Placed';
  import type { Shop } from '../lib/gen/Shop';
  import type { ModelOffer } from '../lib/gen/ModelOffer';
  import type { Lot } from '../lib/gen/Lot';
  import GhostBoard from './GhostBoard.svelte';
  import BuyMode from './BuyMode.svelte';
  import { decorOf, withBought } from './buy';
  import FurnitureMenu from './FurnitureMenu.svelte';
  import DailyEdition from './DailyEdition.svelte';
  import { edition, standupLine, type Edition } from './daily';
  import type { Digest } from '../lib/gen/Digest';
  import type { Office } from '../lib/gen/Office';

  interface Props {
    files?: RepoFile[];
    agents: Agent[];
    tasks: Task[];
    approvals: Approval[];
    artifacts: Artifact[];
    selection: WorldSelection | null;
    hover: WorldSelection | null;
    /** Light the office for this hour (0..24) instead of your real clock. */
    hour?: number | null;
    reducedMotion: boolean;
    /** Backdrop mode: dimmed and non-interactive behind work panels. */
    backdrop: boolean;
    onselect: (sel: WorldSelection | null) => void;
    onhover: (sel: WorldSelection | null) => void;
    subscribe: (cb: (ev: EventRow) => void) => () => void;
    history: EventRow[];
    narrate?: NarrateFn;
    artifactUrl?: (artifact: Artifact) => string | null;
    ticker?: boolean;
    insets?: { top?: number; right?: number; bottom?: number; left?: number };
    /** The Sims layer: accounts (quota, for the Out of quota moodlet), actions from the click menu, Pause. */
    accounts?: AccountInfo[];
    onaction?: (agentId: string, action: AgentAction) => void;
    /** The look editor saved a new look (the agent's `look_seed`). */
    onlook?: (agentId: string, lookSeed: string) => void;
    paused?: boolean;
    onpause?: (paused: boolean) => void;
    /** Pick who gets a job offer's job (the office pins open offers by the door). */
    onpickapplicant?: (offerId: string, index: number) => void;
    /** Agents let go (ghosts by the memory archive), and bringing one back. */
    ghosts?: Agent[];
    onbringback?: (agentId: string) => void;
    /** The repo's lot: its walls, floors and furniture (null: the Studio). */
    lot?: Lot | null;
    /** Build/Buy: the budget and furniture (null: not available). */
    shop?: Shop | null;
    onplace?: (piece: Placed) => void;
    onsell?: (uid: string) => void;
    /** Save the office as a file (`codly/office@1`: the lot and the crew, no code). */
    onexport?: () => void;
    /** The day's edition and standup: the digest since the last one, or null when it is not due. */
    digest?: () => Promise<Digest | null>;
    /** The paper was closed: the edition is read (until then it comes back on each mount). */
    oneditionread?: () => void;
    /** Home and back (sims.ts): tell the core an agent is at home until then (ms), or back (null). */
    onhome?: (agentId: string, until: number | null) => void;
    /** A shift worked and rested from, or a sickness (its career's stamina). */
    onrest?: (agentId: string, sick: boolean) => void;
    /** Tired (energy under 30%) or rested again: the Lead prefers the rested. */
    ontired?: (agentId: string, tired: boolean) => void;
    /** A new Legend's offer (its CLI's biggest context window, if bigger), and taking it. */
    biggerModel?: (agentId: string) => Promise<ModelOffer | null>;
    ongivemodel?: (agentId: string, model: string) => void;
    /** Where energy, coffee and home are kept between sessions (localStorage key), e.g. per repo. */
    simsKey?: string;
    /** Transfer to…: your other offices, and moving an agent to one (it walks out once the core says so). */
    offices?: () => Promise<Office[]>;
    ontransfer?: (agentId: string, office: Office) => void;
  }

  let {
    files,
    agents,
    tasks,
    approvals,
    artifacts,
    selection,
    hover,
    hour = null,
    reducedMotion,
    backdrop,
    onselect,
    onhover,
    subscribe,
    history,
    narrate,
    artifactUrl,
    ticker = true,
    insets,
    accounts = [],
    onaction,
    onlook,
    paused = false,
    onpause,
    onpickapplicant,
    ghosts = [],
    onbringback,
    lot = null,
    shop = null,
    onplace,
    onsell,
    onexport,
    digest,
    oneditionread,
    onhome,
    onrest,
    ontired,
    biggerModel,
    ongivemodel,
    simsKey,
    offices,
    ontransfer,
  }: Props = $props();

  // The morning edition and standup (daily.ts), once the agents have walked in.
  let paper = $state<Edition | null>(null);
  let standup = $state<string[]>([]);
  const nameOf = (id: string) => agents.find((a) => a.id === id)?.name ?? ghosts.find((a) => a.id === id)?.name ?? 'Someone';
  async function morning() {
    const d = await digest?.().catch(() => null);
    if (!d || !engine) return;
    paper = edition(d);
    const period = Date.now() - d.since > 36 * 3_600_000 ? 'Since last time' : 'Yesterday';
    const here = new Set(agents.filter((a) => !a.archived).map((a) => a.id));
    standup = engine.standup(d.reports.filter((r) => here.has(r.agent_id)).map((r) => ({ id: r.agent_id, text: standupLine(r, period) })));
  }

  // Job offers from the event stream: open ones, then a few seconds of "Hired" once closed.
  let offers = $state<JobOffer[]>([]);
  function seeOffer(ev: EventRow) {
    if (ev.kind.type !== 'job_offer') return;
    const o = ev.kind.offer;
    const now = Date.now();
    if (o.hired == null && o.closes_at < now) return; // from history, long gone
    offers = [...offers.filter((x) => x.id !== o.id), o];
    if (o.hired != null) setTimeout(() => (offers = offers.filter((x) => x.id !== o.id)), 3000);
  }

  const sims = new SimsTracker();
  live.sims = sims;
  // Home and back: every couple of seconds the exhausted and the sick go home once their task is
  // done and the rested come back (sims.ts); what changed goes to the core, the tags to the door.
  function restTick() {
    const e = engine;
    if (!e) return;
    const now = Date.now();
    const status = new Map(tasks.map((t) => [t.id, t.status]));
    // 'task': its queued task started (at home, the core does that only when nobody else was free).
    const busy = (id: string): Busy => {
      const task = agents.find((a) => a.id === id)?.task_id;
      if (task && status.get(task) === 'running') return 'task';
      const c = plumbobOf(e.agentState(id));
      return c === 'green' || c === 'amber';
    };
    sims.tick(now, busy, (id) => e.isPresent(id));
    flushRest(now);
  }
  function flushRest(now = Date.now()) {
    const o = sims.drain();
    for (const h of o.home) onhome?.(h.id, h.until);
    for (const r of o.rest) onrest?.(r.id, r.sick);
    for (const t of o.tired) ontired?.(t.id, t.tired);
    const tags = new Map<string, string>();
    for (const a of agents) {
      const h = a.archived ? null : sims.stats(a.id, now, null).rest.home;
      if (h) tags.set(a.id, `Home · back in ${Math.max(1, Math.ceil((h.until - now) / 60_000))} min`);
    }
    engine?.setHome(tags);
  }
  let saves = 0;
  function saveRest() {
    if (!simsKey) return;
    try {
      localStorage.setItem(simsKey, sims.save());
    } catch {}
  }
  /** The click menu's Send home and Call in are the office's own; the rest go to the app. */
  function act(id: string, a: AgentAction) {
    if (a === 'home' || a === 'callin') {
      if (a === 'home') sims.sendHome(id, Date.now());
      else sims.callIn(id, Date.now());
      flushRest();
      saveRest();
    } else onaction?.(id, a);
  }
  let menuAt = $state<{ id: string; x: number; y: number } | null>(null);
  let furniture = $state<import('./engine').FurnitureMenu | null>(null);
  const hudInsets = $derived({ top: insets?.top ?? 0, right: insets?.right ?? 0, bottom: insets?.bottom ?? 0, left: insets?.left ?? 0 });

  let el: HTMLDivElement;
  let engine = $state<OfficeEngine | null>(null);
  let items = $state<TickerItem[]>([]);
  let roster = $state<AgentListItem[]>([]);
  let announce = $state('');
  let urgent = $state('');
  let lastAnnounce = 0;
  let pending: string | null = null;
  let announceTimer: ReturnType<typeof setTimeout> | undefined;

  // aria-live: at most one polite announcement every 5 s; approvals go out immediately.
  function politely(text: string) {
    const now = Date.now();
    if (now - lastAnnounce >= 5000) {
      lastAnnounce = now;
      announce = text;
      return;
    }
    pending = text;
    if (announceTimer) return;
    announceTimer = setTimeout(() => {
      announceTimer = undefined;
      if (pending) {
        lastAnnounce = Date.now();
        announce = pending;
        pending = null;
      }
    }, 5000 - (now - lastAnnounce));
  }

  onMount(() => {
    const e = new OfficeEngine(el, {
      reducedMotion,
      backdrop,
      onselect: (s) => {
        onselect(s);
        // A click on an agent in the office opens its click menu (The Sims' pie menu).
        const p = s?.agentId ? e.screenOf(s.agentId) : null;
        menuAt = p && s?.agentId ? { id: s.agentId, ...p } : null;
      },
      onhover: (s) => onhover(s),
      onticker: (list) => {
        items = list.slice(-4);
        const last = list[list.length - 1];
        if (last) politely(last.text + (last.code ? ` ${last.code}` : ''));
      },
      onagents: (list) => (roster = list),
      onannounce: (text) => (urgent = text),
      onfurniture: (f) => {
        furniture = f;
        if (f) menuAt = null;
      },
      artifactUrl,
      narrate,
      lot,
    });
    el.classList.toggle('of-backdrop', backdrop);
    e.setFiles($state.snapshot(files) as RepoFile[] | undefined);
    e.setAgents($state.snapshot(agents) as Agent[]);
    e.setTasks($state.snapshot(tasks) as Task[]);
    e.setArtifacts($state.snapshot(artifacts) as Artifact[]);
    e.replay(history);
    sims.setAgents($state.snapshot(agents) as Agent[]);
    // Needs drive what an agent does on its break (a tired one naps); what it does refills them.
    e.setNeeds((id) => {
      const s = sims.stats(id, Date.now(), e.agentState(id));
      const h = headline(s);
      return { energy: s.needs.energy, morale: s.needs.morale, mood: h?.emoji ? { emoji: h.emoji, label: `${h.label}: ${h.why}` } : null, jittery: s.rest.jittery };
    });
    e.setActivity((id, now, act) => sims.setActivity(id, now, act));
    if (simsKey)
      try {
        sims.load(localStorage.getItem(simsKey), Date.now());
      } catch {}
    sims.setTasks($state.snapshot(tasks) as Task[]);
    for (const ev of history) {
      sims.ingest(ev);
      seeOffer(ev);
    }
    e.setApprovals($state.snapshot(approvals) as Approval[]);
    e.setSelection(selection);
    e.setHover(hover);
    engine = e;
    flushRest();
    const rest = setInterval(() => {
      restTick();
      if (++saves % 5 === 0) saveRest();
    }, 2000);
    return () => {
      clearInterval(rest);
      saveRest();
      if (live.sims === sims) live.sims = null;
      engine = null;
      clearTimeout(announceTimer);
      e.dispose();
    };
  });

  // The morning runs once per mount, when the office is on screen (not behind a panel).
  let dawned = false;
  $effect(() => {
    if (!engine || backdrop || dawned) return;
    dawned = true;
    const t = setTimeout(morning, reducedMotion ? 0 : 2500);
    return () => clearTimeout(t);
  });

  // Live events (re-subscribe if the source changes; events are de-duplicated by id).
  $effect(() => {
    const e = engine;
    const sub = subscribe;
    if (!e) return;
    return sub((ev) => {
      e.push(ev);
      sims.ingest(ev);
      seeOffer(ev);
    });
  });

  $effect(() => {
    const snap = $state.snapshot(files) as RepoFile[] | undefined;
    engine?.setFiles(snap);
  });
  $effect(() => {
    const snap = $state.snapshot(agents) as Agent[];
    engine?.setAgents(snap);
    sims.setAgents(snap);
  });
  $effect(() => {
    const snap = $state.snapshot(tasks) as Task[];
    engine?.setTasks(snap);
    sims.setTasks(snap);
  });
  $effect(() => {
    const snap = $state.snapshot(approvals) as Approval[];
    engine?.setApprovals(snap);
  });
  $effect(() => {
    const snap = $state.snapshot(artifacts) as Artifact[];
    engine?.setArtifacts(snap);
  });
  $effect(() => {
    const s = selection ? { agentId: selection.agentId ?? null, taskId: selection.taskId ?? null, path: selection.path ?? null } : null;
    engine?.setSelection(s);
  });
  $effect(() => {
    const h = hover ? { agentId: hover.agentId ?? null, taskId: hover.taskId ?? null, path: hover.path ?? null } : null;
    engine?.setHover(h);
  });
  $effect(() => {
    engine?.setHour(hour);
  });
  $effect(() => {
    engine?.setReducedMotion(reducedMotion);
  });
  $effect(() => {
    engine?.setBackdrop(backdrop);
  });
  $effect(() => {
    engine?.setNarrate(narrate);
  });
  $effect(() => {
    const i = insets ? { top: insets.top, right: insets.right, bottom: insets.bottom, left: insets.left } : undefined;
    engine?.setInsets(i);
  });
  $effect(() => {
    engine?.setArtifactUrl(artifactUrl);
  });
  $effect(() => {
    engine?.setGhosts(ghosts.map((g) => ({ id: g.id, color: g.color, seed: g.look_seed || g.id, provider: g.provider })));
  });
  // The foosball, the PS5 and the other built-in kinds you bought stand in the lot (they play
  // there); the rest of what you bought is drawn as models.
  const officeLot = $derived(lot && withBought(lot, shop?.placed ?? []));
  $effect(() => {
    if (officeLot) engine?.setLot(officeLot);
  });
  $effect(() => {
    engine?.setDecor(decorOf(shop?.placed ?? []));
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex (the office takes arrow keys, W/A/S/D, +/−, Q/E, F and Esc) -->
<div
  class="of-office"
  class:of-backdrop={backdrop}
  bind:this={el}
  tabindex={backdrop ? -1 : 0}
  role="application"
  aria-label="The office: agents are characters working in rooms. Arrow keys pan, Q and E rotate, plus and minus zoom, F follows the selected agent, Escape clears the selection."
  aria-describedby="of-roster"
>
  <div class="of-vignette" aria-hidden="true"></div>
  {#if engine && !backdrop}
    <SimsHud
      {engine}
      {sims}
      {agents}
      {accounts}
      selectedId={selection?.agentId ?? null}
      {menuAt}
      insets={hudInsets}
      {narrate}
      onaction={onaction ? act : undefined}
      {onlook}
      onclosemenu={() => (menuAt = null)}
      {paused}
      {onpause}
      {biggerModel}
      {ongivemodel}
      {tasks}
      {offices}
      {ontransfer}
    />
  {/if}
  {#if engine && !backdrop && paper}
    <DailyEdition {paper} {nameOf} {standup} top={hudInsets.top} onclose={() => ((paper = null), oneditionread?.())} />
  {/if}
  {#if engine && offers.length}
    <JobBoard {offers} insets={hudInsets} onpick={onpickapplicant} />
  {/if}
  {#if engine && !backdrop && ghosts.length}
    <GhostBoard {ghosts} {engine} insets={hudInsets} {onbringback} />
  {/if}
  {#if engine && !backdrop && furniture}
    <FurnitureMenu
      {engine}
      menu={furniture}
      {agents}
      onclose={() => (furniture = null)}
      onagent={(id) => {
        const p = engine?.screenOf(id);
        if (p) menuAt = { id, ...p };
      }}
    />
  {/if}
  {#if engine && !backdrop && shop && onplace && onsell}
    <BuyMode {engine} {shop} lot={officeLot} crew={agents.map((a) => a.career.rank)} insets={hudInsets} {onplace} {onsell} {onexport} />
  {/if}
  {#if ticker && !backdrop && items.length}
    <ol class="of-ticker" aria-hidden="true">
      {#each items as t, i (t.id)}
        <li style:opacity={0.45 + (0.55 * (i + 1)) / items.length} data-tone={t.tone}>
          {#if t.color}<span class="of-tick-dot" style:background={t.color}></span>{/if}
          <span>{t.text}</span>{#if t.code}<code>{t.code}</code>{/if}
        </li>
      {/each}
    </ol>
  {/if}
  <div class="of-sr" aria-live="polite" aria-atomic="true">{announce}</div>
  <div class="of-sr" aria-live="assertive" aria-atomic="true">{urgent}</div>
  <ul class="of-sr" id="of-roster" role="list" aria-label="Agents in the office">
    {#each roster as a (a.id)}
      <li role="listitem" aria-current={selection?.agentId === a.id ? 'true' : undefined}>
        <button
          type="button"
          tabindex={backdrop ? -1 : 0}
          aria-pressed={selection?.agentId === a.id}
          onclick={() => onselect({ agentId: a.id })}
          onfocus={() => onhover({ agentId: a.id })}
          onblur={() => onhover(null)}>{a.name} ({a.role}, {a.provider}): {a.sentence}</button
        >
      </li>
    {/each}
  </ul>
</div>

<style>
  .of-office {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background: var(--of-sky, #12131f);
    outline: none;
    contain: strict;
    isolation: isolate;
    user-select: none;
    -webkit-user-select: none;
    --of-font: var(--font-ui, 'Inter Variable', -apple-system, BlinkMacSystemFont, system-ui, sans-serif);
    --of-mono: var(--font-mono, 'JetBrains Mono Variable', ui-monospace, 'SF Mono', Menlo, monospace);
  }
  .of-office:focus-visible {
    box-shadow: inset 0 0 0 2px color-mix(in oklch, var(--focus, #6aa0ff) 70%, transparent);
  }
  .of-office :global(.of-canvas) {
    position: absolute;
    inset: 0;
    display: block;
    touch-action: none;
  }
  .of-vignette {
    position: absolute;
    inset: 0;
    pointer-events: none;
    background: radial-gradient(130% 100% at 50% 45%, transparent 60%, rgba(6, 6, 14, 0.38) 100%);
  }
  .of-office[data-theme='light'] .of-vignette {
    background: radial-gradient(130% 100% at 50% 45%, transparent 64%, rgba(120, 100, 70, 0.12) 100%);
  }
  .of-backdrop :global(.of-canvas),
  .of-backdrop :global(.of-overlay),
  .of-backdrop :global(.of-label) {
    pointer-events: none !important;
  }
  .of-backdrop :global(.of-overlay) {
    opacity: 0.82;
  }

  /* ───── overlay labels (created by overlay.ts) ───── */
  .of-office :global(.of-overlay) {
    position: absolute;
    inset: 0;
    pointer-events: none;
    overflow: hidden;
    font-family: var(--of-font);
    -webkit-font-smoothing: antialiased;
  }
  .of-office :global(.of-label) {
    position: absolute;
    left: 0;
    top: 0;
    white-space: nowrap;
    will-change: transform;
    pointer-events: none;
  }
  .of-office :global(.of-agent) {
    pointer-events: auto;
    cursor: pointer;
    padding: 3px 8px 4px 7px;
    border-radius: 9px;
    background: rgba(24, 20, 30, 0.82);
    color: #f3ede4;
    box-shadow:
      0 0 0 1px rgba(255, 240, 220, 0.08),
      0 6px 18px -8px rgba(0, 0, 0, 0.7);
    transition:
      opacity 160ms ease,
      box-shadow 160ms ease;
  }
  .of-office :global(.of-agent::after) {
    content: '';
    position: absolute;
    left: 50%;
    bottom: -3px;
    width: 7px;
    height: 7px;
    margin-left: -3.5px;
    background: inherit;
    transform: rotate(45deg);
    border-radius: 0 0 2px 0;
  }
  .of-office :global(.of-head) {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 11.5px;
    line-height: 15px;
    font-weight: 650;
    letter-spacing: 0.005em;
  }
  .of-office :global(.of-dot) {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--of-id);
    flex: none;
  }
  .of-office :global(.of-icon) {
    display: inline-flex;
    color: var(--of-id);
    opacity: 0.95;
  }
  .of-office :global(.of-icon[data-icon='approval']),
  .of-office :global(.of-icon[data-icon='question']) {
    color: #ffb547;
  }
  .of-office :global(.of-icon[data-icon='blocked']),
  .of-office :global(.of-icon[data-icon='tests-fail']) {
    color: #ff8a7e;
  }
  .of-office :global(.of-icon[data-icon='tests-pass']),
  .of-office :global(.of-icon[data-icon='done']) {
    color: #6fe39b;
  }
  .of-office :global(.of-mood) {
    font-size: 12px;
    line-height: 13px;
  }
  .of-office :global(.of-badge) {
    font-size: 9.5px;
    font-weight: 700;
    line-height: 13px;
    padding: 0 5px;
    border-radius: 6px;
    letter-spacing: 0.02em;
    color: var(--of-id);
    background: color-mix(in oklch, var(--of-id) 18%, transparent);
  }
  .of-office :global(.of-chip) {
    font-size: 10px;
    font-weight: 550;
    line-height: 13px;
    padding: 0 5px;
    border-radius: 6px;
    white-space: nowrap;
    color: rgba(243, 236, 226, 0.78);
    background: rgba(255, 255, 255, 0.08);
  }
  .of-office[data-theme='light'] :global(.of-chip) {
    color: rgba(50, 40, 34, 0.72);
    background: rgba(60, 40, 20, 0.07);
  }
  .of-office :global(.of-line) {
    margin-top: 1px;
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: 11.5px;
    line-height: 16px;
    font-weight: 450;
    color: rgba(243, 236, 226, 0.8);
  }
  .of-office :global(.of-line code) {
    font-family: var(--of-mono);
    font-size: 10.5px;
    color: #fffaf2;
    background: rgba(255, 255, 255, 0.09);
    padding: 0 4px;
    border-radius: 4px;
  }
  .of-office :global(.of-compact .of-line) {
    display: none;
  }
  .of-office :global(.of-dim) {
    opacity: 0.78;
  }
  .of-office :global(.of-crowded) {
    opacity: 0.7;
  }
  .of-office :global(.of-agent[data-tone='warn']) {
    background: rgba(60, 38, 8, 0.92);
    box-shadow:
      0 0 0 1px rgba(255, 196, 92, 0.65),
      0 0 24px -4px rgba(255, 176, 56, 0.55);
  }
  .of-office :global(.of-agent[data-tone='warn'] .of-line) {
    color: #ffe3ad;
  }
  .of-office :global(.of-agent[data-tone='danger']) {
    box-shadow:
      0 0 0 1px rgba(255, 110, 100, 0.55),
      0 6px 18px -8px rgba(0, 0, 0, 0.7);
  }
  .of-office :global(.of-agent[data-tone='danger'] .of-line) {
    color: #ffb4ac;
  }
  .of-office :global(.of-agent[data-tone='success'] .of-line) {
    color: #a9f0c5;
  }
  .of-office :global(.of-agent[data-tone='muted'] .of-line) {
    color: rgba(225, 215, 200, 0.6);
  }
  .of-office :global(.of-agent.of-selected) {
    box-shadow:
      0 0 0 1.5px var(--of-id),
      0 0 22px -6px var(--of-id),
      0 6px 18px -8px rgba(0, 0, 0, 0.7);
  }
  .of-office :global(.of-agent.of-hovered) {
    box-shadow:
      0 0 0 1px var(--of-id),
      0 6px 18px -8px rgba(0, 0, 0, 0.7);
  }
  /* A Legend: a gold tag. */
  .of-office :global(.of-agent.of-gold) {
    background: linear-gradient(180deg, rgba(74, 54, 14, 0.94), rgba(44, 32, 8, 0.94));
    box-shadow:
      0 0 0 1.5px #e9b03a,
      0 0 18px -4px rgba(255, 201, 74, 0.6),
      0 6px 18px -8px rgba(0, 0, 0, 0.7);
  }
  .of-office :global(.of-agent.of-gold .of-title) {
    color: #ffd27a;
  }
  .of-office :global(.of-room) {
    font-size: 9.5px;
    line-height: 12px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: rgba(255, 236, 210, 0.42);
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.6);
  }
  .of-office :global(.of-tag) {
    padding: 2px 8px;
    border-radius: 7px;
    font-size: 11px;
    line-height: 16px;
    font-weight: 600;
    color: #f3ede4;
    background: rgba(24, 20, 30, 0.8);
  }

  /* daylight */
  .of-office[data-theme='light'] :global(.of-agent) {
    background: rgba(255, 252, 246, 0.94);
    color: #2a2420;
    box-shadow:
      0 0 0 1px rgba(60, 40, 20, 0.09),
      0 6px 18px -10px rgba(80, 56, 24, 0.4);
  }
  .of-office[data-theme='light'] :global(.of-line) {
    color: rgba(50, 40, 34, 0.72);
  }
  .of-office[data-theme='light'] :global(.of-line code) {
    color: #1d1916;
    background: rgba(60, 40, 20, 0.07);
  }
  .of-office[data-theme='light'] :global(.of-icon[data-icon='approval']),
  .of-office[data-theme='light'] :global(.of-icon[data-icon='question']) {
    color: #b86a00;
  }
  .of-office[data-theme='light'] :global(.of-icon[data-icon='blocked']),
  .of-office[data-theme='light'] :global(.of-icon[data-icon='tests-fail']) {
    color: #b8352a;
  }
  .of-office[data-theme='light'] :global(.of-icon[data-icon='tests-pass']),
  .of-office[data-theme='light'] :global(.of-icon[data-icon='done']) {
    color: #1f8a4c;
  }
  .of-office[data-theme='light'] :global(.of-agent[data-tone='warn']) {
    background: rgba(255, 243, 220, 0.97);
    box-shadow:
      0 0 0 1px rgba(214, 140, 20, 0.6),
      0 0 22px -6px rgba(230, 150, 30, 0.45);
  }
  .of-office[data-theme='light'] :global(.of-agent[data-tone='warn'] .of-line) {
    color: #7a4a00;
  }
  .of-office[data-theme='light'] :global(.of-agent[data-tone='danger'] .of-line) {
    color: #a3281c;
  }
  .of-office[data-theme='light'] :global(.of-agent[data-tone='success'] .of-line) {
    color: #1f7a45;
  }
  .of-office[data-theme='light'] :global(.of-agent[data-tone='muted'] .of-line) {
    color: rgba(50, 40, 34, 0.5);
  }
  .of-office[data-theme='light'] :global(.of-room) {
    color: rgba(92, 72, 56, 0.5);
    text-shadow: 0 1px 0 rgba(255, 255, 255, 0.6);
  }
  .of-office[data-theme='light'] :global(.of-tag) {
    background: rgba(255, 252, 246, 0.94);
    color: #2a2420;
  }

  /* ───── ticker ───── */
  .of-ticker {
    position: absolute;
    left: 16px;
    bottom: 14px;
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 3px;
    max-width: min(460px, 60%);
    pointer-events: none;
    font-family: var(--of-font);
    font-size: 11.5px;
    line-height: 16px;
    color: rgba(240, 230, 216, 0.82);
  }
  .of-ticker li {
    display: flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.8);
  }
  .of-ticker code {
    font-family: var(--of-mono);
    font-size: 10.5px;
    padding: 0 4px;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.08);
  }
  .of-tick-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex: none;
  }
  .of-ticker li[data-tone='warn'] {
    color: #ffd58a;
  }
  .of-ticker li[data-tone='danger'] {
    color: #ffb1a8;
  }
  .of-ticker li[data-tone='success'] {
    color: #a9f0c5;
  }
  .of-office[data-theme='light'] .of-ticker {
    color: rgba(50, 40, 34, 0.8);
  }
  .of-office[data-theme='light'] .of-ticker li {
    text-shadow: 0 1px 0 rgba(255, 255, 255, 0.85);
  }
  .of-office[data-theme='light'] .of-ticker code {
    background: rgba(60, 40, 20, 0.06);
  }
  .of-office[data-theme='light'] .of-ticker li[data-tone='warn'] {
    color: #8a5300;
  }
  .of-office[data-theme='light'] .of-ticker li[data-tone='danger'] {
    color: #a3281c;
  }
  .of-office[data-theme='light'] .of-ticker li[data-tone='success'] {
    color: #1f7a45;
  }

  .of-sr {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
    border: 0;
  }
</style>
