<!--
  An approval, inline where it happened. Exactly what will run and where, why (the agent's
  words), inferred risks, and only the choices the provider offered. The oldest visible request
  takes the keyboard: ⌘↵ once · ⌥⌘↵ session · ⌘⌫ deny · ⇧⌘⌫ deny with a note.
  Questions (AskUserQuestion / requestUserInput) render as a choice to answer.
  A plan (Claude's plan mode asks to leave it with `ExitPlanMode`) renders as the plan itself:
  accept it, or send it back with a note.
  The same command asked by several agents is one card (`twins`): one answer goes to all.
  "Always allow in this repo" saves a rule in Codly for every agent of the repo; the card
  offers it only when the core says the request can be saved (never for what Codly always asks).
-->
<script lang="ts">
  import { tick } from 'svelte';
  import type { Approval } from '../../lib/gen/Approval';
  import type { ApprovalChoice } from '../../lib/gen/ApprovalChoice';
  import type { ApprovalRuleKind } from '../../lib/gen/ApprovalRuleKind';
  import type { ApprovalRuleOffer } from '../../lib/gen/ApprovalRuleOffer';
  import type { JsonValue } from '../../lib/gen/serde_json/JsonValue';
  import { api } from '../../lib/api';
  import { copyText, resolveApprovals, saveApprovalRule } from '../../lib/state/actions';
  import { yourAnswers } from '../../lib/state/answers';
  import { keyTargetApproval } from '../../lib/state/keys';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { clock, timer } from '../../lib/util/format';
  import { idVar } from '../../lib/util/labels';
  import { outcomeBy, outcomeWord } from '../../lib/util/outcome';
  import { oneLine, tildify } from '../../lib/util/sanitize';
  import Icon from '../../components/Icon.svelte';
  import Markdown from '../../components/Markdown.svelte';
  import Portrait from '../../components/Portrait.svelte';
  import Provenance from '../../components/Provenance.svelte';
  import Why, { type WhyLine } from '../../components/Why.svelte';
  import { PERMISSIONS } from '../LeadControls.svelte';

  interface Props {
    approval: Approval;
    showAgent?: boolean;
    /** Other agents' requests that wait for the same answer: this card answers them too. */
    twins?: Approval[];
  }
  let { approval, showAgent = true, twins = [] }: Props = $props();

  const agent = $derived(store.agentMap.get(approval.agent_id) ?? null);
  /** Who else asked the same, by name. */
  const others = $derived([...new Set(twins.filter((t) => t.agent_id !== approval.agent_id).map((t) => store.agentMap.get(t.agent_id)?.name ?? 'an agent'))]);
  const task = $derived(approval.task_id ? (store.taskMap.get(approval.task_id) ?? null) : null);
  const pending = $derived(approval.status === 'pending');
  const subject = $derived(approval.subject);
  const offered = $derived(new Set(approval.choices));
  const isKeyTarget = $derived(pending && keyTargetApproval()?.id === approval.id);

  let noteOpen = $state(false);
  let note = $state('');
  let busy = $state(false);
  let noteEl: HTMLInputElement | undefined = $state();
  let el: HTMLElement | undefined = $state();

  /** The plan a Claude agent in plan mode shows you before it changes anything. */
  const plan = $derived.by(() => {
    if (subject.kind !== 'tool' || subject.name !== 'ExitPlanMode') return null;
    const input = subject.input as { plan?: unknown } | null;
    return typeof input?.plan === 'string' ? input.plan : null;
  });

  const headline = $derived.by(() => {
    const who = agent?.name ?? 'An agent';
    if (plan !== null) return `${who} shows you its plan`;
    switch (subject.kind) {
      case 'command':
        if (others.length) return `${who} and ${others.length === 1 ? others[0] : `${others.length} others`} want to run the same command`;
        return `${who} wants to run a command`;
      case 'file_change': return `${who} wants to change ${subject.paths.length === 1 ? 'a file' : `${subject.paths.length} files`}`;
      case 'tool': return `${who} wants to use ${oneLine(subject.name, 40)}`;
      case 'question': return `${who} asks you`;
      default: return `${who} asks for permission`;
    }
  });

  const inside = $derived(subject.kind === 'command' && subject.cwd && agent?.worktree ? subject.cwd.startsWith(agent.worktree) : null);

  const why = $derived.by((): WhyLine[] => {
    const lines: WhyLine[] = [];
    if (task) lines.push({ prov: 'observed', text: `Task ${task.key} · ${task.title}` });
    void store.rev;
    const todo = agent ? (store.live[agent.id]?.todo ?? []) : [];
    const at = todo.findIndex((s) => s.status === 'in_progress');
    if (at >= 0) lines.push({ prov: 'observed', text: `Step ${at + 1} of ${todo.length}: ${oneLine(todo[at].text, 90)}` });
    if (subject.kind === 'command' && /\brm\s+-\w*r|\bsudo\b|git\s+push|curl[^|]*\|\s*(ba|z)?sh/.test(subject.command)) {
      // The mode by the name Settings and the pickers give it (the pickers do not offer Full access).
      const mode = store.settings.approval_mode;
      const label = mode === 'full' ? 'Full access' : (PERMISSIONS.find((p) => p.id === mode)?.label ?? mode);
      lines.push({ prov: 'inferred', text: `Codly always asks about this kind of command in ${label} mode.` });
    }
    return lines;
  });

  const CHOICE: Record<ApprovalChoice, { label: string; kbd?: string; cls: string }> = {
    allow_once: { label: 'Approve once', kbd: '⌘↵', cls: 'primary' },
    allow_session: { label: 'Allow for this session', kbd: '⌥⌘↵', cls: '' },
    allow_always: { label: 'Always allow', cls: '' },
    deny: { label: 'Deny', kbd: '⌘⌫', cls: 'ghost' },
    cancel: { label: 'Stop the turn', cls: 'danger' },
  };
  const ORDER: ApprovalChoice[] = ['allow_once', 'allow_session', 'allow_always', 'deny', 'cancel'];
  /** A plan is accepted or rejected, not "approved once". */
  const choiceLabel = (c: ApprovalChoice) => (plan === null ? CHOICE[c].label : c === 'allow_once' ? 'Accept the plan' : c === 'deny' ? 'Reject the plan' : CHOICE[c].label);

  async function decide(choice: ApprovalChoice, message: string | null = null, answers: JsonValue | null = null) {
    if (!pending || busy || !offered.has(choice)) return;
    busy = true;
    await resolveApprovals([approval, ...twins].map((a) => a.id), choice, message, answers);
    busy = false;
    noteOpen = false;
  }

  // ── "Always allow in this repo" ──
  // What the core lets this request be saved as. It records that when its rules decide to ask
  // you, which can be a moment after the card shows: read it again once that decision is in.
  let ruleOffer = $state<ApprovalRuleOffer | null>(null);
  const rulePrefix = $derived(ruleOffer?.prefix ?? null);
  const ruleExact = $derived(ruleOffer?.exact ?? null);
  const brokerAsked = $derived.by(() => {
    void store.rev;
    return store.decisions.some((d) => d.type === 'approval_policy' && (d.detail as { approval_id?: unknown } | null)?.approval_id === approval.id);
  });
  $effect(() => {
    if (!pending || subject.kind !== 'command') return;
    void brokerAsked;
    let stale = false;
    api.approvalRuleOffer(approval.id).then(
      (o) => {
        if (!stale) ruleOffer = o;
      },
      () => {
        /* the choice stays hidden: the other answers work without it */
      },
    );
    return () => {
      stale = true;
    };
  });
  async function saveRule(kind: ApprovalRuleKind) {
    if (!pending || busy) return;
    busy = true;
    await saveApprovalRule(approval.id, kind);
    busy = false;
  }
  function openNote() {
    if (!offered.has('deny')) return;
    noteOpen = true;
    tick().then(() => noteEl?.focus());
  }
  $effect(() => {
    if (ui.denyNoteFor === approval.id) {
      ui.denyNoteFor = null;
      openNote();
    }
  });
  $effect(() => {
    if (el && (ui.revealApproval === approval.id || twins.some((t) => t.id === ui.revealApproval))) {
      ui.revealApproval = null;
      el.scrollIntoView({ block: 'center', behavior: ui.reducedMotion ? 'auto' : 'smooth' });
    }
  });

  // ── questions ──
  type Q = { question: string; header?: string; options: { label: string; description?: string }[]; multiSelect?: boolean };
  const questions = $derived.by((): Q[] => {
    if (subject.kind !== 'question') return [];
    const raw = subject.questions as unknown;
    const arr = Array.isArray(raw) ? raw : raw && typeof raw === 'object' && Array.isArray((raw as { questions?: unknown }).questions) ? (raw as { questions: unknown[] }).questions : [];
    return arr
      .filter((q): q is Record<string, unknown> => !!q && typeof q === 'object')
      .map((q) => ({
        question: String(q.question ?? q.prompt ?? ''),
        header: q.header ? String(q.header) : undefined,
        options: Array.isArray(q.options) ? q.options.map((o) => (typeof o === 'string' ? { label: o } : { label: String((o as Record<string, unknown>).label ?? ''), description: (o as Record<string, unknown>).description as string | undefined })) : [],
        multiSelect: !!q.multiSelect,
      }));
  });
  let picks = $state<Record<number, string[]>>({});
  let freeText = $state('');
  function toggle(qi: number, label: string, multi: boolean) {
    const cur = picks[qi] ?? [];
    picks = { ...picks, [qi]: multi ? (cur.includes(label) ? cur.filter((l) => l !== label) : [...cur, label]) : [label] };
  }
  function submitAnswers() {
    const answers: Record<string, JsonValue> = {};
    questions.forEach((q, i) => (answers[q.question] = q.multiSelect ? (picks[i] ?? []) : (picks[i]?.[0] ?? '')));
    if (!questions.length) answers.answer = freeText;
    decide(offered.has('allow_once') ? 'allow_once' : approval.choices[0], null, answers);
  }
  const unanswered = $derived(questions.some((q, i) => q.options.length > 0 && !picks[i]?.length));

  // A question is answered or skipped, with what you answered; only a permission is approved or denied.
  const outcome = $derived(outcomeWord(approval.status, approval));
  const by = $derived(outcomeBy(approval.decided_by));
  const answer = $derived(subject.kind === 'question' && approval.status === 'approved' ? (yourAnswers.get(approval.id) ?? null) : null);
  const what = $derived(subject.kind === 'command' ? subject.command : subject.kind === 'question' ? (questions[0]?.question ?? 'a question') : subject.kind === 'file_change' ? subject.paths.join(', ') : subject.kind === 'tool' ? (plan !== null ? 'its plan' : subject.name) : 'permissions');
</script>

{#if !pending}
  <div class="resolved s-{approval.status}">
    <Icon name={approval.status === 'approved' ? 'check' : 'x'} size={12} stroke={2} />
    <span class="r-l">{outcome} {by}</span>
    {#if agent}<span class="r-who">{agent.name}</span>{/if}
    <code class="truncate">{oneLine(what, 120)}</code>
    {#if answer}<span class="r-a truncate" data-tip={answer}>“{oneLine(answer, 80)}”</span>{/if}
    <time class="num">{clock(approval.created_at)}</time>
  </div>
{:else}
  <article class="card" class:key={isKeyTarget} bind:this={el} style:--hue={idVar(agent?.color ?? 0)} aria-label={headline}>
    <header>
      {#if showAgent && agent}<button class="who" onclick={() => ui.selectAgent(agent.id)}><Portrait {agent} size={24} /></button>{/if}
      <h3>{headline}{#if subject.kind === 'question' && questions[0]?.header}<span class="qh">{questions[0].header}</span>{/if}</h3>
      <span class="since num" data-tip="Waiting since {clock(approval.created_at)}">{timer(ui.now - approval.created_at)}</span>
    </header>

    {#if subject.kind === 'command'}
      <div class="cmd selectable">
        <span class="dollar">$</span><code>{subject.command}</code>
        <button class="icon-btn sm copy" aria-label="Copy command" data-tip="Copy" onclick={() => copyText(subject.command, 'Command copied')}><Icon name="copy" size={13} /></button>
      </div>
      <div class="meta">
        {#if subject.cwd}<code class="cwd truncate" data-tip={subject.cwd}>{tildify(subject.cwd)}</code>{/if}
        {#if inside === true}<span class="ok"><Icon name="check" size={11} stroke={2.2} />inside {agent?.name}’s worktree</span>
        {:else if inside === false}<span class="bad"><Icon name="alert" size={11} />outside the worktree</span>{/if}
      </div>
      {#if others.length}<p class="same">Also asked by {others.join(', ')}, each in the same folder of its own worktree. Your answer goes to all of them.</p>{/if}
    {:else if subject.kind === 'file_change'}
      <ul class="paths">{#each subject.paths as p, pi (pi)}<li><Icon name="file" size={13} /><code class="selectable">{p}</code></li>{/each}</ul>
      {#if subject.diff}<pre class="diff selectable">{subject.diff}</pre>{/if}
    {:else if plan !== null}
      <div class="plan selectable"><Markdown text={plan} brief /></div>
    {:else if subject.kind === 'tool'}
      <pre class="diff selectable">{subject.name}({JSON.stringify(subject.input, null, 2)})</pre>
    {:else if subject.kind === 'question'}
      {#if questions.length}
        {#each questions as q, qi (qi)}
          <fieldset class="q">
            <legend>{q.question}</legend>
            {#each q.options as o, oi (oi)}
              <label class="opt" class:on={(picks[qi] ?? []).includes(o.label)}>
                <input type={q.multiSelect ? 'checkbox' : 'radio'} name="q-{approval.id}-{qi}" checked={(picks[qi] ?? []).includes(o.label)} onchange={() => toggle(qi, o.label, !!q.multiSelect)} />
                <span class="ot"><b>{o.label}</b>{#if o.description}<span class="od">{o.description}</span>{/if}</span>
              </label>
            {/each}
          </fieldset>
        {/each}
      {:else}
        <pre class="diff selectable">{JSON.stringify(subject.questions, null, 2)}</pre>
        <input class="input" placeholder="Your answer…" bind:value={freeText} />
      {/if}
    {:else}
      <pre class="diff selectable">{JSON.stringify(subject.detail, null, 2)}</pre>
    {/if}

    {#if approval.reason}
      <p class="reason selectable"><Provenance kind="model" compact /><span>“{oneLine(approval.reason, 280)}”</span></p>
    {/if}
    {#if approval.risks.length}
      <div class="risks">
        {#each approval.risks as r, ri (ri)}<span class="risk">{r}</span>{/each}
        <Provenance kind="inferred" />
      </div>
    {/if}

    {#if subject.kind === 'question'}
      <footer>
        <button class="btn primary" disabled={busy || unanswered} onclick={submitAnswers}>Send answer</button>
        {#if offered.has('deny')}<button class="btn ghost" disabled={busy} onclick={() => decide('deny')}>Skip</button>{/if}
      </footer>
    {:else if noteOpen}
      <form class="note" onsubmit={(e) => { e.preventDefault(); decide('deny', note.trim() || null); }}>
        <input bind:this={noteEl} class="input" placeholder="Tell {agent?.name ?? 'the agent'} what to do instead…" bind:value={note} spellcheck="true"
          onkeydown={(e) => e.key === 'Escape' && (e.stopPropagation(), (noteOpen = false))} />
        <button class="btn" type="submit" disabled={busy}>{plan !== null ? 'Send the plan back' : 'Deny with note'}</button>
      </form>
    {:else}
      <footer>
        {#each ORDER.filter((c) => offered.has(c)) as c (c)}
          <button class="btn {CHOICE[c].cls}" disabled={busy} onclick={() => decide(c)}>
            {choiceLabel(c)}{#if isKeyTarget && CHOICE[c].kbd}<span class="k">{CHOICE[c].kbd}</span>{/if}
          </button>
        {/each}
        {#if offered.has('deny')}<button class="btn ghost" disabled={busy} onclick={openNote}>{plan !== null ? 'Send it back with a note…' : 'Deny with a note…'}{#if isKeyTarget}<span class="k">⇧⌘⌫</span>{/if}</button>{/if}
      </footer>
      {#if rulePrefix || ruleExact}
        <div class="always">
          <span class="a-l" data-tip="Codly approves this request and stops asking about it, for every agent in this repo. Remove the rule in Settings → Safety.">Always allow in this repo, for every agent:</span>
          {#if rulePrefix}<button class="btn sm" disabled={busy} onclick={() => saveRule('prefix')}>Commands that start with <code>{rulePrefix}</code></button>{/if}
          {#if ruleExact}<button class="btn sm" disabled={busy} onclick={() => saveRule('exact')}>This exact command</button>{/if}
        </div>
      {/if}
    {/if}
    <Why lines={why} />
  </article>
{/if}

<style>
  .card { position: relative; display: flex; flex-direction: column; gap: 12px; padding: 16px 20px 16px; border-radius: var(--r-lg); background: linear-gradient(180deg, var(--amber-soft), transparent 70%), var(--surface-1); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--amber) 30%, transparent); scroll-margin: 80px; }
  .card.key { box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--amber) 60%, transparent), 0 0 0 4px color-mix(in srgb, var(--amber) 12%, transparent); }
  header { display: flex; align-items: center; gap: 10px; min-width: 0; }
  .who { display: grid; border-radius: 50%; }
  h3 { flex: 1; min-width: 0; font-size: var(--t-lg); font-weight: var(--w-semi); color: var(--ink-1); }
  .qh { margin-left: 8px; font-size: var(--t-xs); font-weight: var(--w-semi); letter-spacing: 0.06em; text-transform: uppercase; color: var(--amber-ink); }
  .since { font-size: var(--t-sm); color: var(--amber-ink); }
  .cmd { position: relative; display: flex; gap: 10px; padding: 12px 40px 12px 14px; border-radius: var(--r-md); background: var(--code-bg); font-family: var(--font-mono); font-size: 12.5px; line-height: 19px; }
  .dollar { color: var(--ink-3); }
  .cmd code { color: var(--ink-1); white-space: pre-wrap; word-break: break-all; }
  .copy { position: absolute; right: 6px; top: 7px; }
  .meta { display: flex; align-items: center; gap: 12px; font-size: var(--t-sm); color: var(--ink-3); min-width: 0; margin-top: -4px; }
  .cwd { font-size: 11px; color: var(--ink-3); }
  .ok { display: inline-flex; align-items: center; gap: 4px; color: var(--ok); white-space: nowrap; }
  .bad { display: inline-flex; align-items: center; gap: 4px; color: var(--bad); white-space: nowrap; }
  .reason { display: flex; gap: 8px; align-items: flex-start; font-size: var(--t-md); line-height: var(--lh-md); color: var(--ink-2); }
  .reason :global(.pv) { margin-top: 1px; }
  .risks { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .risk { height: 22px; padding: 0 9px; border-radius: var(--r-full); font-size: var(--t-sm); line-height: 22px; color: var(--ink-2); background: var(--surface-2); }
  footer { display: flex; flex-wrap: wrap; gap: 6px; }
  .same { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-2); margin-top: -4px; }
  .always { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; font-size: var(--t-sm); color: var(--ink-3); }
  .a-l { margin-right: 2px; }
  .always code { font-family: var(--font-mono); font-size: 11.5px; color: var(--ink-1); }
  .note { display: flex; gap: 6px; }
  .note .input { flex: 1; }
  .paths { display: flex; flex-direction: column; gap: 4px; }
  .paths li { display: flex; align-items: center; gap: 6px; color: var(--ink-3); }
  .paths code { font-size: var(--mono); color: var(--ink-1); }
  .diff { margin: 0; max-height: 220px; overflow: auto; padding: 10px 12px; border-radius: var(--r-md); background: var(--code-bg); font-size: 11.5px; line-height: 17px; font-family: var(--font-mono); color: var(--ink-2); }
  .plan { max-height: 360px; overflow: auto; padding: 10px 12px; border-radius: var(--r-md); background: var(--code-bg); font-size: var(--t-md); line-height: var(--lh-md); color: var(--ink-1); }
  .q { border: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  legend { font-size: 16px; line-height: 24px; color: var(--ink-1); margin-bottom: 6px; padding: 0; }
  .opt { display: flex; gap: 10px; align-items: flex-start; padding: 10px 12px; border-radius: var(--r-md); background: var(--surface-1); transition: background-color var(--d-2) var(--ease-out), box-shadow var(--d-2); }
  .opt:hover { background: var(--surface-2); }
  .opt.on { background: var(--surface-2); box-shadow: inset 0 0 0 1.5px var(--hue); }
  .opt input { margin-top: 3px; accent-color: var(--hue); }
  .ot { display: flex; flex-direction: column; gap: 2px; }
  .ot b { font-weight: var(--w-semi); }
  .od { color: var(--ink-3); font-size: var(--t-sm); line-height: var(--lh-sm); }
  .resolved { display: flex; align-items: center; gap: 8px; height: 28px; font-size: var(--t-sm); color: var(--ink-3); min-width: 0; }
  .resolved :global(svg) { color: var(--ink-3); }
  .s-approved :global(svg) { color: var(--ok); }
  .s-denied :global(svg) { color: var(--bad); }
  .r-l { color: var(--ink-2); white-space: nowrap; }
  .r-who { color: var(--ink-2); font-weight: var(--w-med); }
  .resolved code { flex: 1; font-size: 11.5px; color: var(--ink-3); }
  .r-a { flex: 0 1 auto; max-width: 50%; color: var(--ink-1); }
  .resolved time { font-size: var(--t-xs); }
</style>
