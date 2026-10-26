<!--
  The agent sheet. First screen answers: what is it doing (the same sentence as everywhere) and
  why, does it need me, what is its task. Then its own steps, the files it touched (and why),
  its helpers, live output, honest cost, and the full history. Every fact carries its
  provenance: Observed (CLI event) · Inferred (Codly heuristic) · Provided (Codly put it in
  context) · Model-stated (its own words).
-->
<script lang="ts">
  import { tick } from 'svelte';
  import type { Agent } from '../lib/gen/Agent';
  import type { EventRow } from '../lib/gen/EventRow';
  import { copyText, interruptAgent, stopAgent } from '../lib/state/actions';
  import { accounts } from '../lib/state/accounts.svelte';
  import { narrate } from '../lib/state/narration';
  import { store, type FileTouch } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { clock, clockSec, dur, pct, resetsIn, timer, tokens, usd } from '../lib/util/format';
  import { BILLING_LABEL, COMPANY_LABEL, GLOSSARY, idVar, methodProvenance, pastVerb, phrase, prettyModel, PROVIDER_LABEL, TASK_STATUS_LABEL, targetLabel } from '../lib/util/labels';
  import { basename, clean, dirname, oneLine, tildify } from '../lib/util/sanitize';
  import ApprovalCard from '../home/cards/ApprovalCard.svelte';
  import BillingBadge from '../components/BillingBadge.svelte';
  import Icon from '../components/Icon.svelte';
  import Menu, { type MenuItem } from '../components/Menu.svelte';
  import Orbit from '../components/Orbit.svelte';
  import Portrait from '../components/Portrait.svelte';
  import Provenance from '../components/Provenance.svelte';
  import Say from '../components/Say.svelte';
  import Section from '../components/Section.svelte';
  import Why, { type WhyLine } from '../components/Why.svelte';
  import UsageBars from '../components/UsageBars.svelte';
  import TaskChecks from './TaskChecks.svelte';

  interface Props {
    agent: Agent;
  }
  let { agent }: Props = $props();

  const n = $derived(narrate(agent.id));
  const live = $derived.by(() => {
    void store.rev;
    void store.liveRev;
    const l = store.live[agent.id];
    return l ? { ...l } : null;
  });
  const now = $derived.by(() => {
    void store.rev;
    void store.liveRev;
    return store.now(agent.id);
  });
  const task = $derived(agent.task_id ? (store.taskMap.get(agent.task_id) ?? null) : null);
  const approvals = $derived(store.pendingApprovals.filter((a) => a.agent_id === agent.id));
  /** The same card is already on screen in the Lead conversation: point at it instead of repeating it. */
  const inConversation = $derived(ui.room === 'home' && ui.mode !== 'office' && !!approvals.length && approvals.every((a) => store.approvalThread(a) === ui.thread));
  const cli = $derived(store.clis.find((c) => c.provider === agent.provider) ?? null);
  const todo = $derived(live?.todo ?? []);
  const todoDone = $derived(todo.filter((s) => s.status === 'completed').length);
  const step = $derived(todo.findIndex((s) => s.status === 'in_progress'));
  const since = $derived(n?.since ?? live?.statusSince ?? agent.updated_at);

  // ── Why? ──
  const why = $derived.by((): WhyLine[] => {
    const lines: WhyLine[] = [];
    const seg = now?.seg;
    if (seg) {
      const what = seg.target.command ? `: ${oneLine(seg.target.command, 80)}` : seg.target.path ? `: ${seg.target.path}` : '';
      lines.push({ prov: seg.confidence === 'observed' ? 'observed' : 'inferred', text: `${seg.name} call${what}`, meta: clockSec(seg.start) });
      if (seg.confidence !== 'observed') lines.push({ prov: 'inferred', text: `Codly read this ${seg.name} call as “${seg.activity}” from its input.` });
      const f = seg.target.path ? live?.files[seg.target.path] : null;
      if (f) for (const r of f.reasons.filter((r) => r.prov === 'provided' || (r.prov === 'inferred' && r.text.startsWith('Found'))).slice(-2)) lines.push({ prov: r.prov, text: r.text });
    }
    if (step >= 0) lines.push({ prov: 'observed', text: `Step ${step + 1} of ${todo.length} in its own list: ${todo[step].text}` });
    if (task) lines.push({ prov: 'observed', text: `Working on ${task.key} · ${task.title}` });
    const thought = live?.reasoning[live.reasoning.length - 1];
    if (thought) lines.push({ prov: 'model', text: `“${oneLine(thought.text, 220)}”`, meta: clock(thought.ts) });
    return lines;
  });

  // ── decisions ──
  const decisions = $derived.by(() => {
    void store.rev;
    return store.decisions.filter((d) => d.agentId === agent.id || (task && d.taskId === task.id)).slice().reverse();
  });
  const routeWhy = $derived<WhyLine[]>(decisions.filter((d) => d.type === 'route' || d.type === 'assign').map((d) => ({ prov: methodProvenance(d.method), text: d.summary, meta: clock(d.ts) })));

  // ── files ──
  const files = $derived(live ? Object.values(live.files).sort((a, b) => b.lastTs - a.lastTs) : []);
  const fileSummary = $derived.by(() => {
    const read = files.filter((f) => f.read && !f.modified).length;
    const changed = files.filter((f) => f.modified).length;
    const provided = files.filter((f) => f.provided && !f.read && !f.modified).length;
    return [changed && `${changed} changed`, read && `${read} read`, provided && `${provided} provided`].filter(Boolean).join(' · ') || 'none yet';
  });
  function fileReasons(f: FileTouch) {
    const action = [...f.reasons].reverse().find((r) => /^(Created|Edited|Deleted|Read|Changed|Reported)/.test(r.text)) ?? f.reasons[f.reasons.length - 1];
    const origin = [...f.reasons].reverse().find((r) => r.prov === 'provided' || (r.prov === 'inferred' && r.text.startsWith('Found')));
    return [action, origin].filter((r, i, a) => r && a.indexOf(r) === i) as FileTouch['reasons'];
  }
  const fileState = (f: FileTouch) => (f.created ? 'created' : f.deleted ? 'deleted' : f.modified ? 'changed' : f.read ? 'read' : 'provided');

  // ── subagents ──
  const subs = $derived(live?.subagents ?? []);

  // ── cost & context ──
  const u = $derived(agent.usage);
  const ctxFrac = $derived(u.context_used != null && u.context_window ? u.context_used / u.context_window : null);
  const costLine = $derived.by(() => {
    if (u.cost_usd == null) return agent.provider === 'codex' ? 'Tokens only — Codex reports no cost' : 'No cost reported';
    if (agent.billing === 'subscription') return `≈ ${usd(u.cost_usd)} at list price · included in your plan`;
    if (agent.billing === 'api_key') return `${usd(u.cost_usd)} · billed to your API key`;
    if (agent.billing === 'third_party') return `${usd(u.cost_usd)} · billed by the provider`;
    return `≈ ${usd(u.cost_usd)}`;
  });
  const limits = $derived(store.rateLimits.filter((r) => r.provider === agent.provider));
  // Its sign-in: the quota it draws from (labelled only when the CLI has several in use).
  const account = $derived(accounts.of(agent));
  const acctTag = $derived(accounts.tag(agent));
  const WINDOW: Record<string, string> = { five_hour: '5-hour window', seven_day: '7-day window', weekly: 'Weekly window' };

  // ── live output ──
  const liveOut = $derived.by(() => {
    void store.liveRev;
    const seg = now?.seg;
    if (seg) {
      const o = store.liveOut.get(seg.callId);
      if (o) return { title: seg.label || seg.name, text: o.text, live: true, ok: null as boolean | null };
    }
    const lo = live?.lastOutput;
    if (lo) {
      const s = live?.segments.find((x) => x.callId === lo.callId);
      return { title: s ? `${pastVerb(s.activity)} ${s.label}` : 'Last output', text: lo.text, live: false, ok: lo.ok };
    }
    return null;
  });
  let outEl: HTMLPreElement | undefined = $state();
  $effect(() => {
    void liveOut?.text;
    tick().then(() => outEl && (outEl.scrollTop = outEl.scrollHeight));
  });

  // ── history ──
  let filter = $state<'all' | 'tools' | 'files' | 'decisions' | 'errors'>('all');
  let rawOpen = $state<number | null>(null);
  const log = $derived.by(() => {
    void store.rev;
    const out: EventRow[] = [];
    for (let i = store.events.length - 1; i >= 0 && out.length < 250; i--) {
      const e = store.events[i];
      if (e.agent_id !== agent.id && !(task && e.task_id === task.id && e.kind.type === 'decision')) continue;
      const t = e.kind.type;
      if (t === 'usage_updated' || t === 'rate_limit' || t === 'agent_changed') continue;
      if (filter === 'tools' && t !== 'tool_started' && t !== 'tool_completed') continue;
      if (filter === 'files' && t !== 'files_changed' && !(t === 'tool_started' && ['read', 'edit', 'create', 'delete'].includes((e.kind as { activity: string }).activity))) continue;
      if (filter === 'decisions' && t !== 'decision') continue;
      if (filter === 'errors' && t !== 'error' && !(t === 'tool_completed' && !(e.kind as { ok: boolean }).ok)) continue;
      out.push(e);
    }
    return out;
  });
  function describe(e: EventRow): { icon: string; text: string; tone?: 'bad' | 'good' | 'warn' } {
    const k = e.kind;
    switch (k.type) {
      case 'tool_started': return { icon: 'play', text: `${phrase(k.activity, targetLabel(k.target, k.activity))} · ${k.name}` };
      case 'tool_completed': return { icon: k.ok ? 'check' : 'x-circle', text: `${k.ok ? 'Finished' : 'Failed'}${k.exit_code != null ? ` · exit ${k.exit_code}` : ''}${k.tests_passed != null ? ` · ${k.tests_passed} passed${k.tests_failed ? `, ${k.tests_failed} failed` : ''}` : ''}${k.duration_ms != null ? ` · ${dur(k.duration_ms)}` : ''}`, tone: k.ok ? undefined : 'bad' };
      case 'files_changed': return { icon: 'diff', text: `Changed ${k.paths.map(basename).join(', ')}` };
      case 'message': return { icon: 'chat', text: oneLine(k.text, 120) };
      case 'reasoning': return { icon: 'chat', text: `Thought: ${oneLine(k.text, 110)}` };
      case 'turn_started': return { icon: 'arrow-right', text: `Turn started${k.input ? `: ${oneLine(k.input, 80)}` : ''}` };
      case 'turn_completed': return { icon: 'check-circle', text: `Turn ${k.status}${k.duration_ms ? ` · ${dur(k.duration_ms)}` : ''}`, tone: k.status === 'completed' ? undefined : 'warn' };
      case 'status_changed': return { icon: 'refresh', text: `Status → ${k.status.replace('_', ' ')}` };
      case 'plan_updated': return { icon: 'list', text: `Its list: ${k.steps.filter((s) => s.status === 'completed').length} of ${k.steps.length} done` };
      case 'approval_requested': return { icon: 'flag', text: `Asked for approval${k.approval.subject.kind === 'command' ? `: ${oneLine(k.approval.subject.command, 60)}` : ''}`, tone: 'warn' };
      case 'approval_resolved': return { icon: 'flag', text: `Approval ${k.status} by ${k.by === 'user' ? 'you' : k.by}`, tone: k.status === 'approved' ? 'good' : 'bad' };
      case 'retrieval': return { icon: 'link', text: `Codly provided ${k.citations.length} citations for “${oneLine(k.query, 50)}”` };
      case 'session_started': return { icon: 'bolt', text: `Session started · ${PROVIDER_LABEL[k.provider]} ${k.cli_version ?? ''}` };
      case 'sub_agent': return { icon: 'crew', text: `Helper ${k.status}: ${oneLine(k.description, 90)}` };
      case 'error': return { icon: 'alert', text: oneLine(k.message, 120), tone: 'bad' };
      case 'decision': return { icon: 'help', text: k.summary };
      case 'committed': return { icon: 'commit', text: `Committed ${k.sha.slice(0, 7)} · ${k.files.length} files` };
      case 'verified': return { icon: k.ok ? 'check-circle' : 'x-circle', text: `Verified ${k.command}`, tone: k.ok ? 'good' : 'bad' };
      case 'merged': return { icon: 'merge', text: `Merged into ${k.branch}`, tone: 'good' };
      case 'memory_added': return { icon: 'memory', text: `Saved: ${oneLine(k.memory.text, 90)}` };
      case 'artifact_added': return { icon: 'image', text: `Made ${k.artifact.title}` };
      case 'triple_added': return { icon: 'graph', text: `${k.triple.subject.split(':').pop()} ${k.triple.predicate} ${k.triple.object.split(':').pop()}` };
      case 'exited': return { icon: 'stop', text: `Process exited${k.code != null ? ` (${k.code})` : ''}` };
      case 'compacting': return { icon: 'layers', text: `Compacted context${k.pre_tokens ? ` ${tokens(k.pre_tokens)} → ${tokens(k.post_tokens)}` : ''}` };
      case 'retrying': return { icon: 'refresh', text: `Retrying (${k.attempt}): ${k.reason}`, tone: 'warn' };
      case 'chat': return { icon: 'chat', text: oneLine(k.message.text, 100) };
      default: return { icon: 'info', text: k.type.replace(/_/g, ' ') };
    }
  }

  // Jump targets: a timeline click (eventId) opens History; a subagent companion opens Subagents.
  let root: HTMLDivElement | undefined = $state();
  $effect(() => {
    const id = ui.selection?.kind === 'agent' ? ui.selection.eventId : undefined;
    if (!id) return;
    if (!ui.sections.history) ui.toggleSection('history');
    filter = 'all';
    tick().then(() => root?.querySelector(`[data-ev="${id}"]`)?.scrollIntoView({ block: 'center', behavior: ui.reducedMotion ? 'auto' : 'smooth' }));
  });
  $effect(() => {
    const f = ui.sheetFocus;
    if (!f) return;
    ui.sheetFocus = null;
    if (ui.sections[f] === false) ui.toggleSection(f);
    tick().then(() => root?.querySelector(`[data-section="${f}"]`)?.scrollIntoView({ block: 'start', behavior: ui.reducedMotion ? 'auto' : 'smooth' }));
  });

  // ── menu ──
  let moreBtn: HTMLButtonElement | undefined = $state();
  let moreOpen = $state(false);
  let confirmStop = $state(false);
  const working = $derived(n?.tone === 'live' || n?.tone === 'wait');
  const moreItems = $derived<MenuItem[]>([
    { label: 'Open a terminal in its worktree', icon: 'terminal', disabled: !agent.worktree, run: () => { ui.terminalCwd = agent.worktree; ui.go('terminal'); } },
    { label: 'Copy worktree path', icon: 'copy', disabled: !agent.worktree, run: () => copyText(agent.worktree ?? '', 'Path copied') },
    { kind: 'separator' },
    { label: 'Interrupt this turn', icon: 'pause', disabled: !working, run: () => interruptAgent(agent.id) },
    { label: 'Stop the agent…', icon: 'stop', danger: true, disabled: agent.status === 'offline', run: () => (confirmStop = true) },
  ]);
</script>

<div class="as" bind:this={root} style:--hue={idVar(agent.color)}>
  <header class="head">
    {#if n}<Orbit size={64} progress={n.progress} tone={n.tone} stroke={2.5}><Portrait {agent} size={50} /></Orbit>{/if}
    <div class="h-text">
      <h2 class="serif">{agent.name}</h2>
      <p class="h-sub">{agent.role === 'lead' ? 'Lead · ' : agent.role === 'reviewer' ? 'Reviewer · ' : ''}{PROVIDER_LABEL[agent.provider]} ({COMPANY_LABEL[agent.provider]}){#if acctTag}{' · '}<span class="acct" data-tip={account?.identity ? `Runs on the ${acctTag} sign-in · ${account.identity}` : `Runs on the ${acctTag} sign-in`}>{acctTag}</span>{/if}{agent.model ? ` · ${prettyModel(agent.model)}` : ''}</p>
      <div class="h-tags"><BillingBadge billing={agent.billing} plan={account?.plan ?? cli?.plan} /></div>
    </div>
  </header>
  <div class="h-actions">
    <button class="btn sm" onclick={() => ui.openThread(store.threadOf(agent.id))}><Icon name="chat" size={13} />{agent.role === 'lead' ? 'Talk to the Lead' : `Talk to ${agent.name}`}</button>
    {#if working}<button class="btn sm ghost" onclick={() => interruptAgent(agent.id)}><Icon name="pause" size={13} />Interrupt</button>{/if}
    <button class="icon-btn" bind:this={moreBtn} aria-label="More actions" onclick={() => (moreOpen = !moreOpen)}><Icon name="more" /></button>
  </div>
  {#if confirmStop}
    <div class="confirm">
      <span>Stop {agent.name}? The process ends; its worktree is kept.</span>
      <button class="btn sm danger" onclick={() => { confirmStop = false; stopAgent(agent.id); }}>Stop</button>
      <button class="btn sm ghost" onclick={() => (confirmStop = false)}>Keep</button>
    </div>
  {/if}

  <div class="sections">
    <Section id="now" title="Now">
      <p class="now tone-{n?.tone}">{#if n}<Say parts={n.parts} />{/if}</p>
      <p class="now-meta num">{#if !n?.parts.some((p) => p.timer)}{n?.tone === 'live' ? `for ${timer(ui.now - since)}` : `since ${clock(since)}`}{#if agent.worktree}{' · '}{/if}{/if}{#if agent.worktree}<code data-tip={GLOSSARY.worktree}>{task?.branch ?? tildify(agent.worktree)}</code>{/if}</p>
      <div class="why"><Why lines={why} /></div>
    </Section>

    {#if approvals.length}
      <Section id="needs" title="Needs you" tone="attention" collapsible={false}>
        {#if inConversation}
          <div class="needs-ptr">
            <span>{approvals.length === 1 ? 'Its request is open in the conversation.' : `${approvals.length} requests are open in the conversation.`}</span>
            <button class="btn sm amber" onclick={() => (ui.revealApproval = approvals[0].id)}>Show it</button>
          </div>
        {:else}
          <div class="needs">{#each approvals as ap (ap.id)}<ApprovalCard approval={ap} showAgent={false} />{/each}</div>
        {/if}
      </Section>
    {/if}

    <Section id="task" title="Task" summary={task ? `${task.key} · ${task.title}` : 'None'}>
      {#if task}
        <button class="task" onclick={() => ui.selectTask(task.id)}>
          <span class="tk">{task.key}</span><span class="tt">{task.title}</span><span class="ts">{TASK_STATUS_LABEL[task.status]}</span>
        </button>
        {#if task.goal}<p class="goal selectable">{task.goal}</p>{/if}
        <TaskChecks {task} />
        <Why lines={routeWhy} label="Why {agent.name}?" />
      {:else}
        <p class="small">{agent.role === 'lead' ? 'The Lead plans and coordinates; it never takes a task.' : 'No task right now. Assign one from the Plan board.'}</p>
      {/if}
    </Section>

    {#if todo.length}
      <Section id="steps" title="Steps" summary="{todoDone} of {todo.length}">
        {#snippet aside()}<span class="count num">{todoDone} of {todo.length}</span>{/snippet}
        <ol class="todo">
          {#each todo as s, i (i)}
            <li class="t-{s.status}"><span class="tb">{#if s.status === 'completed'}<Icon name="check" size={10} stroke={2.4} />{/if}</span><span class="tx">{s.text}</span></li>
          {/each}
        </ol>
        <p class="src"><Provenance kind="observed" /> its own todo list</p>
      </Section>
    {/if}

    <Section id="files" title="Files" summary={fileSummary}>
      {#snippet aside()}<span class="count">{fileSummary}</span>{/snippet}
      {#if files.length}
        <ul class="files">
          {#each files as f (f.path)}
            <li>
              <button class="file f-{fileState(f)}" onpointerenter={() => ui.setHover({ path: f.path, agentId: agent.id })} onpointerleave={() => ui.setHover(null)} onclick={() => ui.selectFile(f.path)}>
                <span class="f-path truncate"><span class="f-dir">{dirname(f.path) ? dirname(f.path) + '/' : ''}</span>{basename(f.path)}</span>
                <span class="f-state">{fileState(f)}</span>
              </button>
              {#each fileReasons(f) as r, ri (ri)}<span class="f-why truncate" data-tip={r.text}><Provenance kind={r.prov} compact /> {r.text}</span>{/each}
            </li>
          {/each}
        </ul>
      {:else}
        <p class="small">No files touched yet.</p>
      {/if}
    </Section>

    {#if subs.length}
      <Section id="subagents" title="Helpers" summary={String(subs.length)}>
        <ul class="subs">
          {#each subs as s (s.callId)}
            <li>
              <span class="s-dot" class:on={s.status === 'running'}></span>
              <div class="s-txt"><p class="selectable">{s.description}</p><p class="s-meta num">{s.status === 'running' ? `working · ${timer(ui.now - s.startedAt)}` : `${s.status} · ${dur(s.ts - s.startedAt)}`}</p></div>
              <Provenance kind="observed" compact />
            </li>
          {/each}
        </ul>
      </Section>
    {/if}

    <Section id="output" title="Output" summary={liveOut ? (liveOut.live ? 'live' : liveOut.title) : 'none'}>
      {#if liveOut}
        <div class="out-head">{#if liveOut.live}<span class="live-dot"></span>{:else}<Icon name={liveOut.ok === false ? 'x-circle' : 'check'} size={12} />{/if}<code class="truncate">{liveOut.title}</code></div>
        <pre class="out selectable" bind:this={outEl}>{clean(liveOut.text)}</pre>
      {:else}
        <p class="small">No command output yet.</p>
      {/if}
    </Section>

    <Section id="cost" title="Cost & context" summary="{tokens(u.input_tokens + u.output_tokens)} tokens{ctxFrac != null ? ` · ${pct(ctxFrac)} of context` : ''}">
      <div class="cost">
        <p class="price">{costLine}</p>
        <div class="kv">
          <span data-tip="Input tokens this session">in <b class="num">{tokens(u.input_tokens)}</b></span>
          <span data-tip="Output tokens this session">out <b class="num">{tokens(u.output_tokens)}</b></span>
          <span data-tip={GLOSSARY.cache}>cache <b class="num">{tokens(u.cache_read_tokens)}</b></span>
        </div>
        {#if ctxFrac != null}
          <div class="gauge" data-tip={GLOSSARY.context}>
            <div class="g-bar"><i style:width="{Math.min(100, ctxFrac * 100)}%" class:near={ctxFrac >= 0.8}></i></div>
            <span class="num">{tokens(u.context_used)} of {tokens(u.context_window)} context in the last turn</span>
          </div>
        {/if}
        {#if account?.limits.length}
          <div class="quota">
            {#if acctTag}<p class="q-head">Quota of its <b>{acctTag}</b> sign-in{account.identity ? ` · ${account.identity}` : ''}</p>{/if}
            <UsageBars limits={account.limits} now={ui.now} />
          </div>
        {:else}
          {#each limits as r (r.window)}
            <p class="limit num">{WINDOW[r.window] ?? r.window}: {pct(r.usedFraction)} used{r.resetsAt ? ` · resets ${resetsIn(r.resetsAt, ui.now)}` : ''}</p>
          {/each}
        {/if}
        <p class="src"><Provenance kind="observed" /> {BILLING_LABEL[agent.billing]} · as reported by {PROVIDER_LABEL[agent.provider]}</p>
      </div>
    </Section>

    <Section id="history" title="History" summary="{log.length} events">
      <div class="filters lenses" role="radiogroup" aria-label="Filter">
        {#each ['all', 'tools', 'files', 'decisions', 'errors'] as f (f)}
          <button role="radio" aria-checked={filter === f} onclick={() => (filter = f as typeof filter)}>{f[0].toUpperCase() + f.slice(1)}</button>
        {/each}
      </div>
      <div class="log">
        {#each log as e (e.id)}
          {@const d = describe(e)}
          <button class="ev" class:hl={ui.selection?.kind === 'agent' && ui.selection.eventId === e.id} data-ev={e.id} onclick={() => (rawOpen = rawOpen === e.id ? null : e.id)}>
            <time class="num">{clockSec(e.ts)}</time>
            <span class="ev-ic tone-{d.tone ?? 'n'}"><Icon name={d.icon} size={12} /></span>
            <span class="ev-t truncate">{d.text}</span>
            {#if e.kind.type === 'decision'}<Provenance kind={methodProvenance(e.kind.method)} compact />{/if}
          </button>
          {#if rawOpen === e.id}<pre class="raw selectable">{JSON.stringify(e, null, 2)}</pre>{/if}
        {:else}
          <p class="small">Nothing yet.</p>
        {/each}
      </div>
    </Section>
  </div>
</div>

{#if moreOpen && moreBtn}<Menu anchor={moreBtn} items={moreItems} onclose={() => (moreOpen = false)} align="end" width={250} />{/if}

<style>
  .as { display: flex; flex-direction: column; min-height: 0; height: 100%; overflow-y: auto; }
  .head { display: flex; gap: 16px; align-items: center; padding: 24px 22px 10px; background: radial-gradient(120% 120% at 0% 0%, color-mix(in srgb, var(--hue) 14%, transparent), transparent 60%); }
  .h-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; padding-right: 30px; }
  h2 { font-size: var(--d-lg); line-height: 1; }
  .h-sub { font-size: var(--t-sm); color: var(--ink-3); }
  .h-tags { margin-top: 6px; }
  .h-actions { display: flex; align-items: center; gap: 6px; padding: 6px 22px 12px; }
  .h-actions .icon-btn { margin-left: auto; }
  .confirm { display: flex; align-items: center; gap: 6px; margin: 0 22px 12px; padding: 10px 12px; border-radius: var(--r-md); background: var(--bad-soft); font-size: var(--t-sm); }
  .confirm span { flex: 1; }
  .now { font-size: 16px; line-height: 24px; color: var(--ink-1); overflow-wrap: anywhere; }
  .now.tone-wait { color: var(--amber-ink); }
  .now-meta { margin-top: 2px; font-size: var(--t-sm); color: var(--ink-3); overflow-wrap: anywhere; }
  .now-meta code { font-size: 11px; color: var(--ink-2); }
  .why { margin-top: 8px; }
  .needs { display: flex; flex-direction: column; gap: 10px; }
  .needs-ptr { display: flex; align-items: center; gap: 10px; font-size: var(--t-md); color: var(--amber-ink); }
  .needs-ptr span { flex: 1; }
  .task { display: flex; align-items: center; gap: 8px; width: 100%; padding: 10px 12px; border-radius: var(--r-md); background: var(--surface-1); text-align: left; }
  .task:hover { background: var(--surface-2); }
  .tk { font-family: var(--font-mono); font-size: 11.5px; color: var(--ink-3); }
  .tt { flex: 1; min-width: 0; font-weight: var(--w-med); }
  .ts { font-size: var(--t-xs); color: var(--ink-3); white-space: nowrap; }
  .goal { margin: 10px 0 2px; font-size: var(--t-md); line-height: var(--lh-md); color: var(--ink-2); }
  .small { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .count { font-size: var(--t-sm); color: var(--ink-3); white-space: nowrap; }
  .todo { display: flex; flex-direction: column; gap: 7px; }
  .todo li { display: flex; gap: 9px; align-items: flex-start; font-size: var(--t-md); line-height: 19px; color: var(--ink-2); }
  .tb { width: 15px; height: 15px; margin-top: 2px; border-radius: 50%; flex-shrink: 0; box-shadow: inset 0 0 0 1.5px var(--line-3); display: grid; place-items: center; color: var(--night); }
  .t-completed .tb { background: var(--ok); box-shadow: none; }
  .t-completed .tx { color: var(--ink-3); }
  .t-in_progress .tb { box-shadow: inset 0 0 0 1.5px var(--live); background: radial-gradient(circle, var(--live) 0 3px, transparent 3.5px); }
  .t-in_progress .tx { color: var(--ink-1); font-weight: var(--w-med); }
  .src { margin-top: 12px; display: flex; align-items: center; gap: 6px; font-size: var(--t-xs); color: var(--ink-3); }
  .files { display: flex; flex-direction: column; gap: 6px; }
  .files li { display: flex; flex-direction: column; gap: 2px; }
  .file { display: flex; align-items: center; gap: 8px; width: 100%; padding: 3px 6px; margin: 0 -6px; border-radius: var(--r-sm); text-align: left; }
  .file:hover { background: var(--surface-2); }
  .f-path { flex: 1; font-family: var(--font-mono); font-size: 11.5px; color: var(--ink-1); }
  .f-dir { color: var(--ink-3); }
  .f-state { font-size: var(--t-xs); color: var(--ink-3); }
  .f-changed .f-state, .f-created .f-state { color: var(--hue); }
  .f-why { display: flex; align-items: center; gap: 5px; font-size: var(--t-xs); color: var(--ink-3); }
  .subs { display: flex; flex-direction: column; gap: 10px; }
  .subs li { display: flex; gap: 10px; align-items: flex-start; }
  .s-dot { width: 9px; height: 9px; margin-top: 5px; border-radius: 50%; background: var(--ink-4); flex-shrink: 0; }
  .s-dot.on { background: color-mix(in srgb, var(--hue) 75%, white); box-shadow: 0 0 8px var(--hue); }
  .s-txt { flex: 1; min-width: 0; font-size: var(--t-md); line-height: var(--lh-md); }
  .s-meta { font-size: var(--t-xs); color: var(--ink-3); }
  .out-head { display: flex; align-items: center; gap: 6px; margin-bottom: 6px; font-size: var(--t-sm); color: var(--ink-3); }
  .out-head code { font-size: 11.5px; color: var(--ink-2); }
  .live-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--aurora); box-shadow: 0 0 0 3px var(--live-soft); }
  .out { margin: 0; max-height: 240px; overflow: auto; padding: 10px 12px; border-radius: var(--r-md); background: var(--code-bg); font-family: var(--font-mono); font-size: 11.5px; line-height: 17px; color: var(--ink-2); white-space: pre-wrap; word-break: break-word; }
  .cost { display: flex; flex-direction: column; gap: 10px; }
  .price { font-size: var(--t-md); color: var(--ink-1); }
  .kv { display: flex; gap: 16px; font-size: var(--t-sm); color: var(--ink-3); }
  .kv b { color: var(--ink-1); font-weight: var(--w-med); margin-left: 3px; }
  .gauge { display: flex; flex-direction: column; gap: 6px; font-size: var(--t-sm); color: var(--ink-2); }
  .g-bar { height: 5px; border-radius: 3px; background: var(--surface-3); overflow: hidden; }
  .g-bar i { display: block; height: 100%; border-radius: 3px; background: var(--ink-3); transition: width var(--d-4) var(--ease-out); }
  .g-bar i.near { background: var(--amber); }
  .limit { font-size: var(--t-sm); color: var(--ink-3); }
  .acct { color: var(--ink-2); font-weight: var(--w-med); }
  .quota { display: flex; flex-direction: column; gap: 7px; }
  .q-head { font-size: var(--t-sm); color: var(--ink-3); }
  .q-head b { color: var(--ink-1); font-weight: var(--w-med); }
  .filters { margin-bottom: 10px; }
  .log { display: flex; flex-direction: column; max-height: 380px; overflow-y: auto; margin: 0 -8px; }
  .ev { display: flex; align-items: center; gap: 8px; height: 26px; padding: 0 8px; border-radius: var(--r-sm); font-size: var(--t-sm); text-align: left; color: var(--ink-2); }
  .ev:hover { background: var(--surface-1); }
  .ev.hl { background: var(--surface-3); color: var(--ink-1); }
  .ev time { font-size: var(--t-xs); color: var(--ink-3); width: 64px; flex-shrink: 0; }
  .ev-ic { color: var(--ink-3); display: grid; }
  .tone-bad { color: var(--bad); }
  .tone-good { color: var(--ok); }
  .tone-warn { color: var(--amber); }
  .ev-t { flex: 1; }
  .raw { margin: 2px 8px 6px; padding: 8px 10px; max-height: 220px; overflow: auto; border-radius: var(--r-md); background: var(--code-bg); font-size: 11px; line-height: 15px; color: var(--ink-2); }
</style>
