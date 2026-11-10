<!--
  The Sims layer over the office (sims.ts for the numbers, all from real data):
  - top-left: the selected agent's action queue (what it does now, ✕ to interrupt; its to-dos queued)
  - bottom-left: its panel: portrait, rank, mood, and tabs for Needs / Mood / Skills / Friends / Career,
    and Look (Create-a-Sim: hairstyle, hair and skin color, outfit, glasses; saved in its look_seed)
  - top-center: a promotion (its real track record), with the offer of trust (Junior, Senior and Staff
    perks) and of a stronger model; a new Legend is offered its CLI's biggest context window
    (only when bigger than its own: the core says which, `bigger_model`)
  - around a clicked agent: the click menu (chat, task, model, send home or call in, stop, details,
    and Transfer to… another of your offices: never mid-task, never the Lead)
  - in its Needs tab: Send home / Call in (sims.ts: home refills its energy; the Lead never goes)
  - bottom-right: the estimated cost so far, Pause new work, walls up/down
  Shown only in the full office (not as the backdrop), inside the app's insets.
-->
<script lang="ts" module>
  export type AgentAction = 'chat' | 'task' | 'model' | 'stop' | 'interrupt' | 'details' | 'export' | 'trust' | 'untrust' | 'home' | 'callin';
</script>

<script lang="ts">
  import type { AccountInfo } from '../lib/gen/AccountInfo';
  import type { Agent } from '../lib/gen/Agent';
  import type { ModelOffer } from '../lib/gen/ModelOffer';
  import type { Office } from '../lib/gen/Office';
  import type { Task } from '../lib/gen/Task';
  import { transferBlock } from '../lib/util/transfer';
  import type { OfficeEngine } from './engine';
  import type { NarrateFn } from './OfficeView.svelte';
  import { agentPortrait } from './portrait';
  import { LOOK_STYLES, lookFor, lookPicks, withPick, type LookPick } from './gfx/character';
  import { HAIR, lin, SKIN } from './palette';
  import Scrapbook from './Scrapbook.svelte';
  import { sound } from './sound';
  import { bondLabel, friendScore, levelOf, type SimSheet, type SimsTracker } from './sims';
  import { TRAITS, traitsOf } from './traits';
  import { atLeast, contextLabel, MENTORED_XP, PERKS, rankLabel, rankProgress, titleOf, TRAITS as WORK_TRAITS, WANT_XP, wantProgress } from '../lib/util/career';

  interface Props {
    engine: OfficeEngine;
    sims: SimsTracker;
    agents: Agent[];
    accounts: AccountInfo[];
    selectedId: string | null;
    /** Screen point of a click on an agent (opens the click menu), or null. */
    menuAt: { id: string; x: number; y: number } | null;
    insets: { top: number; right: number; bottom: number; left: number };
    narrate?: NarrateFn;
    onaction?: (agentId: string, action: AgentAction) => void;
    onlook?: (agentId: string, lookSeed: string) => void;
    onclosemenu: () => void;
    paused?: boolean;
    onpause?: (paused: boolean) => void;
    /** A new Legend's offer: its CLI's model with the biggest context window, if bigger than its own. */
    biggerModel?: (agentId: string) => Promise<ModelOffer | null>;
    /** You took the offer: give it that model. */
    ongivemodel?: (agentId: string, model: string) => void;
    /** The board (an agent on a task can't transfer). */
    tasks?: Task[];
    /** Your other offices, for Transfer to…, and the move itself. */
    offices?: () => Promise<Office[]>;
    ontransfer?: (agentId: string, office: Office) => void;
  }
  let { engine, sims, agents, accounts, selectedId, menuAt, insets, narrate, onaction, onlook, onclosemenu, paused = false, onpause, biggerModel, ongivemodel, tasks = [], offices, ontransfer }: Props = $props();

  let tab = $state<'needs' | 'mood' | 'skills' | 'friends' | 'career' | 'look'>('needs');
  let walls = $state(true);
  let muted = $state(sound.muted);
  $effect(() => sound.onChange(() => (muted = sound.muted)));
  let tick = $state(0);
  $effect(() => {
    const t = setInterval(() => tick++, 1000);
    return () => clearInterval(t);
  });

  const agent = $derived(agents.find((a) => a.id === selectedId) ?? null);
  const sheet = $derived.by((): SimSheet | null => {
    void tick;
    if (!agent) return null;
    return sims.stats(agent.id, Date.now(), engine.agentState(agent.id));
  });
  const nameOf = (id: string) => agents.find((a) => a.id === id)?.name ?? 'someone';
  const career = $derived(agent?.career ?? null);
  const climb = $derived(career ? rankProgress(career) : null);
  const want = $derived(career && agent?.role !== 'lead' ? wantProgress(career) : null);
  // Friends: what this session saw (sims.ts), and the reviews saved on its career (they
  // survive restarts). ponytail: the higher of the two scores, since the replayed log overlaps the saved count.
  const friends = $derived.by(() => {
    const out = new Map((sheet?.relations ?? []).map((r) => [r.agentId, r]));
    for (const [id, n] of Object.entries(career?.friends ?? {})) {
      const score = friendScore(n);
      const cur = out.get(id);
      if (!cur || cur.score < score) out.set(id, { agentId: id, score, label: bondLabel(n), why: `${n} time${n === 1 ? '' : 's'} together: reviews, plans shipped, breaks` });
    }
    return [...out.values()].filter((r) => agents.some((a) => a.id === r.agentId)).sort((a, b) => b.score - a.score);
  });
  let allFriends = $state(false);
  const bondOf = (id: string) => (career?.mentor === id ? 'Mentor' : agents.find((a) => a.id === id)?.career.mentor === agent?.id ? 'Mentee' : null);
  /** Skills from its saved record (they survive restarts and travel with it). */
  const skills = $derived(
    Object.entries(career?.skills ?? {})
      .sort((a, b) => b[1] - a[1])
      .slice(0, 6)
      .map(([name, xp]) => ({ name, ...levelOf(xp) })),
  );
  const workRule = (t: string) => WORK_TRAITS.find((x) => x.id === t)?.rule ?? null;

  // Promotions: a rank above the one seen before (not on first sight); Legend is the title above Staff.
  let ranks = new Map<string, string>();
  let promo = $state<{ id: string; name: string; rank: string; perk: boolean; legend: boolean; offer: ModelOffer | null } | null>(null);
  let promoTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    for (const a of agents) {
      const r = a.career?.legend ? 'legend' : (a.career?.rank ?? 'intern');
      const before = ranks.get(a.id);
      if (before && before !== r && ORDER.indexOf(r) > ORDER.indexOf(before)) {
        const legend = r === 'legend';
        promo = { id: a.id, name: a.name, rank: a.career ? titleOf(a.career) : 'Intern', perk: PERKS.some((p) => p.rank === r), legend, offer: null };
        clearTimeout(promoTimer);
        promoTimer = setTimeout(() => (promo = null), legend ? 30_000 : 15_000);
        // A Legend's focus can grow for real: the biggest context window of its CLI, if bigger.
        if (legend && biggerModel) {
          const id = a.id;
          biggerModel(id)
            .then((offer) => promo?.id === id && offer && (promo = { ...promo, offer }))
            .catch(() => {});
        }
      }
      ranks.set(a.id, r);
    }
  });
  const ORDER = ['intern', 'junior', 'senior', 'staff', 'legend'];

  // The look editor: each pick goes into its look_seed after a `#` (gfx/character.ts).
  const seedOf = (a: Agent) => a.look_seed || a.id;
  const look = $derived(agent ? lookFor(seedOf(agent), agent.color, agent.role, agent.provider) : null);
  const same = (rgb: number[], hex: string) => String(lin(hex)) === String(rgb);
  function pick(k: LookPick, v: string | null) {
    if (agent) onlook?.(agent.id, withPick(seedOf(agent), k, v));
  }
  const OUTFITS = [['hood', 'Hoodie'], ['headphones', 'Headphones'], ['none', 'Tee']] as const;
  const portrait = $derived(agent ? agentPortrait(agent, 96) : '');

  // The action queue: now, then what's next on its own to-do list.
  const queue = $derived.by(() => {
    void tick;
    if (!agent) return null;
    const st = engine.agentState(agent.id);
    if (!st) return null;
    const n = narrate?.(agent.id);
    const home = sheet?.rest.home;
    const nowText = home ? `At home · back in ${backIn(home.until)}` : st.approval ? `Waiting for you: ${st.approval.text}` : n?.parts?.length ? n.parts.map((p) => p.t).join('') : st.status === 'idle' ? 'Taking a break' : st.status;
    const busy = st.status !== 'idle' && st.status !== 'offline' && st.status !== 'done';
    const next = st.todo.filter((t) => !t.done && !t.active).slice(0, 4).map((t) => t.text);
    return { nowText, busy, approval: !!st.approval, next };
  });

  const cost = $derived(agents.reduce((s, a) => s + (a.usage?.cost_usd ?? 0), 0));
  const PLUMB: Record<string, string> = { green: '#4fdc7b', amber: '#ffbf3c', red: '#ff6b5b', blue: '#62a8ff', grey: '#b9c0cc' };

  function act(a: AgentAction, id = agent?.id) {
    if (id) onaction?.(id, a);
    onclosemenu();
  }
  const pct = (x: number | null) => (x == null ? '—' : `${Math.round(x * 100)}%`);
  const backIn = (until: number) => `${Math.max(1, Math.ceil((until - Date.now()) / 60_000))} min`;
  const menuRest = $derived(menuAt ? sims.stats(menuAt.id, Date.now(), engine.agentState(menuAt.id)).rest : null);
  const menuAgent = $derived(menuAt ? agents.find((a) => a.id === menuAt.id) ?? null : null);
  type MenuItem = { a: AgentAction | 'transfer'; label: string; when?: (a: Agent) => boolean; why?: (a: Agent) => string | null };
  const MENU: MenuItem[] = [
    { a: 'chat', label: 'Chat 1:1' },
    { a: 'task', label: 'Show task', when: (a) => !!a.task_id },
    { a: 'details', label: 'Details' },
    { a: 'model', label: 'Change model' },
    { a: 'export', label: 'Export agent', when: (a) => a.role !== 'lead' },
    { a: 'interrupt', label: 'Stop its turn', when: (a) => a.status === 'working' || a.status === 'thinking' },
    { a: 'home', label: 'Send home', when: () => !!menuRest?.canGoHome && !menuRest.home },
    { a: 'callin', label: 'Call in', when: () => !!menuRest?.home },
    { a: 'stop', label: 'Stop it', when: (a) => a.status !== 'offline' },
    // Disabled, with the reason, while it can't go (on a task, mid-turn).
    { a: 'transfer', label: 'Transfer to…', when: (a) => a.role !== 'lead' && !!ontransfer && !!offices, why: (a) => transferBlock(a, tasks) },
  ];
  const items = $derived(menuAgent ? MENU.filter((m) => !m.when || m.when(menuAgent)) : []);
  /** A wider ring past seven items, so the pills don't overlap. */
  const spread = $derived(1 + Math.max(0, items.length - 7) * 0.12);

  // Transfer to…: your other offices, picked where the click menu was.
  let picker = $state<{ id: string; name: string; x: number; y: number; list: Office[] | null; error: string | null } | null>(null);
  function choose(m: MenuItem, a: Agent) {
    if (m.a !== 'transfer') return act(m.a, a.id);
    if (!menuAt || !offices) return;
    const p = { id: a.id, name: a.name, x: menuAt.x, y: menuAt.y, list: null, error: null };
    picker = p;
    onclosemenu();
    offices().then(
      (list) => picker?.id === p.id && (picker = { ...picker, list }),
      (e) => picker?.id === p.id && (picker = { ...picker, list: [], error: String(e) }),
    );
  }
  function transfer(o: Office) {
    if (picker) ontransfer?.(picker.id, o);
    picker = null;
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === 'Escape' && picker) {
      e.stopPropagation();
      picker = null;
    } else if (e.key === 'Escape' && menuAt) {
      e.stopPropagation();
      onclosemenu();
    }
  }
</script>

<svelte:window onkeydown={onkey} />

{#if queue && agent}
  <div class="hud-queue" style:top="{insets.top + 10}px" style:left="{insets.left + 10}px" aria-label="{agent.name}'s actions">
    <div class="q-now" class:amber={queue.approval}>
      <img src={portrait} alt="" width="28" height="28" />
      <span class="q-text">{queue.nowText}</span>
      {#if queue.busy && onaction}
        <button class="q-x" aria-label="Stop {agent.name}'s turn" title="Stop this turn" onclick={() => act('interrupt')}>✕</button>
      {/if}
    </div>
    {#each queue.next as t, i (i)}
      <div class="q-next"><span class="q-n">{i + 1}</span>{t}</div>
    {/each}
  </div>
{/if}

{#if sheet && agent}
  <section class="hud-panel" style:bottom="{insets.bottom + 10}px" style:left="{insets.left + 10}px" aria-label="{agent.name}">
    <header>
      <img src={portrait} alt="" width="52" height="52" />
      <div>
        <div class="p-name"><span class="p-gem" style:background={PLUMB[sheet.plumbob]}></span>{agent.name}</div>
        <div class="p-sub">{agent.role} · {agent.provider} · <b>{sheet.mood}</b></div>
        <div class="p-traits">
          {#each traitsOf(agent.id) as t (t)}
            {@const rule = workRule(t)}
            <span class:work={rule} title={rule ? `Changes how it works: ${rule}` : TRAITS[t].about}>{TRAITS[t].label}</span>
          {/each}
        </div>
        {#if career && climb}
          <div class="p-rank" title={climb.next ? `${climb.toGo} XP to ${climb.next}` : 'Top rank'}>
            <b class:legend={career.legend}>{titleOf(career)}</b><span class="bar"><span class="fill xp" style:width="{Math.round(climb.progress * 100)}%"></span></span><span>{career.xp} XP</span>
          </div>
        {/if}
        {#if want}
          <div class="p-want" class:met={want.met} title={want.met ? `Fulfilled: +${WANT_XP} XP and a good mood` : `Today's want, from its real work: +${WANT_XP} XP and a good mood once it's done`}>
            <span>{want.met ? '🌟' : '🎯'}</span><span class="truncate">{want.label}</span><span>{want.met ? 'Done' : `${want.done}/${want.goal}`}</span>
          </div>
        {/if}
      </div>
    </header>
    <div class="tabs" role="tablist">
      {#each [['needs', 'Needs'], ['mood', 'Mood'], ['skills', 'Skills'], ['friends', 'Friends'], ['career', 'Career'], ...(onlook ? [['look', 'Look']] : [])] as [k, l] (k)}
        <button role="tab" aria-selected={tab === k} class:on={tab === k} onclick={() => (tab = k as typeof tab)}>{l}</button>
      {/each}
    </div>
    <div class="p-body">
      {#if tab === 'needs'}
        {#each [['Energy', sheet.needs.energy, `Tires over a shift of real work; going home refills it (stamina ${career?.stamina ?? 0})`], ['Focus', sheet.needs.focus, 'Context window left'], ['Morale', sheet.needs.morale, 'How its recent work went'], ['Hunger', sheet.needs.hunger, 'Work makes it hungry; the stove cooks a meal fastest, then the microwave; the fridge or a toaster only snacks (up to 60%); home fills it up'], ['Thirst', sheet.needs.thirst, 'Work makes it thirsty; the water cooler fills it fastest, the fridge or a coffee partly; home fills it up']] as [label, v, why] (label)}
          <div class="need" title={why as string}>
            <span class="n-l">{label}</span>
            <span class="bar"><span class="fill" class:low={v != null && (v as number) < 0.25} class:mid={v != null && (v as number) >= 0.25 && (v as number) < 0.55} style:width={v == null ? '0%' : `${Math.round((v as number) * 100)}%`}></span></span>
            <span class="n-v">{pct(v as number | null)}</span>
          </div>
        {/each}
        {@const r = sheet.rest}
        {#if r.home}
          <div class="rest"><span>🏠 {r.home.sick ? 'Resting off a sickness' : 'At home'} · back in {backIn(r.home.until)}</span>{#if onaction}<button onclick={() => onaction?.(agent.id, 'callin')} title="Overtime: morale −15%, and it keeps only half the energy it got back at home">Call in</button>{/if}</div>
        {:else if r.canGoHome && onaction}
          <div class="rest">
            <span>{r.sick ? '🤒 Sick: goes home after this task' : r.exhausted ? '😵 Goes home after this task' : r.caffeine ? `☕ ${r.caffeine} cup${r.caffeine === 1 ? '' : 's'} of coffee${r.jittery ? ' · jittery' : ''}` : r.cap < 1 ? `Capped at ${Math.round(r.cap * 100)}% this shift` : 'Home refills its energy'}</span>
            <button onclick={() => onaction?.(agent.id, 'home')} title="It walks out and comes back rested in a few minutes; its conversation stays as it is">Send home</button>
          </div>
        {/if}
      {:else if tab === 'mood'}
        {#if sheet.effects.length}
          <ul class="effects" aria-label="What its mood does">{#each sheet.effects as e (e)}<li>{e}</li>{/each}</ul>
        {:else if agent.role !== 'lead'}
          <p class="hint">Above 75% morale its energy lasts longer and it seeks friends on breaks; under 35% it tires faster. Breaks with friends, good meals and passing checks lift it.</p>
        {/if}
        {#each sheet.moodlets as m (m.key)}
          <div class="moodlet" data-tone={m.tone}><b>{m.label}</b><span>{m.why}</span></div>
        {:else}
          <p class="empty">No strong feelings right now.</p>
        {/each}
      {:else if tab === 'skills'}
        {#each skills as s (s.name)}
          <div class="skill">
            <span class="n-l">{s.name}</span>
            <span class="pips">{#each Array(10) as _, i (i)}<span class="pip" class:on={i < s.level} class:part={i === s.level} style:--p={i === s.level ? s.progress : 0}></span>{/each}</span>
            <span class="n-v">{s.level}</span>
          </div>
        {:else}
          <p class="empty">Skills grow with the files it changes and the checks it passes.</p>
        {/each}
      {:else if tab === 'look' && look}
        <div class="look">
          <div class="lk"><span class="n-l">Hair</span>
            <button class="step" aria-label="Previous hairstyle" onclick={() => pick('style', String((look.style + LOOK_STYLES - 1) % LOOK_STYLES))}>‹</button>
            <span class="n-v">{look.style + 1} / {LOOK_STYLES}</span>
            <button class="step" aria-label="Next hairstyle" onclick={() => pick('style', String((look.style + 1) % LOOK_STYLES))}>›</button>
          </div>
          <div class="lk"><span class="n-l">Color</span><span class="sw">{#each HAIR as h, i (h)}<button class="dot" class:on={same(look.hair, h)} style:background={h} aria-label="Hair color {i + 1}" onclick={() => pick('hair', String(i))}></button>{/each}</span></div>
          <div class="lk"><span class="n-l">Skin</span><span class="sw">{#each SKIN as h, i (h)}<button class="dot" class:on={same(look.skin, h)} style:background={h} aria-label="Skin tone {i + 1}" onclick={() => pick('skin', String(i))}></button>{/each}</span></div>
          {#if agent.role === 'worker' || agent.role === 'solo'}
            <div class="lk"><span class="n-l">Outfit</span>{#each OUTFITS as [k, l] (k)}<button class="chip" class:on={look.accessory === k} onclick={() => pick('acc', k)}>{l}</button>{/each}</div>
          {/if}
          <div class="lk"><span class="n-l">Glasses</span><button class="chip" class:on={look.glasses} onclick={() => pick('glasses', look.glasses ? '0' : '1')}>{look.glasses ? 'On' : 'Off'}</button></div>
          <div class="lk">
            <button class="link" onclick={() => onlook?.(agent.id, Math.random().toString(36).slice(2, 8))}>Shuffle</button>
            {#if lookPicks(seedOf(agent)).picks.size}<button class="link" onclick={() => onlook?.(agent.id, lookPicks(seedOf(agent)).base)}>Undo my picks</button>{/if}
          </div>
        </div>
      {:else if tab === 'career' && career}
        <div class="stats">
          <span><b>{career.stats.merged}</b> merged</span><span><b>{career.stats.checks_passed}</b> checks passed</span><span><b>{career.stats.approved}</b> approved</span><span><b>{career.stats.commits}</b> commits</span>
          <span title="From its days on the crew, merges and shifts; less each time it got sick"><b>{career.stamina}</b> stamina</span><span><b>{career.stats.shifts}</b> shifts</span>
        </div>
        {@const trusted = PERKS.filter((p) => atLeast(career.trust, p.rank) && atLeast(career.rank, p.rank)).at(-1)}
        {@const offer = PERKS.find((p) => !atLeast(career.trust, p.rank) && atLeast(career.rank, p.rank))}
        {@const next = PERKS.find((p) => !atLeast(career.rank, p.rank))}
        <div class="trust">
          <div class="perk" class:on={!!trusted} title={PERKS.map((p) => `${rankLabel(p.rank)}: ${p.perk}`).join('\n')}>
            <span>{#if trusted}<b>Trusted</b> {trusted.perk}{:else}<b>Not trusted yet</b> it asks before it acts{/if}</span>
            {#if offer && onaction}<button onclick={() => onaction?.(agent.id, 'trust')}>Trust as {rankLabel(offer.rank)}</button>
            {:else if trusted && onaction}<button class="link" onclick={() => onaction?.(agent.id, 'untrust')}>Take back</button>{/if}
          </div>
          {#if next}<p class="next">At {rankLabel(next.rank)}: {next.perk.toLowerCase()}</p>{/if}
        </div>
        <Scrapbook milestones={career.milestones} highlights />
      {:else}
        {#each friends.slice(0, allFriends ? undefined : 6) as r (r.agentId)}
          {@const bond = bondOf(r.agentId)}
          <div class="friend" title={bond === 'Mentor' ? `Its mentor: their reviews earn it +${MENTORED_XP} XP while it is an Intern or Junior` : bond === 'Mentee' ? 'It mentors them: its reviews earn them extra XP' : r.why}>
            <span class="n-l">{nameOf(r.agentId)}<em>{bond ?? r.label}</em></span><span class="bar"><span class="fill" style:width="{r.score}%"></span></span>
          </div>
        {:else}
          <p class="empty">Gets to know the agents it reviews or is reviewed by, ships a plan with, or spends a break with. A Senior reviewing an Intern or Junior becomes its mentor.</p>
        {/each}
        {#if friends.length > 6}<button class="link" onclick={() => (allFriends = !allFriends)}>{allFriends ? 'Fewer' : `All ${friends.length}`}</button>{/if}
      {/if}
    </div>
  </section>
{/if}

{#if menuAt && menuAgent}
  <div class="pie" style:left="{menuAt.x}px" style:top="{menuAt.y}px" role="menu" aria-label="{menuAgent.name}">
    {#each items as m, i (m.a)}
      {@const ang = (i / items.length) * Math.PI * 2 - Math.PI / 2}
      {@const why = m.why?.(menuAgent) ?? null}
      <button role="menuitem" style:transform="translate({Math.cos(ang) * spread * 92}px, {Math.sin(ang) * spread * 70}px) translate(-50%, -50%)" disabled={!!why} aria-disabled={!!why} title={why ?? undefined} onclick={() => choose(m, menuAgent)}>{m.label}</button>
    {/each}
    <span class="pie-name">{menuAgent.name}</span>
  </div>
{/if}

{#if picker}
  <div class="transfer" style:left="{picker.x}px" style:top="{picker.y}px" role="dialog" aria-label="Transfer {picker.name} to another office">
    <header><b>Transfer {picker.name} to…</b><button class="x" aria-label="Cancel" onclick={() => (picker = null)}>✕</button></header>
    {#if picker.list === null}
      <p class="empty">Looking for your other offices…</p>
    {:else if picker.error}
      <p class="empty">{picker.error}</p>
    {:else}
      {#each picker.list as o (o.id)}
        <button class="office" onclick={() => transfer(o)} title={o.repo_path}>
          <span class="o-name">{o.name}</span>
          <span class="o-crew">{o.crew === 0 ? 'nobody yet' : `${o.crew} on the crew`}</span>
        </button>
      {:else}
        <p class="empty">Open another repo once to get a second office.</p>
      {/each}
      {#if picker.list.length}<p class="note">Its career, traits and memory go with it; trust and friendships stay here.</p>{/if}
    {/if}
  </div>
{/if}

{#if promo}
  <div class="promo" style:top="{insets.top + 12}px" role="status">
    <span>🎉 <b>{promo.name}</b> was promoted to <b>{promo.rank}</b></span>
    {#if onaction && promo.perk}<button onclick={() => { if (promo) onaction?.(promo.id, 'trust'); promo = null; }}>Trust it more</button>{/if}
    {#if promo.legend}
      {#if ongivemodel && promo.offer}{@const o = promo.offer}<button title="Its CLI's biggest context window: more room to focus. Nothing changes unless you click." onclick={() => { if (promo) ongivemodel?.(promo.id, o.model); promo = null; }}>Give it {o.model} · {contextLabel(o.context_window)} context</button>{/if}
    {:else if onaction}<button onclick={() => { if (promo) onaction?.(promo.id, 'model'); promo = null; }}>Give a stronger model</button>{/if}
    <button class="x" aria-label="Dismiss" onclick={() => (promo = null)}>✕</button>
  </div>
{/if}

<div class="hud-live" style:bottom="{insets.bottom + 10}px" style:right="{insets.right + 10}px">
  <span class="money" title="List-price estimate of this session's tokens. Subscriptions are not charged per token.">≈ ${cost.toFixed(2)}</span>
  {#if onpause}
    <button class:on={paused} onclick={() => onpause?.(!paused)} title={paused ? 'Resume: start new tasks again' : 'Pause: no new tasks start (running ones finish)'}>{paused ? '▶ Resume' : '⏸ Pause'}</button>
  {/if}
  <button onclick={() => { walls = !walls; engine.setWalls(walls); }} title="Walls up or down">{walls ? 'Walls down' : 'Walls up'}</button>
  <button onclick={() => { muted = !muted; sound.setMuted(muted); }} title={muted ? 'Sound off: turn it on (volume in Settings)' : 'Sound on: mute the office'} aria-label={muted ? 'Unmute the office' : 'Mute the office'} aria-pressed={muted}>{muted ? '🔇' : '🔊'}</button>
</div>

<style>
  .hud-queue, .hud-panel, .hud-live, .pie button {
    font: 500 12.5px/1.35 var(--of-font);
    color: var(--ink-1);
  }
  .hud-queue { position: absolute; z-index: 6; display: flex; flex-direction: column; gap: 4px; max-width: 360px; pointer-events: auto; }
  .q-now { display: flex; align-items: center; gap: 8px; padding: 5px 6px 5px 5px; border-radius: 12px; background: var(--glass-strong); box-shadow: inset 0 0 0 1px var(--line); backdrop-filter: blur(14px); }
  .q-now.amber { box-shadow: inset 0 0 0 1.5px var(--amber); }
  .q-now img { border-radius: 8px; background: var(--surface-3); }
  .q-text { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .q-x { width: 22px; height: 22px; border-radius: 6px; border: 0; background: var(--bad-soft); color: var(--bad); cursor: pointer; }
  .q-x:hover { background: color-mix(in srgb, var(--bad) 32%, transparent); }
  .q-next { display: flex; gap: 8px; align-items: center; padding: 3px 10px 3px 8px; border-radius: 10px; background: var(--glass); color: var(--ink-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .q-n { font: 600 10px var(--of-mono); color: var(--ink-3); }

  .hud-panel { position: absolute; z-index: 6; width: 300px; border-radius: 16px; background: var(--glass-strong); backdrop-filter: blur(18px) saturate(140%); box-shadow: var(--pop-shadow); overflow: hidden; pointer-events: auto; }
  header { display: flex; gap: 10px; align-items: center; padding: 12px 12px 8px; }
  header img { border-radius: 12px; background: var(--surface-3); }
  .p-name { display: flex; align-items: center; gap: 7px; font: 600 15px/1.2 var(--of-font); }
  .p-gem { width: 10px; height: 14px; clip-path: polygon(50% 0, 100% 50%, 50% 100%, 0 50%); }
  .p-sub { color: var(--ink-2); text-transform: capitalize; margin-top: 2px; }
  .p-sub b { color: var(--ink-1); font-weight: 600; }
  .p-traits { display: flex; gap: 4px; margin-top: 5px; }
  .p-traits span { padding: 1px 7px; border-radius: 999px; background: color-mix(in srgb, var(--focus) 14%, transparent); color: var(--focus); font-size: 11px; }
  .p-traits span.work { box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--of-green) 55%, transparent); color: var(--of-green); background: color-mix(in srgb, var(--of-green) 12%, transparent); }
  .p-rank { display: grid; grid-template-columns: auto 1fr auto; gap: 6px; align-items: center; margin-top: 6px; color: var(--ink-2); font-size: 11px; }
  .p-rank b { color: var(--of-gold); font-weight: 600; }
  .fill.xp { background: linear-gradient(90deg, var(--amber), var(--of-gold)); }
  .p-want { display: grid; grid-template-columns: auto 1fr auto; gap: 6px; align-items: center; margin-top: 5px; color: var(--ink-2); font-size: 11px; }
  .p-want .truncate { color: var(--ink-1); }
  .p-want.met .truncate { color: var(--of-green); }
  .p-rank b.legend { text-shadow: 0 0 8px color-mix(in srgb, var(--amber) 80%, transparent); }
  .rest { display: flex; align-items: center; justify-content: space-between; gap: 8px; margin-top: 2px; color: var(--ink-2); font-size: 11.5px; }
  .rest button { flex: none; height: 24px; padding: 0 12px; border: 0; border-radius: 999px; background: var(--surface-3); color: var(--ink-1); font: 600 11.5px inherit; cursor: pointer; }
  .rest button:hover { background: var(--focus); color: var(--on-aurora); }
  .stats { display: grid; grid-template-columns: 1fr 1fr; gap: 4px 10px; color: var(--ink-2); }
  .stats b { color: var(--ink-1); }
  .promo { position: absolute; left: 50%; transform: translateX(-50%); z-index: 410; display: flex; gap: 10px; align-items: center; padding: 8px 8px 8px 14px; border-radius: 999px; background: var(--glass-strong); box-shadow: inset 0 0 0 1.5px var(--of-gold), var(--pop-shadow); color: var(--ink-1); font: 500 13px var(--of-font); pointer-events: auto; animation: pop 0.25s ease-out both; }
  .promo button { height: 26px; padding: 0 12px; border: 0; border-radius: 999px; background: var(--amber); color: #2a1a04; font: 600 12px inherit; cursor: pointer; }
  .trust { display: grid; gap: 6px; margin-bottom: 10px; }
  .next, .hint { margin: 0 2px; font-size: 11px; color: var(--ink-3); }
  .hint { margin-bottom: 8px; line-height: 1.45; }
  .effects { margin: 0 0 8px; padding: 7px 9px 7px 24px; border-radius: 9px; background: var(--amber-soft); color: var(--ink-1); font-size: 11.5px; line-height: 1.5; }
  .perk { display: flex; gap: 8px; align-items: center; justify-content: space-between; padding: 6px 8px; border-radius: 8px; background: var(--surface-1); color: var(--ink-1); font-size: 11.5px; }
  .perk b { color: var(--of-gold); font-weight: 600; margin-right: 4px; }
  .perk.on { box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--of-gold) 50%, transparent); }
  .perk.locked { opacity: 0.55; }
  .perk .tag { flex: none; color: var(--ink-2); font-size: 10.5px; }
  .perk.on .tag { color: var(--of-gold); }
  .perk button { flex: none; height: 22px; padding: 0 10px; border: 0; border-radius: 999px; background: var(--amber); color: #2a1a04; font: 600 11px inherit; cursor: pointer; }
  .link { justify-self: start; border: 0; background: none; padding: 0; color: var(--ink-2); font: 500 11px inherit; text-decoration: underline; cursor: pointer; }
  .promo button.x { background: none; color: var(--ink-2); padding: 0 6px; }
  .tabs { display: flex; gap: 2px; padding: 0 8px; }
  .look { display: flex; flex-direction: column; gap: 7px; }
  .lk { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .lk .n-l { width: 52px; }
  .sw { display: flex; gap: 4px; flex-wrap: wrap; }
  .dot { width: 16px; height: 16px; border-radius: 50%; border: 2px solid transparent; padding: 0; cursor: pointer; }
  .dot.on { border-color: var(--ink-1); }
  .step, .chip { height: 22px; min-width: 22px; padding: 0 8px; border: 0; border-radius: 6px; background: var(--surface-2); color: var(--ink-2); font: inherit; cursor: pointer; }
  .chip.on { background: var(--surface-3); color: var(--ink-1); }
  .tabs button { flex: 1; height: 28px; border: 0; border-radius: 8px; background: none; color: var(--ink-2); font: inherit; cursor: pointer; }
  .tabs button.on { background: var(--surface-2); color: var(--ink-1); }
  .p-body { display: flex; flex-direction: column; gap: 7px; padding: 10px 12px 14px; min-height: 104px; }
  .need, .skill, .friend { display: grid; grid-template-columns: 64px 1fr 34px; align-items: center; gap: 8px; }
  .friend { grid-template-columns: 110px 1fr; }
  .friend em { margin-left: 5px; color: var(--of-gold); font-style: normal; font-size: 10px; }
  .n-l { color: var(--ink-1); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .n-v { text-align: right; color: var(--ink-2); font: 500 11.5px var(--of-mono); }
  .bar { height: 9px; border-radius: 5px; background: var(--surface-2); overflow: hidden; }
  .fill { display: block; height: 100%; border-radius: 5px; background: var(--of-green); transition: width 0.6s; }
  .fill.mid { background: linear-gradient(90deg, var(--amber), var(--of-gold)); }
  .fill.low { background: var(--bad); }
  .pips { display: flex; gap: 3px; }
  .pip { flex: 1; height: 8px; border-radius: 2px; background: var(--surface-2); }
  .pip.on { background: var(--focus); }
  .pip.part { background: linear-gradient(90deg, var(--focus) calc(var(--p) * 100%), var(--surface-2) 0); }
  .moodlet { display: flex; flex-direction: column; padding: 6px 9px; border-radius: 9px; background: var(--surface-1); border-left: 3px solid var(--ink-2); }
  .moodlet[data-tone='good'] { border-left-color: var(--of-green); }
  .moodlet[data-tone='bad'] { border-left-color: var(--bad); }
  .moodlet span { color: var(--ink-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .empty { margin: 0; color: var(--ink-3); }
  /* The Career tab's Scrapbook is also in GhostBoard's dark panel, so its rule and toggle are literals for dark: here they follow the theme. */
  .p-body :global(.book) { border-left-color: var(--line-2); }
  .p-body :global(.more) { color: var(--ink-2); }

  .pie { position: absolute; z-index: 7; width: 0; height: 0; pointer-events: none; }
  .pie button { position: absolute; left: 0; top: 0; padding: 7px 12px; border: 0; border-radius: 999px; background: var(--glass-strong); box-shadow: var(--pop-shadow); white-space: nowrap; cursor: pointer; pointer-events: auto; animation: pop 0.18s ease-out both; }
  .pie button:hover, .pie button:focus-visible { background: var(--focus); color: var(--on-aurora); outline: none; }
  .pie button:disabled { color: var(--ink-3); cursor: not-allowed; }
  .pie button:disabled:hover { background: var(--glass-strong); }

  /* Above the name tags (a selected one is at 1100, overlay.ts), its own agent's included. */
  .transfer { position: absolute; z-index: 1200; transform: translate(-50%, -50%); width: 250px; display: flex; flex-direction: column; gap: 6px; padding: 10px; border-radius: 14px; background: var(--glass-strong); backdrop-filter: blur(18px); box-shadow: var(--pop-shadow); color: var(--ink-1); font: 500 12.5px/1.35 var(--of-font); pointer-events: auto; animation: pop 0.18s ease-out both; }
  .transfer header { display: flex; justify-content: space-between; align-items: center; padding: 0 0 2px 2px; }
  .transfer .x { border: 0; background: none; color: var(--ink-2); cursor: pointer; font: inherit; }
  .transfer .office { display: flex; justify-content: space-between; align-items: center; gap: 8px; padding: 7px 10px; border: 0; border-radius: 9px; background: var(--surface-2); color: var(--ink-1); font: inherit; text-align: left; cursor: pointer; }
  .transfer .office:hover, .transfer .office:focus-visible { background: var(--focus); color: var(--on-aurora); outline: none; }
  .o-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
  .o-crew { flex: none; color: var(--ink-2); font-size: 11px; }
  .office:hover .o-crew, .office:focus-visible .o-crew { color: var(--on-aurora); }
  .transfer .note { margin: 2px 2px 0; color: var(--ink-3); font-size: 11px; }
  /* Over the scene, which follows the hour, not the theme: white on a dark shadow reads on both. */
  .pie-name { position: absolute; transform: translate(-50%, -50%); font: 600 12px var(--of-font); color: #fff; text-shadow: 0 1px 3px #000; }
  @keyframes pop { from { opacity: 0; scale: 0.6; } }

  .hud-live { position: absolute; z-index: 6; display: flex; gap: 6px; align-items: center; pointer-events: auto; }
  .hud-live button, .money { height: 30px; padding: 0 12px; border: 0; border-radius: 999px; background: var(--glass-strong); box-shadow: inset 0 0 0 1px var(--line); color: var(--ink-1); font: inherit; display: inline-flex; align-items: center; cursor: pointer; }
  .money { cursor: default; color: var(--of-green); font-family: var(--of-mono); }
  .hud-live button.on { box-shadow: inset 0 0 0 1.5px var(--of-gold); color: var(--of-gold); }
  @media (prefers-reduced-motion: reduce) { .pie button, .transfer { animation: none; } .fill { transition: none; } }
</style>
