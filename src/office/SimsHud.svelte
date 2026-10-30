<!--
  The Sims layer over the office (sims.ts for the numbers, all from real data):
  - top-left: the selected agent's action queue (what it does now, ✕ to interrupt; its to-dos queued)
  - bottom-left: its panel: portrait, rank, mood, and tabs for Needs / Mood / Skills / Friends / Career
  - top-center: a promotion (its real track record), with the offer of trust (Junior, Senior and Staff
    perks) and of a stronger model
  - around a clicked agent: the click menu (chat, task, model, stop, details)
  - bottom-right: the estimated cost so far, Pause new work, walls up/down
  Shown only in the full office (not as the backdrop), inside the app's insets.
-->
<script lang="ts" module>
  export type AgentAction = 'chat' | 'task' | 'model' | 'stop' | 'interrupt' | 'details' | 'export' | 'trust' | 'untrust';
</script>

<script lang="ts">
  import type { AccountInfo } from '../lib/gen/AccountInfo';
  import type { Agent } from '../lib/gen/Agent';
  import type { OfficeEngine } from './engine';
  import type { NarrateFn } from './OfficeView.svelte';
  import { agentPortrait } from './portrait';
  import Scrapbook from './Scrapbook.svelte';
  import { sound } from './sound';
  import { levelOf, type SimSheet, type SimsTracker } from './sims';
  import { TRAITS, traitsOf } from './traits';
  import { atLeast, MENTORED_XP, PERKS, rankLabel, rankProgress, TRAITS as WORK_TRAITS } from '../lib/util/career';

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
    onclosemenu: () => void;
    paused?: boolean;
    onpause?: (paused: boolean) => void;
  }
  let { engine, sims, agents, accounts, selectedId, menuAt, insets, narrate, onaction, onclosemenu, paused = false, onpause }: Props = $props();

  let tab = $state<'needs' | 'mood' | 'skills' | 'friends' | 'career'>('needs');
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
  // Friends: what this session saw (sims.ts), and the reviews saved on its career (they
  // survive restarts). ponytail: the higher of the two scores, since the replayed log overlaps the saved count.
  const friends = $derived.by(() => {
    const out = new Map((sheet?.relations ?? []).map((r) => [r.agentId, r]));
    for (const [id, n] of Object.entries(career?.friends ?? {})) {
      const score = Math.min(100, Math.round(25 * Math.log2(1 + 2 * n)));
      const cur = out.get(id);
      if (!cur || cur.score < score) out.set(id, { agentId: id, score, why: cur?.why ?? `${n} review${n === 1 ? '' : 's'} between them` });
    }
    return [...out.values()].filter((r) => agents.some((a) => a.id === r.agentId)).sort((a, b) => b.score - a.score);
  });
  const bondOf = (id: string) => (career?.mentor === id ? 'Mentor' : agents.find((a) => a.id === id)?.career.mentor === agent?.id ? 'Mentee' : null);
  /** Skills from its saved record (they survive restarts and travel with it). */
  const skills = $derived(
    Object.entries(career?.skills ?? {})
      .sort((a, b) => b[1] - a[1])
      .slice(0, 6)
      .map(([name, xp]) => ({ name, ...levelOf(xp) })),
  );
  const workRule = (t: string) => WORK_TRAITS.find((x) => x.id === t)?.rule ?? null;

  // Promotions: a rank above the one seen before (not on first sight).
  let ranks = new Map<string, string>();
  let promo = $state<{ id: string; name: string; rank: string; perk: boolean } | null>(null);
  let promoTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    for (const a of agents) {
      const r = a.career?.rank ?? 'intern';
      const before = ranks.get(a.id);
      if (before && before !== r && ORDER.indexOf(r) > ORDER.indexOf(before)) {
        promo = { id: a.id, name: a.name, rank: rankLabel(r), perk: PERKS.some((p) => p.rank === r) };
        clearTimeout(promoTimer);
        promoTimer = setTimeout(() => (promo = null), 15_000);
      }
      ranks.set(a.id, r);
    }
  });
  const ORDER = ['intern', 'junior', 'senior', 'staff'];
  const portrait = $derived(agent ? agentPortrait(agent, 96) : '');

  // The action queue: now, then what's next on its own to-do list.
  const queue = $derived.by(() => {
    void tick;
    if (!agent) return null;
    const st = engine.agentState(agent.id);
    if (!st) return null;
    const n = narrate?.(agent.id);
    const nowText = st.approval ? `Waiting for you: ${st.approval.text}` : n?.parts?.length ? n.parts.map((p) => p.t).join('') : st.status === 'idle' ? 'Taking a break' : st.status;
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
  const menuAgent = $derived(menuAt ? agents.find((a) => a.id === menuAt.id) ?? null : null);
  const MENU: { a: AgentAction; label: string; when?: (a: Agent) => boolean }[] = [
    { a: 'chat', label: 'Chat 1:1' },
    { a: 'task', label: 'Show task', when: (a) => !!a.task_id },
    { a: 'details', label: 'Details' },
    { a: 'model', label: 'Change model' },
    { a: 'export', label: 'Export agent', when: (a) => a.role !== 'lead' },
    { a: 'interrupt', label: 'Stop its turn', when: (a) => a.status === 'working' || a.status === 'thinking' },
    { a: 'stop', label: 'Send home', when: (a) => a.status !== 'offline' },
  ];
  const items = $derived(menuAgent ? MENU.filter((m) => !m.when || m.when(menuAgent)) : []);

  function onkey(e: KeyboardEvent) {
    if (e.key === 'Escape' && menuAt) {
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
            <b>{rankLabel(career.rank)}</b><span class="bar"><span class="fill xp" style:width="{Math.round(climb.progress * 100)}%"></span></span><span>{career.xp} XP</span>
          </div>
        {/if}
      </div>
    </header>
    <div class="tabs" role="tablist">
      {#each [['needs', 'Needs'], ['mood', 'Mood'], ['skills', 'Skills'], ['friends', 'Friends'], ['career', 'Career']] as [k, l] (k)}
        <button role="tab" aria-selected={tab === k} class:on={tab === k} onclick={() => (tab = k as typeof tab)}>{l}</button>
      {/each}
    </div>
    <div class="p-body">
      {#if tab === 'needs'}
        {#each [['Energy', sheet.needs.energy, 'Quota left on its account'], ['Focus', sheet.needs.focus, 'Context window left'], ['Morale', sheet.needs.morale, 'How its recent work went']] as [label, v, why] (label)}
          <div class="need" title={why as string}>
            <span class="n-l">{label}</span>
            <span class="bar"><span class="fill" class:low={v != null && (v as number) < 0.25} class:mid={v != null && (v as number) >= 0.25 && (v as number) < 0.55} style:width={v == null ? '0%' : `${Math.round((v as number) * 100)}%`}></span></span>
            <span class="n-v">{pct(v as number | null)}</span>
          </div>
        {/each}
      {:else if tab === 'mood'}
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
      {:else if tab === 'career' && career}
        <div class="stats">
          <span><b>{career.stats.merged}</b> merged</span><span><b>{career.stats.checks_passed}</b> checks passed</span><span><b>{career.stats.approved}</b> approved</span><span><b>{career.stats.commits}</b> commits</span>
        </div>
        <div class="trust">
          {#each PERKS as p (p.rank)}
            {@const on = atLeast(career.trust, p.rank) && atLeast(career.rank, p.rank)}
            <div class="perk" class:on class:locked={!atLeast(career.rank, p.rank)}>
              <span><b>{rankLabel(p.rank)}</b> {p.perk}</span>
              {#if on}<span class="tag">Trusted</span>
              {:else if atLeast(career.rank, p.rank) && onaction}<button onclick={() => onaction?.(agent.id, 'trust')}>Trust</button>
              {:else}<span class="tag">at {rankLabel(p.rank)}</span>{/if}
            </div>
          {/each}
          {#if atLeast(career.trust, 'junior') && onaction}<button class="link" onclick={() => onaction?.(agent.id, 'untrust')}>Take trust back</button>{/if}
        </div>
        <Scrapbook milestones={career.milestones} />
      {:else}
        {#each friends as r (r.agentId)}
          {@const bond = bondOf(r.agentId)}
          <div class="friend" title={bond === 'Mentor' ? `Its mentor: their reviews earn it +${MENTORED_XP} XP while it is an Intern or Junior` : bond === 'Mentee' ? 'It mentors them: its reviews earn them extra XP' : r.why}>
            <span class="n-l">{nameOf(r.agentId)}{#if bond}<em>{bond}</em>{/if}</span><span class="bar"><span class="fill" style:width="{r.score}%"></span></span>
          </div>
        {:else}
          <p class="empty">Gets to know the agents it asks, reviews or is reviewed by. A Senior reviewing an Intern or Junior becomes its mentor.</p>
        {/each}
      {/if}
    </div>
  </section>
{/if}

{#if menuAt && menuAgent}
  <div class="pie" style:left="{menuAt.x}px" style:top="{menuAt.y}px" role="menu" aria-label="{menuAgent.name}">
    {#each items as m, i (m.a)}
      {@const ang = (i / items.length) * Math.PI * 2 - Math.PI / 2}
      <button role="menuitem" style:transform="translate({Math.cos(ang) * 92}px, {Math.sin(ang) * 70}px) translate(-50%, -50%)" onclick={() => act(m.a, menuAgent.id)}>{m.label}</button>
    {/each}
    <span class="pie-name">{menuAgent.name}</span>
  </div>
{/if}

{#if promo}
  <div class="promo" style:top="{insets.top + 12}px" role="status">
    <span>🎉 <b>{promo.name}</b> was promoted to <b>{promo.rank}</b></span>
    {#if onaction && promo.perk}<button onclick={() => { if (promo) onaction?.(promo.id, 'trust'); promo = null; }}>Trust it more</button>{/if}
    {#if onaction}<button onclick={() => { if (promo) onaction?.(promo.id, 'model'); promo = null; }}>Give a stronger model</button>{/if}
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
    font: 500 12.5px/1.35 'Inter Variable', Inter, system-ui, sans-serif;
    color: #eef1f7;
  }
  .hud-queue { position: absolute; z-index: 6; display: flex; flex-direction: column; gap: 4px; max-width: 360px; pointer-events: auto; }
  .q-now { display: flex; align-items: center; gap: 8px; padding: 5px 6px 5px 5px; border-radius: 12px; background: rgba(18, 22, 32, 0.82); box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.08); backdrop-filter: blur(14px); }
  .q-now.amber { box-shadow: inset 0 0 0 1.5px #ffbf3c; }
  .q-now img { border-radius: 8px; background: #2a2f3d; }
  .q-text { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .q-x { width: 22px; height: 22px; border-radius: 6px; border: 0; background: rgba(255, 107, 91, 0.18); color: #ff8a7c; cursor: pointer; }
  .q-x:hover { background: rgba(255, 107, 91, 0.32); }
  .q-next { display: flex; gap: 8px; align-items: center; padding: 3px 10px 3px 8px; border-radius: 10px; background: rgba(18, 22, 32, 0.6); color: #aeb5c6; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .q-n { font: 600 10px ui-monospace, monospace; color: #6c748b; }

  .hud-panel { position: absolute; z-index: 6; width: 300px; border-radius: 16px; background: rgba(18, 22, 32, 0.86); backdrop-filter: blur(18px) saturate(140%); box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.08), 0 20px 50px -20px rgba(0, 0, 0, 0.7); overflow: hidden; pointer-events: auto; }
  header { display: flex; gap: 10px; align-items: center; padding: 12px 12px 8px; }
  header img { border-radius: 12px; background: #2a2f3d; }
  .p-name { display: flex; align-items: center; gap: 7px; font: 600 15px/1.2 'Inter Variable', Inter, system-ui, sans-serif; }
  .p-gem { width: 10px; height: 14px; clip-path: polygon(50% 0, 100% 50%, 50% 100%, 0 50%); }
  .p-sub { color: #9aa2b6; text-transform: capitalize; margin-top: 2px; }
  .p-sub b { color: #eef1f7; font-weight: 600; }
  .p-traits { display: flex; gap: 4px; margin-top: 5px; }
  .p-traits span { padding: 1px 7px; border-radius: 999px; background: rgba(98, 182, 247, 0.14); color: #9fcbf5; font-size: 11px; }
  .p-traits span.work { box-shadow: inset 0 0 0 1px rgba(123, 228, 149, 0.55); color: #a9efbd; background: rgba(79, 220, 123, 0.12); }
  .p-rank { display: grid; grid-template-columns: auto 1fr auto; gap: 6px; align-items: center; margin-top: 6px; color: #9aa2b6; font-size: 11px; }
  .p-rank b { color: #ffd27a; font-weight: 600; }
  .fill.xp { background: linear-gradient(90deg, #e9b03a, #ffd27a); }
  .stats { display: grid; grid-template-columns: 1fr 1fr; gap: 4px 10px; color: #9aa2b6; }
  .stats b { color: #eef1f7; }
  .promo { position: absolute; left: 50%; transform: translateX(-50%); z-index: 410; display: flex; gap: 10px; align-items: center; padding: 8px 8px 8px 14px; border-radius: 999px; background: rgba(18, 22, 32, 0.92); box-shadow: inset 0 0 0 1.5px #ffd27a, 0 16px 40px -16px rgba(0, 0, 0, 0.7); color: #eef1f7; font: 500 13px 'Inter Variable', Inter, system-ui, sans-serif; pointer-events: auto; animation: pop 0.25s ease-out both; }
  .promo button { height: 26px; padding: 0 12px; border: 0; border-radius: 999px; background: #e9b03a; color: #1b1405; font: 600 12px inherit; cursor: pointer; }
  .trust { display: grid; gap: 6px; margin-bottom: 10px; }
  .perk { display: flex; gap: 8px; align-items: center; justify-content: space-between; padding: 6px 8px; border-radius: 8px; background: rgba(255, 255, 255, 0.04); color: #c9cfdc; font-size: 11.5px; }
  .perk b { color: #ffd27a; font-weight: 600; margin-right: 4px; }
  .perk.on { box-shadow: inset 0 0 0 1px rgba(255, 210, 122, 0.5); }
  .perk.locked { opacity: 0.55; }
  .perk .tag { flex: none; color: #9aa2b6; font-size: 10.5px; }
  .perk.on .tag { color: #ffd27a; }
  .perk button { flex: none; height: 22px; padding: 0 10px; border: 0; border-radius: 999px; background: #e9b03a; color: #1b1405; font: 600 11px inherit; cursor: pointer; }
  .trust .link { justify-self: start; border: 0; background: none; padding: 0; color: #9aa2b6; font: 500 11px inherit; text-decoration: underline; cursor: pointer; }
  .promo button.x { background: none; color: #9aa2b6; padding: 0 6px; }
  .tabs { display: flex; gap: 2px; padding: 0 8px; }
  .tabs button { flex: 1; height: 28px; border: 0; border-radius: 8px; background: none; color: #9aa2b6; font: inherit; cursor: pointer; }
  .tabs button.on { background: rgba(255, 255, 255, 0.08); color: #eef1f7; }
  .p-body { display: flex; flex-direction: column; gap: 7px; padding: 10px 12px 14px; min-height: 104px; }
  .need, .skill, .friend { display: grid; grid-template-columns: 64px 1fr 34px; align-items: center; gap: 8px; }
  .friend { grid-template-columns: 110px 1fr; }
  .friend em { margin-left: 5px; color: #ffd27a; font-style: normal; font-size: 10px; }
  .n-l { color: #cfd5e2; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .n-v { text-align: right; color: #9aa2b6; font: 500 11.5px ui-monospace, monospace; }
  .bar { height: 9px; border-radius: 5px; background: rgba(255, 255, 255, 0.08); overflow: hidden; }
  .fill { display: block; height: 100%; border-radius: 5px; background: linear-gradient(90deg, #3fcf72, #7be495); transition: width 0.6s; }
  .fill.mid { background: linear-gradient(90deg, #e9b03a, #f5cf6b); }
  .fill.low { background: linear-gradient(90deg, #e2543f, #f08a6c); }
  .pips { display: flex; gap: 3px; }
  .pip { flex: 1; height: 8px; border-radius: 2px; background: rgba(255, 255, 255, 0.08); }
  .pip.on { background: #62b6f7; }
  .pip.part { background: linear-gradient(90deg, #62b6f7 calc(var(--p) * 100%), rgba(255, 255, 255, 0.08) 0); }
  .moodlet { display: flex; flex-direction: column; padding: 6px 9px; border-radius: 9px; background: rgba(255, 255, 255, 0.05); border-left: 3px solid #b9c0cc; }
  .moodlet[data-tone='good'] { border-left-color: #4fdc7b; }
  .moodlet[data-tone='bad'] { border-left-color: #ff6b5b; }
  .moodlet span { color: #9aa2b6; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .empty { margin: 0; color: #8a92a6; }

  .pie { position: absolute; z-index: 7; width: 0; height: 0; pointer-events: none; }
  .pie button { position: absolute; left: 0; top: 0; padding: 7px 12px; border: 0; border-radius: 999px; background: rgba(18, 22, 32, 0.9); box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.12), 0 8px 20px -8px rgba(0, 0, 0, 0.6); white-space: nowrap; cursor: pointer; pointer-events: auto; animation: pop 0.18s ease-out both; }
  .pie button:hover, .pie button:focus-visible { background: #2d6bd8; outline: none; }
  .pie-name { position: absolute; transform: translate(-50%, -50%); font: 600 12px Inter, system-ui, sans-serif; color: #fff; text-shadow: 0 1px 3px #000; }
  @keyframes pop { from { opacity: 0; scale: 0.6; } }

  .hud-live { position: absolute; z-index: 6; display: flex; gap: 6px; align-items: center; pointer-events: auto; }
  .hud-live button, .money { height: 30px; padding: 0 12px; border: 0; border-radius: 999px; background: rgba(18, 22, 32, 0.82); box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.08); color: #eef1f7; font: inherit; display: inline-flex; align-items: center; cursor: pointer; }
  .money { cursor: default; color: #7be495; font-family: ui-monospace, monospace; }
  .hud-live button.on { box-shadow: inset 0 0 0 1.5px #ffbf3c; color: #ffd27a; }
  @media (prefers-reduced-motion: reduce) { .pie button { animation: none; } .fill { transition: none; } }
</style>
