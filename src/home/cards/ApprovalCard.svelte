<!--
  An approval, inline where it happened. Exactly what will run and where, why (the agent's
  words), inferred risks, and only the choices the provider offered. The oldest visible request
  takes the keyboard: ⌘↵ once · ⌥⌘↵ session · ⌘⌫ deny · ⇧⌘⌫ deny with a note.
  Questions (AskUserQuestion / requestUserInput) render as a choice to answer: 1-9 pick, ↵ in the
  answer field or ⌘↵ send. A question that comes in its request's envelope (an MCP server's
  elicitation) shows its text, the envelope folded under "Details".
  A Codex permissions request says in words what it wants (`permissionsAsked`), its detail folded.
  The card is never taller than the thread that shows it: its title, its command and its buttons
  stay, and the rest scrolls inside it (the "Always allow" choices fold when it is short of room).
  A plan (Claude's plan mode asks to leave it with `ExitPlanMode`) renders as the plan itself:
  accept it, accept it with your changes (Claude Code takes your text as the plan), or send it
  back with a note. It has no plain "no" (`footerButtons`): ⌘⌫ opens the note field too.
  Once answered, the card is one row: how it ended, and what you said with the answer (your
  answer to a question, the note of a "no", that you changed the plan), as the core recorded it.
  The plan card and the card for a host that Claude Code's sandbox blocks are for Claude agents
  only: these two tool names are Claude Code's own, and for any other CLI a request with such a
  name shows as every other tool request, by its name and its input.
  The same command asked by several agents is one card (`twins`): one answer goes to all.
  "Always allow in this repo" saves a rule in Codly for every agent of the repo; the card
  offers it only when the core says the request can be saved (never for what Codly always asks):
  a command, or a tool of an MCP server by its name.
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
  import { yourAnswers, yourChoice } from '../../lib/state/answers';
  import { changedPlan } from '../../lib/state/control.svelte';
  import { keyTargetApproval, registerQuestionKeys } from '../../lib/state/keys';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { clock, timer } from '../../lib/util/format';
  import { footerButtons, PLAN_NOTE_HINT, type FooterButton } from '../../lib/util/approvalFooter';
  import { idVar } from '../../lib/util/labels';
  import { firstQuestion, outcomeBy, settledWord, wordsOf } from '../../lib/util/outcome';
  import { permissionsAsked } from '../../lib/util/permissionsAsked';
  import { shownPlan } from '../../lib/util/planRequest';
  import { digitQuestion, sendsFromField } from '../../lib/util/questionKeys';
  import { HOST_ALLOW_LASTS, sandboxHost, sandboxWants } from '../../lib/util/sandbox';
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

  /** The plan a Claude agent in plan mode shows you before it changes anything; null for every other request and every other CLI. */
  const plan = $derived(shownPlan(subject, agent?.provider));

  /** The host a command in Claude Code's sandbox wants to reach, whole ('' = it names none); null for every other request and every other CLI. */
  const netHost = $derived(sandboxHost(subject, agent?.provider));

  /** What a Codex permissions request asks for, in words; null for every other request. */
  const perms = $derived(subject.kind === 'permissions' ? permissionsAsked(subject.detail) : null);

  const headline = $derived.by(() => {
    const who = agent?.name ?? 'An agent';
    if (plan !== null) return `${who} shows you its plan`;
    const net = sandboxWants(subject, agent?.provider, 80);
    if (net !== null) return `${who} wants to ${net}`;
    switch (subject.kind) {
      case 'command':
        if (others.length) return `${who} and ${others.length === 1 ? others[0] : `${others.length} others`} want to run the same command`;
        return `${who} wants to run a command`;
      case 'file_change': return `${who} wants to change ${subject.paths.length === 1 ? 'a file' : `${subject.paths.length} files`}`;
      case 'tool': return `${who} wants to use ${oneLine(subject.name, 40)}`;
      case 'question': return `${who} asks you`;
      case 'permissions': return perms?.lines.length ? `${who} wants to ${perms.wants}` : `${who} asks for more permissions`;
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

  /** The buttons under the request: a plan has its own set, with no plain "no"; a permissions
   *  request is allowed for this turn or for this session. */
  const buttons = $derived(footerButtons(plan !== null, approval.choices, subject.kind === 'permissions'));
  function press(b: FooterButton) {
    if (b.act === 'edit') openEdit();
    else if (b.act === 'note') openNote();
    else if (b.choice) decide(b.choice);
  }

  async function decide(choice: ApprovalChoice, message: string | null = null, answers: JsonValue | null = null) {
    if (!pending || busy || !offered.has(choice)) return;
    busy = true;
    await resolveApprovals([approval, ...twins].map((a) => a.id), choice, message, answers);
    busy = false;
    noteOpen = false;
    editOpen = false;
  }

  // ── a plan you change before you accept it ──
  let editOpen = $state(false);
  let edited = $state('');
  let editEl: HTMLTextAreaElement | undefined = $state();
  function openEdit() {
    if (plan === null) return;
    edited = plan;
    editOpen = true;
    tick().then(() => editEl?.focus());
  }
  /** Accept the plan with your text. Left as it was, it is a plain accept. */
  function acceptEdited() {
    if (plan !== null && edited.trim()) decide('allow_once', null, changedPlan(plan, edited));
  }
  function onEditKey(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.stopPropagation();
      editOpen = false;
    } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      acceptEdited();
    }
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
    if (!pending || (subject.kind !== 'command' && subject.kind !== 'tool')) return;
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
    editOpen = false;
    noteOpen = true;
    tick().then(() => noteEl?.focus());
  }
  /** The "no" with what you wrote. A plan goes back only with a note: without one Claude Code shows the same plan again. */
  function sendNote() {
    const text = note.trim();
    if (plan === null || text) decide('deny', text || null);
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

  // ── the card fits the thread ──
  // At most as tall as the part of the scroller that shows it, less its padding: brought to the
  // bottom (the thread follows the newest line) or to the centre, it is whole. `tight`: with the
  // "Always allow" choices open its body would scroll, so they fold.
  let fit = $state<number | null>(null);
  let tight = $state(false);
  let bodyEl: HTMLElement | undefined = $state();
  function scrollerOf(node: HTMLElement): HTMLElement | null {
    for (let p = node.parentElement; p; p = p.parentElement) if (/(auto|scroll)/.test(getComputedStyle(p).overflowY)) return p;
    return null;
  }
  $effect(() => {
    if (!el || !pending) return;
    const sc = scrollerOf(el);
    if (!sc) return;
    const measure = () => {
      const cs = getComputedStyle(sc);
      const room = sc.clientHeight - parseFloat(cs.paddingTop) - parseFloat(cs.paddingBottom) - 8;
      fit = Math.max(220, Math.floor(room));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(sc);
    return () => ro.disconnect();
  });
  $effect(() => {
    void fit;
    void ruleOffer;
    tight = false;
    tick().then(() => {
      if (bodyEl && bodyEl.scrollHeight > bodyEl.clientHeight + 1) tight = true;
    });
  });
  // When it becomes the request the keyboard answers, the thread shows it whole; and again when
  // the "Always allow" choices come (a moment after the card) while you see its top.
  function showWhole() {
    const card = el;
    // After the thread's own follow of the newest line (its tick) has run.
    if (card) setTimeout(() => card.isConnected && card.scrollIntoView({ block: 'nearest', behavior: 'auto' }), 0);
  }
  let wasKey = false;
  $effect(() => {
    const now = isKeyTarget;
    if (now && !wasKey) tick().then(showWhole);
    wasKey = now;
  });
  $effect(() => {
    void ruleOffer;
    if (!el || !isKeyTarget) return;
    const card = el;
    tick().then(() => {
      const top = card.getBoundingClientRect().top;
      const view = scrollerOf(card)?.getBoundingClientRect();
      if (!view || (top >= view.top - 1 && top < view.bottom)) showWhole();
    });
  });

  // ── questions ──
  // `own`: the question also takes a typed answer of your own. Claude Code's AskUserQuestion
  // answers each question with a string (the recorded `answers: { "<question>": "<label>" }`),
  // and a question with no options can only be answered that way; Codex says it per question
  // (`isOther`). `secret`: Codex asks for a value that is not to be shown (`isSecret`).
  type Q = { question: string; header?: string; options: { label: string; description?: string }[]; multiSelect?: boolean; own: boolean; secret: boolean };
  const questions = $derived.by((): Q[] => {
    if (subject.kind !== 'question') return [];
    const raw = subject.questions as unknown;
    const arr = Array.isArray(raw) ? raw : raw && typeof raw === 'object' && Array.isArray((raw as { questions?: unknown }).questions) ? (raw as { questions: unknown[] }).questions : [];
    return arr
      .filter((q): q is Record<string, unknown> => !!q && typeof q === 'object')
      .map((q) => {
        const options = Array.isArray(q.options) ? q.options.map((o) => (typeof o === 'string' ? { label: o } : { label: String((o as Record<string, unknown>).label ?? ''), description: (o as Record<string, unknown>).description as string | undefined })) : [];
        return {
          question: String(q.question ?? q.prompt ?? ''),
          header: q.header ? String(q.header) : undefined,
          options,
          multiSelect: !!q.multiSelect,
          own: options.length === 0 || q.isOther === true || agent?.provider === 'claude',
          secret: q.isSecret === true,
        };
      });
  });
  /** A question in its request's envelope (an MCP server's elicitation): its text; null when the card lists questions or has no text to show. */
  const envelope = $derived(subject.kind === 'question' && !questions.length ? firstQuestion(approval) : null);
  /** The link an elicitation in "url" mode asks you to open. */
  const envelopeUrl = $derived.by(() => {
    if (subject.kind !== 'question' || questions.length) return null;
    const u = (subject.questions as { mode?: unknown; url?: unknown } | null)?.url;
    return typeof u === 'string' && /^https?:\/\//.test(u) ? u : null;
  });
  let picks = $state<Record<number, string[]>>({});
  let own = $state<Record<number, string>>({});
  let freeText = $state('');
  function toggle(qi: number, label: string, multi: boolean) {
    const cur = picks[qi] ?? [];
    picks = { ...picks, [qi]: multi ? (cur.includes(label) ? cur.filter((l) => l !== label) : [...cur, label]) : [label] };
    // One answer to a one-choice question: an option you pick takes the place of your own text.
    if (!multi && own[qi]) own = { ...own, [qi]: '' };
  }
  function typeOwn(qi: number, text: string, multi: boolean) {
    own = { ...own, [qi]: text };
    if (!multi && text.trim() && picks[qi]?.length) picks = { ...picks, [qi]: [] };
  }
  function submitAnswers() {
    if (busy || unanswered) return;
    const answers: Record<string, JsonValue> = {};
    questions.forEach((q, i) => {
      const typed = (own[i] ?? '').trim();
      const picked = picks[i] ?? [];
      answers[q.question] = q.multiSelect ? (typed ? [...picked, typed] : picked) : typed || (picked[0] ?? '');
    });
    if (!questions.length) answers.answer = freeText;
    decide(offered.has('allow_once') ? 'allow_once' : approval.choices[0], null, answers);
  }
  const answeredAt = (i: number) => !!picks[i]?.length || !!(own[i] ?? '').trim();
  const unanswered = $derived(questions.some((q, i) => (q.options.length > 0 || q.own) && !answeredAt(i)));
  /** ↵ (not ⇧↵) and ⌘↵ in an answer field send. */
  function onAnswerKey(e: KeyboardEvent) {
    if (sendsFromField({ key: e.key, mod: e.metaKey || e.ctrlKey, alt: e.altKey, shift: e.shiftKey, composing: e.isComposing })) {
      e.preventDefault();
      submitAnswers();
    }
  }
  /** A typed answer grows with its text, up to about eight lines. */
  function grow(t: HTMLTextAreaElement) {
    const fit = () => {
      t.style.height = 'auto';
      t.style.height = `${Math.min(t.scrollHeight + 2, 190)}px`;
    };
    fit();
    t.addEventListener('input', fit);
    return { destroy: () => t.removeEventListener('input', fit) };
  }
  // The keys of the card that waits: 1-9 pick in the question that has no answer yet, ⌘↵ sends.
  const digitAt = $derived(digitQuestion(questions.map((q, i) => ({ options: q.options.length, answered: answeredAt(i) }))));
  $effect(() => {
    if (!pending || subject.kind !== 'question') return;
    return registerQuestionKeys(approval.id, {
      options: () => (digitAt >= 0 ? questions[digitAt].options.length : 0),
      pick: (n) => {
        const q = questions[digitAt];
        if (q) toggle(digitAt, q.options[n].label, !!q.multiSelect);
      },
      send: submitAnswers,
    });
  });

  // A question is answered or skipped, a plan is accepted (with your changes, when you changed
  // it) or sent back, every other request is approved or denied. Your words show after it.
  const said = $derived(wordsOf(approval, approval.decided_by));
  const outcome = $derived(settledWord(approval.status, approval, agent?.provider ?? null, !!said?.planChanged));
  const by = $derived(outcomeBy(approval.decided_by));
  const answer = $derived(subject.kind === 'question' && approval.status === 'approved' ? (yourAnswers.get(approval.id) ?? null) : null);
  /** Your words on the closed row: your answer to a question, or the note that went with a "no". */
  const quote = $derived(answer ?? said?.note ?? null);
  const what = $derived(subject.kind === 'command' ? subject.command : subject.kind === 'question' ? (questions[0]?.question || envelope || 'a question') : subject.kind === 'file_change' ? subject.paths.join(', ') : subject.kind === 'tool' ? (plan !== null ? 'its plan' : netHost !== null ? (netHost ? `network access to ${netHost}` : 'network access') : subject.name) : perms?.lines.length ? perms.wants : 'more permissions');
  /** How long you allowed it, when this window sent the answer: a permissions request is for a turn or for the session. */
  const allowedFor = $derived.by(() => {
    if (approval.status !== 'approved') return null;
    const c = yourChoice.get(approval.id);
    return c === 'allow_session' ? 'for this session' : c === 'allow_once' && subject.kind === 'permissions' ? 'for this turn' : null;
  });
  /** The closed row cuts what was asked and your words to fit: both whole, for its tooltip. */
  const asked = $derived(quote ? `${oneLine(what, Infinity)} “${oneLine(quote, Infinity)}”` : null);
</script>

{#if !pending}
  <div class="resolved s-{approval.status}" class:has-answer={!!quote}>
    <Icon name={approval.status === 'approved' ? 'check' : 'x'} size={12} stroke={2} />
    <span class="r-l">{outcome} {by}</span>
    {#if agent}<span class="r-who">{agent.name}</span>{/if}
    {#if quote}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex (the row cuts what was asked and your words: focus shows both whole, as hover does) -->
      <code class="truncate asked" tabindex="0" data-tip={asked}>{oneLine(what, 120)}</code>
      <time class="num">{clock(approval.created_at)}</time>
      <!-- Your words have a line of their own, two at most: as wide as the row. -->
      <span class="r-a" data-tip={asked}>“{oneLine(quote, 600)}”</span>
    {:else}
      <code class="truncate">{oneLine(what, 120)}{#if allowedFor}<span class="r-for"> · {allowedFor}</span>{/if}</code>
      <time class="num">{clock(approval.created_at)}</time>
    {/if}
  </div>
{:else}
  <article class="card" class:key={isKeyTarget} bind:this={el} style:--hue={idVar(agent?.color ?? 0)} style:max-height={fit ? `${fit}px` : null} aria-label={headline}>
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
    {/if}

    <div class="body" bind:this={bodyEl}>
    {#if subject.kind === 'command'}
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
      {#if editOpen}
        <textarea bind:this={editEl} class="plan-edit" rows="12" bind:value={edited} spellcheck="true" aria-label="The plan, with your changes" onkeydown={onEditKey}></textarea>
      {:else}
        <div class="plan selectable"><Markdown text={plan} brief /></div>
      {/if}
    {:else if netHost !== null}
      <p class="net selectable">A shell command of {agent?.name ?? 'the agent'}, inside Claude Code’s sandbox, wants to connect to {#if netHost}<code>{netHost}</code>{:else}a host on the network{/if}. {HOST_ALLOW_LASTS} If you deny, the command gets a network error.</p>
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
                {#if isKeyTarget && qi === digitAt && oi < 9}<span class="k num" aria-hidden="true">{oi + 1}</span>{/if}
              </label>
            {/each}
            {#if q.own}
              {#if q.secret}
                <input class="input" type="password" autocomplete="off" placeholder="Your answer…" aria-label="Your answer to: {q.question}"
                  value={own[qi] ?? ''} oninput={(e) => typeOwn(qi, e.currentTarget.value, !!q.multiSelect)} onkeydown={onAnswerKey} />
              {:else}
                <textarea class="input answer" rows="1" use:grow spellcheck="true" placeholder={q.options.length ? 'Or write your own answer…' : 'Your answer…'} aria-label="Your answer to: {q.question}"
                  value={own[qi] ?? ''} oninput={(e) => typeOwn(qi, e.currentTarget.value, !!q.multiSelect)} onkeydown={onAnswerKey}></textarea>
              {/if}
            {/if}
          </fieldset>
        {/each}
      {:else}
        {#if envelope}
          <p class="ask selectable">{envelope}</p>
          {#if envelopeUrl}<p class="same">It asks you to open <code class="selectable">{envelopeUrl}</code></p>{/if}
        {:else}
          <p class="same">Codly cannot read this question in words: what the agent sent is under Details.</p>
        {/if}
        <textarea class="input answer" rows="1" use:grow spellcheck="true" placeholder="Your answer…" aria-label="Your answer" bind:value={freeText} onkeydown={onAnswerKey}></textarea>
        <details class="raw"><summary>Details</summary><pre class="diff selectable">{JSON.stringify(subject.questions, null, 2)}</pre></details>
      {/if}
    {:else if perms}
      {#if perms.lines.length}
        <ul class="perms">
          {#each perms.lines as l, li (li)}
            <li>
              <Icon name={l.place === null ? 'globe' : 'file'} size={13} />
              <span class="p-v">{l.verb}</span>{#if l.place !== null}<code class="selectable" data-tip={l.place}>{l.place}</code>{/if}
              {#if l.outside}<span class="bad"><Icon name="alert" size={11} />outside the folder it works in</span>{/if}
            </li>
          {/each}
        </ul>
      {:else}
        <p class="same">{agent?.name ?? 'The agent'} asks for more than its sandbox gives it. Codly cannot read what: it is under Details.</p>
      {/if}
      {#if perms.cwd}<p class="meta">It works in&nbsp;<code class="cwd truncate" data-tip={perms.cwd}>{tildify(perms.cwd)}</code></p>{/if}
      {#if offered.has('allow_once') && offered.has('allow_session')}<p class="same">“For this turn” lasts until {agent?.name ?? 'the agent'}’s turn ends, “for this session” until it stops.</p>{/if}
      <details class="raw"><summary>Details</summary><pre class="diff selectable">{JSON.stringify(subject.detail, null, 2)}</pre></details>
    {/if}

    {#if approval.reason && approval.reason !== envelope}
      <p class="reason selectable"><Provenance kind="model" compact /><span>“{oneLine(approval.reason, 280)}”</span></p>
    {/if}
    {#if approval.risks.length}
      <div class="risks">
        {#each approval.risks as r, ri (ri)}<span class="risk">{r}</span>{/each}
        <Provenance kind="inferred" />
      </div>
    {/if}
    <Why lines={why} />
    </div>

    {#if subject.kind === 'question'}
      <footer>
        <button class="btn primary" disabled={busy || unanswered} onclick={submitAnswers}>Send answer{#if isKeyTarget}<span class="k">⌘↵</span>{/if}</button>
        {#if offered.has('deny')}<button class="btn ghost" disabled={busy} onclick={() => decide('deny')}>Skip{#if isKeyTarget}<span class="k">⌘⌫</span>{/if}</button>{/if}
      </footer>
    {:else if noteOpen}
      <form class="note" onsubmit={(e) => { e.preventDefault(); sendNote(); }}>
        <input bind:this={noteEl} class="input" placeholder="Tell {agent?.name ?? 'the agent'} what to do instead…" bind:value={note} spellcheck="true"
          onkeydown={(e) => e.key === 'Escape' && (e.stopPropagation(), (noteOpen = false))} />
        <button class="btn" type="submit" disabled={busy || (plan !== null && !note.trim())}>{plan !== null ? 'Send the plan back' : 'Deny with note'}</button>
        <button class="btn ghost" type="button" disabled={busy} onclick={() => (noteOpen = false)}>Cancel</button>
      </form>
      {#if plan !== null}<p class="same">{PLAN_NOTE_HINT}</p>{/if}
    {:else if editOpen}
      <footer>
        <button class="btn primary" disabled={busy || !edited.trim()} onclick={acceptEdited}>Accept with my changes</button>
        <button class="btn ghost" disabled={busy} onclick={() => (editOpen = false)}>Keep its plan</button>
      </footer>
      <p class="same">{agent?.name ?? 'The agent'} then works from your text: Claude Code takes it as the plan.</p>
    {:else}
      <footer>
        {#each buttons as b (b.label)}
          <button class="btn {b.cls}" disabled={busy} onclick={() => press(b)}>
            {b.label}{#if isKeyTarget && b.kbd}<span class="k">{b.kbd}</span>{/if}
          </button>
        {/each}
      </footer>
      {#if rulePrefix || ruleExact}
        {#snippet ruleButtons()}
          {#if rulePrefix}<button class="btn sm" disabled={busy} onclick={() => saveRule('prefix')}>Commands that start with <code>{rulePrefix}</code></button>{/if}
          {#if ruleExact}<button class="btn sm" disabled={busy} onclick={() => saveRule('exact')}>{subject.kind === 'tool' ? 'This tool' : 'This exact command'}</button>{/if}
        {/snippet}
        {#if tight}
          <details class="always folded">
            <summary>Always allow in this repo…</summary>
            <div class="always">{@render ruleButtons()}</div>
          </details>
        {:else}
          <div class="always">
            <span class="a-l" data-tip="Codly approves this request and stops asking about it, for every agent in this repo. Remove the rule in Settings → Safety.">Always allow in this repo, for every agent:</span>
            {@render ruleButtons()}
          </div>
        {/if}
      {/if}
    {/if}
  </article>
{/if}

<style>
  .card { position: relative; display: flex; flex-direction: column; gap: 12px; padding: 16px 20px 16px; border-radius: var(--r-lg); background: linear-gradient(180deg, var(--amber-soft), transparent 70%), var(--surface-1); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--amber) 30%, transparent); scroll-margin: 80px; }
  /* Never taller than the thread (max-height from the script): the title, the command and the buttons stay, the body gives way and scrolls. */
  .card > * { flex-shrink: 0; }
  .card > .body { flex: 0 1 auto; min-height: 0; overflow-y: auto; overscroll-behavior: contain; display: flex; flex-direction: column; gap: 12px; padding: 2px; margin: -2px; }
  .body:empty { display: none; }
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
  .always.folded { display: block; }
  .always.folded summary { width: fit-content; cursor: pointer; color: var(--ink-2); }
  .always.folded .always { margin-top: 6px; }
  .note { display: flex; gap: 6px; }
  .note .input { flex: 1; }
  .paths { display: flex; flex-direction: column; gap: 4px; }
  .paths li { display: flex; align-items: center; gap: 6px; color: var(--ink-3); }
  .paths code { font-size: var(--mono); color: var(--ink-1); }
  .diff { margin: 0; max-height: 220px; overflow: auto; padding: 10px 12px; border-radius: var(--r-md); background: var(--code-bg); font-size: 11.5px; line-height: 17px; font-family: var(--font-mono); color: var(--ink-2); }
  .net { font-size: var(--t-md); line-height: var(--lh-md); color: var(--ink-2); }
  .net code { font-family: var(--font-mono); font-size: 12.5px; color: var(--ink-1); word-break: break-all; }
  .plan { max-height: 360px; overflow: auto; padding: 10px 12px; border-radius: var(--r-md); background: var(--code-bg); font-size: var(--t-md); line-height: var(--lh-md); color: var(--ink-1); }
  .plan-edit { resize: vertical; min-height: 120px; max-height: 360px; padding: 10px 12px; border-radius: var(--r-md); background: var(--code-bg); font-family: var(--font-mono); font-size: 12.5px; line-height: 19px; color: var(--ink-1); }
  .plan-edit:focus-visible { outline: 2px solid var(--live); outline-offset: 1px; }
  .perms { display: flex; flex-direction: column; gap: 6px; }
  .perms li { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; font-size: var(--t-md); line-height: var(--lh-md); color: var(--ink-2); min-width: 0; }
  .perms li > :global(svg) { color: var(--ink-3); }
  .perms code { font-family: var(--font-mono); font-size: 12.5px; color: var(--ink-1); word-break: break-all; }
  .meta code.cwd { font-family: var(--font-mono); }
  .ask { font-size: 16px; line-height: 24px; color: var(--ink-1); white-space: pre-wrap; }
  .raw summary { width: fit-content; cursor: pointer; font-size: var(--t-sm); color: var(--ink-3); }
  .raw[open] summary { margin-bottom: 6px; }
  .answer { resize: none; min-height: 36px; max-height: 190px; line-height: 20px; padding-top: 7px; padding-bottom: 7px; overflow-y: auto; font-family: inherit; }
  .opt .k { margin-left: auto; align-self: center; min-width: 18px; height: 18px; padding: 0 5px; border-radius: var(--r-sm); font-size: var(--t-xs); line-height: 18px; text-align: center; color: var(--ink-3); background: var(--surface-2); }
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
  .r-for { color: var(--ink-2); }
  /* A row with your words (an answer, a note): what was asked on the row's line, your words on
     their own lines below it, as wide as the row and two lines at most (all of it in the tooltip
     and on focus). */
  .resolved.has-answer { flex-wrap: wrap; height: auto; min-height: 28px; padding: 5px 0; row-gap: 2px; }
  .r-a { flex: 1 0 100%; padding-left: 20px; color: var(--ink-1); line-height: var(--lh-sm); display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; overflow-wrap: anywhere; }
  .resolved time { font-size: var(--t-xs); }
</style>
