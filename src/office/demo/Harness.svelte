<script lang="ts">
  import { onDestroy } from 'svelte';
  import OfficeView, { type WorldSelection } from '../OfficeView.svelte';
  import type { Agent } from '../../lib/gen/Agent';
  import type { Approval } from '../../lib/gen/Approval';
  import type { Artifact } from '../../lib/gen/Artifact';
  import type { EventRow } from '../../lib/gen/EventRow';
  import type { Task } from '../../lib/gen/Task';
  import { agentPortrait } from '../portrait';
  import { AGENTS, TASKS, buildScript, themePreviewImage } from './script';
  import type { JobOffer } from '../../lib/gen/JobOffer';
  import { makeApplicants } from '../../lib/util/career';
  import { TEMPLATES } from '../layout';

  // Options from the URL (see demo.ts).
  const params = new URLSearchParams(location.search);
  const initialTheme: 'dark' | 'light' = params.get('theme') === 'light' ? 'light' : 'dark';
  const skip = Math.max(0, Number(params.get('skip') ?? 0) || 0);
  const speed = Math.max(0.1, Number(params.get('speed') ?? 1) || 1);
  const paused = params.has('paused');
  const initialBackdrop = params.has('backdrop');
  const initialSelect = params.get('select');
  // ?lot=garage or ?lot=small-office: another ready-made office (the Studio by default).
  const lot = TEMPLATES[params.get('lot') as keyof typeof TEMPLATES] ?? TEMPLATES.studio;

  let agents = $state<Agent[]>(AGENTS.map((a) => ({ ...a })));
  let tasks = $state<Task[]>(TASKS.map((t) => ({ ...t })));
  let approvals = $state<Approval[]>([]);
  let artifacts = $state<Artifact[]>([]);
  let selection = $state<WorldSelection | null>(initialSelect ? { agentId: initialSelect } : null);
  let hover = $state<WorldSelection | null>(null);
  const theme = initialTheme;
  // The office follows the real clock; ?hour=21.5 (or the button) pins an hour.
  const HOURS = [null, 9, 18.5, 23];
  let hour = $state<number | null>(params.has('hour') ? Number(params.get('hour')) : null);
  let backdrop = $state(initialBackdrop);
  let reducedMotion = $state(params.has('rm'));
  let idleReport = $state('');
  const preview = themePreviewImage();

  const listeners = new Set<(ev: EventRow) => void>();
  const subscribe = (cb: (ev: EventRow) => void) => {
    listeners.add(cb);
    return () => listeners.delete(cb);
  };

  // A tiny app store: what the real app keeps from the same events.
  function apply(ev: EventRow) {
    const k = ev.kind;
    switch (k.type) {
      case 'agent_changed':
        agents = agents.map((a) => (a.id === k.agent.id ? { ...k.agent, updated_at: ev.ts } : a));
        break;
      case 'status_changed':
        agents = agents.map((a) => (a.id === ev.agent_id ? { ...a, status: k.status, updated_at: ev.ts } : a));
        break;
      case 'task_changed':
        tasks = tasks.some((t) => t.id === k.task.id) ? tasks.map((t) => (t.id === k.task.id ? { ...k.task, updated_at: ev.ts } : t)) : [...tasks, { ...k.task, updated_at: ev.ts }];
        break;
      case 'approval_requested':
        approvals = [...approvals.filter((a) => a.id !== k.approval.id), { ...k.approval, created_at: ev.ts }];
        break;
      case 'approval_resolved':
        approvals = approvals.map((a) => (a.id === k.approval_id ? { ...a, status: k.status } : a));
        break;
      case 'artifact_added':
        artifacts = [...artifacts, { ...k.artifact, created_at: ev.ts }];
        break;
    }
  }

  const steps = buildScript();
  const base = Date.now() - skip * 1000;
  let id = 0;
  const history: EventRow[] = [];
  for (const s of steps) {
    if (s.t >= skip) break;
    const ev: EventRow = { id: ++id, ts: Math.round(base + s.t * 1000), agent_id: s.agent, task_id: s.task, run_id: null, kind: s.kind };
    history.push(ev);
    apply(ev);
  }
  const startedAt = Date.now();
  (window as unknown as { __demoTime: () => number }).__demoTime = () => skip + ((Date.now() - startedAt - 900) / 1000) * speed;
  const timers: number[] = [];
  if (!paused) {
    for (const s of steps) {
      if (s.t < skip) continue;
      const delay = ((s.t - skip) * 1000) / speed + 900;
      timers.push(
        window.setTimeout(() => {
          const ev: EventRow = { id: ++id, ts: Date.now(), agent_id: s.agent, task_id: s.task, run_id: null, kind: s.kind };
          apply(ev);
          for (const l of listeners) l(ev);
        }, delay),
      );
    }
  }
  onDestroy(() => timers.forEach((t) => clearTimeout(t)));

  // ?offer: a job offer goes up by the door after 2 s (pick one, or the first is hired at 30 s).
  let offer: JobOffer | null = null;
  const send = (o: JobOffer) => {
    const ev: EventRow = { id: ++id, ts: Date.now(), agent_id: null, task_id: o.task_id, run_id: null, kind: { type: 'job_offer', offer: o } };
    for (const l of listeners) l(ev);
  };
  const closeOffer = (i: number, byYou: boolean) => {
    if (!offer || offer.hired != null) return;
    offer = { ...offer, hired: i, by_you: byYou };
    send(offer);
  };
  if (params.has('offer'))
    timers.push(
      window.setTimeout(() => {
        const taken = agents.map((a) => a.name);
        offer = { id: 'offer-1', task_id: 'demo-task', title: 'Backend', provider: 'claude', model: 'sonnet', applicants: makeApplicants(3, taken, agents.map((a) => a.color)), posted_at: Date.now(), closes_at: Date.now() + 30_000, hired: null, by_you: false };
        send(offer);
        timers.push(window.setTimeout(() => closeOffer(0, false), 30_000));
      }, 2000),
    );

  function measureIdle() {
    const w = window as unknown as { __raf: number; __timeouts: number };
    const start = w.__raf;
    const t0 = w.__timeouts;
    idleReport = 'measuring 10 s…';
    window.setTimeout(() => {
      idleReport = `${w.__raf - start} rAF · ${w.__timeouts - t0 - 1} timers in 10 s`;
    }, 10_000);
  }

  $effect(() => {
    document.documentElement.dataset.theme = theme;
  });

  const describe = (s: WorldSelection | null) => {
    if (!s) return 'nothing selected';
    if (s.agentId) return `agent · ${agents.find((a) => a.id === s.agentId)?.name ?? s.agentId}`;
    if (s.taskId) return `task · ${tasks.find((t) => t.id === s.taskId)?.key ?? s.taskId}`;
    return `file · ${s.path}`;
  };
  const portraits = $derived(agents.map((a) => ({ a, src: agentPortrait(a, 26) })));
</script>

<div class="stage">
  <OfficeView
    {agents}
    {tasks}
    {approvals}
    {artifacts}
    {selection}
    {hover}
    {hour}
    {lot}
    {reducedMotion}
    {backdrop}
    {subscribe}
    {history}
    onselect={(s) => (selection = s)}
    onhover={(h) => (hover = h)}
    onpickapplicant={(_, i) => closeOffer(i, true)}
    onlook={(id, seed) => (agents = agents.map((a) => (a.id === id ? { ...a, look_seed: seed } : a)))}
    artifactUrl={(a) => (a.id === 'art-1' ? preview : null)}
  />
  {#if backdrop}
    <div class="column" aria-hidden="true">
      <div class="glass convo">The Lead conversation sits here; the office stays legible around it.</div>
    </div>
  {/if}
  <div class="bar glass">
    <strong>Codly · office</strong>
    <button onclick={() => (hour = HOURS[(HOURS.indexOf(hour) + 1) % HOURS.length])}>{hour == null ? 'Real clock' : `${Math.floor(hour)}:${String(Math.round((hour % 1) * 60)).padStart(2, '0')}`}</button>
    <button aria-pressed={backdrop} onclick={() => (backdrop = !backdrop)}>Backdrop</button>
    <button aria-pressed={reducedMotion} onclick={() => (reducedMotion = !reducedMotion)}>Reduce motion</button>
    <button onclick={measureIdle}>Measure idle</button>
    {#if idleReport}<span class="muted">{idleReport}</span>{/if}
    <span class="muted">{describe(selection)}</span>
    <span class="portraits">
      {#each portraits as p (p.a.id)}
        <button class="pt" title={p.a.name} onclick={() => (selection = { agentId: p.a.id })}
          >{#if p.src}<img alt={p.a.name} src={p.src} width="26" height="26" />{/if}</button
        >
      {/each}
    </span>
  </div>
</div>

<style>
  .stage {
    position: fixed;
    inset: 0;
  }
  .glass {
    background: rgba(24, 20, 30, 0.74);
    color: #f3ede4;
    box-shadow:
      0 0 0 1px rgba(255, 240, 220, 0.08),
      0 10px 30px -10px rgba(0, 0, 0, 0.6);
    backdrop-filter: blur(14px);
    -webkit-backdrop-filter: blur(14px);
  }
  :global([data-theme='light']) .glass {
    background: rgba(255, 252, 246, 0.82);
    color: #2a2420;
    box-shadow:
      0 0 0 1px rgba(60, 40, 20, 0.08),
      0 10px 30px -14px rgba(80, 56, 24, 0.3);
  }
  .bar {
    position: absolute;
    top: 12px;
    left: 12px;
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 6px 8px 6px 12px;
    border-radius: 12px;
    font: 500 12px/18px 'Inter Variable', system-ui, sans-serif;
  }
  .bar button {
    font: inherit;
    color: inherit;
    background: rgba(255, 255, 255, 0.08);
    border: 0;
    border-radius: 7px;
    padding: 3px 9px;
    cursor: pointer;
  }
  .bar button[aria-pressed='true'] {
    background: rgba(255, 190, 120, 0.3);
  }
  :global([data-theme='light']) .bar button {
    background: rgba(60, 40, 20, 0.07);
  }
  .muted {
    opacity: 0.6;
  }
  .portraits {
    display: flex;
    gap: 3px;
  }
  .bar .pt {
    padding: 0;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    background: transparent;
  }
  .pt img {
    display: block;
    border-radius: 50%;
  }
  .column {
    position: absolute;
    left: 50%;
    top: 64px;
    bottom: 24px;
    width: 640px;
    margin-left: -320px;
    display: flex;
    flex-direction: column;
    pointer-events: none;
  }
  .convo {
    flex: 1;
    border-radius: 18px;
    padding: 24px;
    font: 400 15px/24px 'Inter Variable', system-ui, sans-serif;
    opacity: 0.95;
  }
</style>
