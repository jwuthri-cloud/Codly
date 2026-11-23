<!--
  The composer: one line that grows. ↵ sends, ⇧↵ breaks the line. It never locks: the message
  shows at once, and if the agent is mid-turn it queues (the conversation says so). Start with
  `@agent` to talk to one agent 1:1 (the thread switches and shows as a chip); `@` mid-sentence
  mentions an agent or a file of the repo (its path, which the agent reads), `#` refers to a task.
  `/` alone on the line lists Codly's own actions (new request, from an issue or a pull request,
  stop, mode, model, clear…) and the controls of the agent you talk to (compact, context, its
  way of working, undo, your terminal, its MCP servers: composerActions.ts), and ↵
  runs one; it is not sent. One that cannot run right now (stop, with nothing running) says why
  and is not sent either. A CLI's own slash commands are not offered: Codly cannot run them, and
  a message that starts with a slash command Codly does not have is not sent (the composer names
  the closest action). While the agent is in your terminal the composer says so and offers
  "Give it back".
  Esc twice on an empty line stops the running turn; ↑ on an empty line brings back what you sent. A
  draft (text and files) is kept when the composer leaves the screen, and Edit on an earlier
  message does not replace it. A new request that holds a draft stays a tab when you leave it.
  The hint in the empty field is an element of its own, not the field's `placeholder`: a hint
  longer than the field ends in an ellipsis in every engine (a textarea's placeholder is cut
  mid-letter in Chromium).
-->
<script lang="ts">
  import { onDestroy, tick, untrack } from 'svelte';
  import { api } from '../lib/api';
  import type { Agent } from '../lib/gen/Agent';
  import { sendChat, setRequestPrefs, stopThreadByEsc } from '../lib/state/actions';
  import { setPermissionMode } from '../lib/state/control.svelte';
  import { giveBack, loadTakeover, takeovers } from '../lib/state/takeover.svelte';
  import { intakeUi } from '../lib/state/intake.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import { busy, busyHint, stoppedHint, waitingHint } from '../lib/state/queue';
  import { store } from '../lib/state/store.svelte';
  import { CLOSED_LIST, isRequestThread, ui } from '../lib/state/ui.svelte';
  import { fuzzy } from '../lib/util/fuzzy';
  import { idVar, PROVIDER_LABEL, TASK_STATUS_LABEL } from '../lib/util/labels';
  import { basename, dirname } from '../lib/util/sanitize';
  import Icon from '../components/Icon.svelte';
  import DictationButton from './DictationButton.svelte';
  import VoiceButton from './VoiceButton.svelte';
  import { appendTranscript } from '../lib/util/freeflow';
  import Portrait from '../components/Portrait.svelte';
  import LeadControls, { MODES } from './LeadControls.svelte';
  import AgentControls from './AgentControls.svelte';
  import { nextWay, wayOf } from './agentPicker';
  import { messageWithFiles } from './attached';
  import { draftSlot, loadDraft, nextAttId, saveDraft, seedDraft, type Att } from './drafts';
  import { recall, type Recalled } from './recall';
  import { agentTurns } from './agentTurns';
  import { agentDo, slashCommands, slashContext, type SlashDo } from './composerActions';
  import { fileMention, filterSlash, pointerMoved, rankFiles, triggerAt, unknownSlash, unknownSlashLine, type Trigger } from './composerMenu';
  import { repoFiles } from './repoFiles';

  interface Props {
    thread: string;
  }
  let { thread }: Props = $props();

  let text = $state('');

  // ── attachments: pasted, dropped or picked files, saved in the repo; the message names their paths ──
  let files = $state<Att[]>([]);
  let picker: HTMLInputElement | undefined = $state();
  let dragging = $state(false);
  const saving = $derived(files.some((f) => !f.path && !f.failed));
  const ready = $derived(files.filter((f) => f.path));
  const canSend = $derived(!saving && (!!text.trim() || ready.length > 0));

  function addFiles(list: File[]) {
    for (const f of list) {
      files.push({ id: nextAttId(), name: f.name || 'pasted file', path: null, url: f.type.startsWith('image/') ? URL.createObjectURL(f) : null, failed: false });
      const att = files[files.length - 1];
      api.saveAttachment(f).then(
        (p) => (att.path = p),
        (e) => {
          att.failed = true;
          toast({ kind: 'error', title: `Couldn’t attach ${att.name}`, body: e instanceof Error ? e.message : String(e) });
        },
      );
    }
  }
  function drop(a: Att) {
    if (a.url) URL.revokeObjectURL(a.url);
    files = files.filter((x) => x !== a);
  }
  const hasFiles = (e: DragEvent) => !!e.dataTransfer?.types.includes('Files');
  function paste(e: ClipboardEvent) {
    const list = [...(e.clipboardData?.files ?? [])];
    if (!list.length) return;
    e.preventDefault();
    addFiles(list);
  }
  let ta: HTMLTextAreaElement | undefined = $state();
  const slot = $derived(draftSlot(store.workspace?.id, thread));
  let lastSlot = '';
  let gone = false;

  $effect(() => {
    if (slot !== lastSlot) {
      if (lastSlot) saveDraft(lastSlot, text, files);
      const d = loadDraft(slot);
      text = d.text;
      files = d.files;
      lastSlot = slot;
      tick().then(resize);
    }
  });
  // Leaving the screen (another room, the full office) keeps the draft. `lastSlot`, not `slot`:
  // an office switch in the same moment must not file it under the new office.
  onDestroy(() => {
    gone = true;
    if (lastSlot) saveDraft(lastSlot, text, files);
  });
  $effect(() => {
    const seed = ui.composerSeed;
    if (seed && seed.thread === thread) {
      ui.composerSeed = null;
      // A message ↑ brought back and left as it was is not a draft.
      const next = untrack(() => seedDraft(recalled?.text === text ? '' : text, seed));
      text = next.text;
      if (next.kept) toast({ kind: 'info', title: 'Clear the composer to edit this message', body: 'What you typed is still there.' });
      tick().then(() => {
        resize();
        ta?.focus();
        ta?.setSelectionRange(text.length, text.length);
      });
    }
  });

  // A new request with something in its composer stays a tab when you open another one.
  $effect(() => {
    const here = thread;
    const holds = !!text.trim() || files.length > 0;
    untrack(() => ui.holdDraft(here, holds));
  });

  /** The line was emptied by a pick that may leave this conversation in the same moment (/new,
   *  "@Ada" at the start). The effect above then runs for the next conversation only, so a new
   *  request is told now that it holds no text: left empty, it is dropped, not kept as a tab. */
  function emptied() {
    ui.holdDraft(thread, files.length > 0);
  }

  $effect(() => {
    if (ui.composerFocus) tick().then(() => ta?.focus());
  });

  const agent = $derived(store.threadAgent(thread));
  const oneToOne = $derived(!isRequestThread(thread));
  /** An agent on a task asks per that task's request's permissions (the broker's rule), so its 1:1 shows those. */
  const workerRequest = $derived.by(() => {
    const t = agent && (agent.role === 'worker' || agent.role === 'solo') && agent.task_id ? store.taskMap.get(agent.task_id) : null;
    return t ? store.taskThread(t.id) : null;
  });
  const isBusy = $derived(busy(thread));
  const fresh = $derived(!oneToOne && !agent && !store.chats.some((c) => c.thread === thread) && !store.outbox.some((o) => o.thread === thread));
  /** Its turn stands still on a request of its own, shown in this conversation: it waits for you, it is not at work. */
  const waitsForYou = $derived(isBusy && !!agent && store.pendingApprovals.some((a) => a.agent_id === agent.id));
  /** The agent is in your terminal: Codly sends it nothing until you give it back. */
  const inTerminal = $derived(!!agent && takeovers.has(agent.id));
  const placeholder = $derived(
    inTerminal && agent ? `${agent.name} is in your terminal: give it back to message it here`
    : waitsForYou ? waitingHint(agent?.name ?? 'The agent', agent?.provider)
    : isBusy ? busyHint(agent?.name ?? 'The agent', agent?.provider)
    : oneToOne && agent?.status === 'offline' ? stoppedHint(agent.name)
    : oneToOne ? `Message ${agent?.name ?? 'the agent'} directly…`
    : fresh && thread !== 'lead' ? 'Describe the new request — its own Lead plans it in parallel…'
    : 'Ask the Lead to build, fix or explain — @ for a file, / for actions',
  );

  // ── the menu: @ an agent or a file, # a task, / one of Codly's actions ──
  type Opt = { id: string; label: string; sub: string } & (
    | { kind: 'agent'; agentId: string; insert: string }
    | { kind: 'task' | 'file'; insert: string }
    | { kind: 'command'; icon: string; hint: string; run: (() => void) | null }
  );
  const SHOWN = 6;
  let mention = $state<Trigger | null>(null);
  let mi = $state(0);

  /** The repo's indexed files, asked for when `@` opens the menu. */
  let filePaths = $state.raw<string[]>([]);
  let filesFailed = $state(false);
  function loadFiles() {
    const office = store.workspace?.id;
    if (!office) return;
    repoFiles(office).then(
      (paths) => {
        if (store.workspace?.id !== office) return;
        filePaths = paths;
        filesFailed = false;
      },
      () => (filesFailed = true),
    );
  }

  let lead: LeadControls | undefined = $state();
  const plan = $derived.by(() => {
    void store.rev;
    return oneToOne ? null : store.planOf(thread);
  });
  function clearDraft() {
    for (const f of files) if (f.url) URL.revokeObjectURL(f.url);
    files = [];
  }
  /** `/` opened the menu: ask the core what only it knows about the agent you talk to (its
   *  terminal, and what it keeps of its turns: the read starts git). Kept per agent. */
  function loadControls(a: Agent | null) {
    if (!a) return;
    void loadTakeover(a.id);
    void agentTurns.ask(a);
  }
  /** What each action of `/` does: the agent's controls are the palette's too (`agentDo`). */
  const actions: SlashDo = {
    ...agentDo(() => thread),
    newRequest: () => ui.newRequest(),
    fromIssue: () => (intakeUi.dialog = 'issue'),
    fromPullRequest: () => (intakeUi.dialog = 'pull_request'),
    requestMode: (mode) => void setRequestPrefs(thread, { mode }),
    leadModels: () => lead?.showModels(),
    changes: () => {
      if (plan) ui.openDiff({ kind: 'plan', id: plan.id });
    },
    clear: clearDraft,
    closed: () => ui.openThread(CLOSED_LIST),
    settings: () => (ui.settingsOpen = true),
    shortcuts: () => (ui.shortcutsOpen = true),
  };
  /** What `/` offers: Codly's actions and the agent's controls (composerActions.ts). */
  const commands = $derived(slashCommands(slashContext(thread, MODES), actions));
  /** The line starts with a slash command Codly does not have: it is not sent. */
  const unknown = $derived(unknownSlash(text, commands.map((c) => c.name)));
  // Another conversation while the menu is open: ask about its agent (once per agent, not on
  // each of its updates).
  const agentId = $derived(agent?.id ?? null);
  $effect(() => {
    void agentId;
    untrack(() => {
      if (mention?.trigger === '/') loadControls(agent);
    });
  });

  const options = $derived.by((): Opt[] => {
    const m = mention;
    if (!m) return [];
    if (m.trigger === '/') return filterSlash(commands, m.query).map((c): Opt => ({ id: `/${c.name}`, kind: 'command', label: c.label, sub: `/${c.name}`, icon: c.icon, ...('off' in c ? { hint: c.off, run: null } : { hint: c.hint, run: c.run }) }));
    const best = (pool: Opt[]): Opt[] =>
      pool
        .map((o) => ({ o, m: fuzzy(m.query, o.label) }))
        .filter((x) => x.m)
        .sort((a, b) => b.m!.score - a.m!.score)
        .slice(0, SHOWN)
        .map((x) => x.o);
    if (m.trigger === '#') return best(store.tasks.map((t): Opt => ({ id: t.id, kind: 'task', insert: `#${t.key} `, label: `${t.key} · ${t.title}`, sub: TASK_STATUS_LABEL[t.status] })));
    return [
      ...best(store.agents.filter((a) => a.status !== 'offline').map((a): Opt => ({ id: a.id, kind: 'agent', insert: `@${a.name} `, label: a.name, sub: a.role === 'lead' ? 'Lead' : PROVIDER_LABEL[a.provider], agentId: a.id }))),
      ...rankFiles(filePaths, m.query, SHOWN).map((path): Opt => ({ id: `file:${path}`, kind: 'file', insert: fileMention(path), label: basename(path), sub: dirname(path) })),
    ];
  });
  /** The files could not be listed: the menu says so instead of showing none. */
  const noFiles = $derived(mention?.trigger === '@' && !!mention.query && filesFailed);

  function detect() {
    if (!ta) return;
    const next = triggerAt(text, ta.selectionStart ?? 0);
    if (next?.trigger === '@' && mention?.trigger !== '@') loadFiles();
    if (next?.trigger === '/' && mention?.trigger !== '/') loadControls(agent);
    mention = next;
    if (next) mi = 0;
  }

  function pick(o: Opt) {
    if (!mention || !ta) return;
    const end = ta.selectionStart ?? text.length;
    if (o.kind === 'command') {
      // It cannot run right now: its row says why, and the line stays as typed.
      if (!o.run) return;
      // The line was only the action's name: it runs, and nothing is sent.
      mention = null;
      text = '';
      emptied();
      tick().then(resize);
      o.run();
      return;
    }
    if (o.kind === 'agent' && mention.atStart) {
      // "@Ada" at the start: talk to Ada 1:1.
      const rest = text.slice(end).trimStart();
      mention = null;
      text = '';
      emptied();
      ui.openThread(store.threadOf(o.agentId));
      tick().then(() => {
        text = rest;
        ta?.focus();
        resize();
      });
      return;
    }
    text = text.slice(0, mention.start) + o.insert + text.slice(end);
    const caret = mention.start + o.insert.length;
    mention = null;
    tick().then(() => {
      ta?.focus();
      ta?.setSelectionRange(caret, caret);
      resize();
    });
  }

  /** Where the pointer was last seen over the menu: a row takes the arrow keys' place only when it really moved. */
  let lastPointer: { x: number; y: number } | null = null;
  function hover(e: PointerEvent, i: number) {
    const now = { x: e.clientX, y: e.clientY };
    if (pointerMoved(lastPointer, now)) mi = i;
    lastPointer = now;
  }

  /** The menu scrolls when it is long (every action, in a low window): the row the arrow keys are on stays in view. */
  let menu: HTMLUListElement | undefined = $state();
  function showActive() {
    const row = menu?.querySelector<HTMLElement>('.opt.active');
    if (!menu || !row) return;
    if (row.offsetTop < menu.scrollTop) menu.scrollTop = row.offsetTop;
    else if (row.offsetTop + row.offsetHeight > menu.scrollTop + menu.clientHeight) menu.scrollTop = row.offsetTop + row.offsetHeight - menu.clientHeight;
  }

  function resize() {
    if (!ta) return;
    ta.style.height = 'auto';
    ta.style.height = Math.min(200, ta.scrollHeight) + 'px';
  }

  /** Never blocks: clear the line, show the message, send in the background. */
  function send() {
    if (!canSend || unknown) return;
    sendChat(thread, messageWithFiles(text, ready.flatMap((f) => (f.path ? [f.path] : []))));
    for (const f of files) if (f.url) URL.revokeObjectURL(f.url);
    files = [];
    text = '';
    saveDraft(slot, '', []);
    recalled = null;
    mention = null;
    tick().then(resize);
  }

  /** A transcript from Free Flow joins the draft for review. It is never sent for you. */
  function dictated(forSlot: string, transcript: string) {
    if (gone || forSlot !== slot) {
      const d = loadDraft(forSlot);
      saveDraft(forSlot, appendTranscript(d.text, transcript), d.files);
      return;
    }
    text = appendTranscript(text, transcript);
    tick().then(() => {
      resize();
      ta?.focus();
      ta?.setSelectionRange(text.length, text.length);
    });
  }

  /** The message ↑ brought back (while the line still shows it untouched). */
  let recalled: Recalled | null = null;
  /** ↑ / ↓ on an empty line (or on a recalled message you have not edited) walk through what you sent here. */
  function walk(e: KeyboardEvent): boolean {
    const at = recalled && recalled.text === text ? recalled.at : null;
    if (at == null && (text || e.key === 'ArrowDown')) return false;
    const sent = [...store.chats.filter((c) => c.thread === thread && c.author === 'user'), ...store.outbox.filter((o) => o.thread === thread)].map((m) => m.text);
    const step = recall(sent, at, e.key === 'ArrowUp' ? 'older' : 'newer');
    if (!step) return false;
    e.preventDefault();
    recalled = step.at == null ? null : { at: step.at, text: step.text };
    text = step.text;
    tick().then(() => {
      resize();
      ta?.setSelectionRange(text.length, text.length);
    });
    return true;
  }

  function key(e: KeyboardEvent) {
    if (e.isComposing) return; // IME: Enter confirms the composition, it doesn't send
    if (mention && options.length) {
      if (e.key === 'ArrowDown') { e.preventDefault(); mi = (mi + 1) % options.length; tick().then(showActive); return; }
      if (e.key === 'ArrowUp') { e.preventDefault(); mi = (mi - 1 + options.length) % options.length; tick().then(showActive); return; }
      // A held key runs no action: its repeats would run the next one too.
      if (e.key === 'Enter' || e.key === 'Tab') { e.preventDefault(); if (!(e.repeat && options[mi].kind === 'command')) pick(options[mi]); return; }
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); mention = null; return; }
    }
    // An open find bar closes first: Esc never stops a turn while something is still open.
    if (e.key === 'Escape' && ui.findOpen && !e.defaultPrevented) {
      e.preventDefault();
      ui.findOpen = false;
      return;
    }
    // Esc on an empty line asks to stop the running turn, and a second Esc stops it (an Esc that
    // cancelled a dictation did its job already, and the repeats of a held key stop nothing).
    // With text typed, or nothing running, Esc leaves the field as before.
    if (e.key === 'Escape' && !e.repeat && !e.defaultPrevented && !text.trim() && stopThreadByEsc(thread)) {
      e.preventDefault();
      return;
    }
    if ((e.key === 'ArrowUp' || e.key === 'ArrowDown') && !e.shiftKey && !e.altKey && !e.metaKey && !e.ctrlKey && walk(e)) return;
    // ⇧⇥ cycles the request's mode, like Claude Code.
    if (e.key === 'Tab' && e.shiftKey && !oneToOne) {
      e.preventDefault();
      const now = store.requestPrefs[thread]?.mode ?? (store.settings.auto_run_plans ? 'auto' : 'plan');
      setRequestPrefs(thread, { mode: MODES[(MODES.findIndex((m) => m.id === now) + 1) % MODES.length].id });
      return;
    }
    // In a 1:1 with a Claude agent it cycles the way of the picker under the composer (one
    // rule, agentPicker.ts; the same call as its menu and the agent's sheet).
    const way = oneToOne ? wayOf(agent, !!workerRequest) : null;
    if (e.key === 'Tab' && e.shiftKey && agent && way) {
      e.preventDefault();
      if (!e.repeat) void setPermissionMode(agent.id, nextWay(way, store.requestPrefs[thread]?.approval_mode ?? store.settings.approval_mode));
      return;
    }
    // ⌫ alone: with a modifier it is another key (⌘⌫ denies), and it never leaves the conversation.
    if (e.key === 'Backspace' && !text && oneToOne && !e.repeat && !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) {
      e.preventDefault();
      ui.openRequest(ui.request);
      return;
    }
    if (e.key === 'Enter' && !e.shiftKey && !e.altKey) {
      if ((e.metaKey || e.ctrlKey) && !text.trim()) return; // ⌘↵ with nothing typed: the global primary action
      e.preventDefault();
      send();
    }
  }
</script>

<div class="wrap" class:dragging role="group" aria-label="Composer"
  ondragenter={(e) => { if (hasFiles(e)) { e.preventDefault(); dragging = true; } }}
  ondragover={(e) => { if (hasFiles(e)) { e.preventDefault(); dragging = true; } }}
  ondragleave={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) dragging = false; }}
  ondrop={(e) => { if (!hasFiles(e)) return; e.preventDefault(); dragging = false; addFiles([...(e.dataTransfer?.files ?? [])]); }}>
  {#if mention && (options.length || noFiles)}
    <ul class="mentions" bind:this={menu} class:actions={mention.trigger === '/'} role="listbox" aria-label={mention.trigger === '/' ? 'Actions' : mention.trigger === '#' ? 'Mention a task' : mention.atStart ? 'Talk to an agent, or mention a file' : 'Mention an agent or a file'}>
      {#each options as o, i (o.id)}
        {#if o.kind === 'agent' && i === 0 && mention?.atStart}<li class="mh eyebrow">Talk 1:1 with</li>{/if}
        {#if o.kind === 'file' && options[i - 1]?.kind !== 'file'}<li class="mh eyebrow">Files</li>{/if}
        <li>
          <button class="opt" class:active={i === mi} class:off={o.kind === 'command' && !o.run} role="option" aria-selected={i === mi} aria-disabled={o.kind === 'command' && !o.run} onpointermove={(e) => hover(e, i)} onmousedown={(e) => { e.preventDefault(); pick(o); }}>
            {#if o.kind === 'agent'}<Portrait agent={store.agentMap.get(o.agentId)} size={20} />{:else}<Icon name={o.kind === 'command' ? o.icon : o.kind === 'file' ? 'file' : 'plan'} size={14} />{/if}
            <span class="ol" class:truncate={o.kind !== 'command'}>{o.label}{#if o.kind === 'command'}<span class="oh">{o.hint}</span>{/if}</span>
            <span class="os truncate">{o.sub}</span>
          </button>
        </li>
      {/each}
      {#if noFiles}<li class="mh bad" role="alert">Couldn’t list the repo’s files</li>{/if}
    </ul>
  {:else if unknown}
    <p class="nosuch" role="status">{unknownSlashLine(unknown)}</p>
  {/if}
  {#if files.length}
    <ul class="atts" aria-label="Attachments">
      {#each files as f (f.id)}
        <li class="att" class:failed={f.failed} class:busy={!f.path && !f.failed}>
          {#if f.url}<img src={f.url} alt="" />{:else}<Icon name="file" size={14} />{/if}
          <span class="an truncate">{f.name}</span>
          <button class="ax" aria-label="Remove {f.name}" onclick={() => drop(f)}><Icon name="x" size={10} stroke={2} /></button>
        </li>
      {/each}
    </ul>
  {/if}
  <div class="composer" class:has={canSend && !unknown} class:one={oneToOne} style:--hue={idVar(agent?.color ?? 0)}>
    {#if oneToOne && agent}
      <button class="to" onclick={() => ui.openRequest(ui.request)} data-tip="Back to the request (⌫ on an empty line)">
        <Portrait {agent} size={18} />{agent.name}<Icon name="x" size={11} stroke={2} />
      </button>
    {/if}
    <div class="field">
      <textarea
        bind:this={ta}
        bind:value={text}
        rows="1"
        spellcheck="true"
        aria-label={placeholder}
        oninput={() => { resize(); detect(); }}
        onkeydown={key}
        onclick={detect}
        onpaste={paste}
        onblur={() => setTimeout(() => (mention = null), 120)}
      ></textarea>
      {#if !text}<span class="hint" aria-hidden="true">{placeholder}</span>{/if}
    </div>
    {#if inTerminal && agent}
      {@const back = agent}
      <button class="btn sm give" onclick={() => void giveBack(back.id)} data-tip="Codly closes the terminal it opened; your next message goes on in Codly">Give it back</button>
    {/if}
    <input bind:this={picker} type="file" multiple hidden onchange={() => { addFiles([...(picker?.files ?? [])]); if (picker) picker.value = ''; }} />
    <button class="attach" onclick={() => picker?.click()} aria-label="Attach files" data-tip="Attach images or files — or paste or drop them"><Icon name="paperclip" size={15} stroke={1.6} /></button>
    <DictationButton target={ta} thread={slot} onText={dictated} />
    <VoiceButton {thread} />
    <button class="send" disabled={!canSend || !!unknown} onclick={send} aria-label={isBusy ? 'Queue' : 'Send'} data-tip={isBusy ? 'Send — it queues until the turn ends' : 'Send'} data-tip-kbd="↵">
      <Icon name="send" size={15} stroke={1.8} />
    </button>
  </div>
  {#if !oneToOne}<LeadControls {thread} bind:this={lead} />{:else if workerRequest}<AgentControls thread={workerRequest} request {agent} />{:else if agent?.role === 'solo'}<AgentControls {thread} {agent} />{/if}
</div>

<style>
  .wrap { position: relative; border-radius: 24px; }
  .wrap.dragging::after { content: 'Drop to attach'; position: absolute; inset: -4px; display: grid; place-items: center; border-radius: 28px; background: color-mix(in srgb, var(--surface-3) 85%, transparent); box-shadow: inset 0 0 0 1.5px var(--ink-3); color: var(--ink-1); font-size: var(--t-md); font-weight: var(--w-med); pointer-events: none; z-index: 2; }
  .atts { display: flex; flex-wrap: wrap; gap: 6px; padding: 0 6px 8px; }
  .att { display: inline-flex; align-items: center; gap: 6px; max-width: 220px; height: 30px; padding: 0 6px 0 5px; border-radius: var(--r-full); background: var(--surface-2); box-shadow: inset 0 0 0 1px var(--line); font-size: var(--t-sm); color: var(--ink-2); }
  .att img { width: 20px; height: 20px; border-radius: 6px; object-fit: cover; flex-shrink: 0; }
  .att.busy { opacity: 0.6; }
  .att.failed { color: var(--bad); }
  .an { min-width: 0; }
  .ax { width: 18px; height: 18px; display: grid; place-items: center; border-radius: 50%; color: var(--ink-3); flex-shrink: 0; }
  .ax:hover { background: var(--surface-3); color: var(--ink-1); }
  .attach { width: 34px; height: 34px; border-radius: 50%; display: grid; place-items: center; color: var(--ink-3); flex-shrink: 0; }
  .attach:hover { background: var(--surface-3); color: var(--ink-1); }
  .composer { display: flex; align-items: flex-end; gap: 8px; min-height: 48px; padding: 7px 7px 7px 18px; border-radius: 24px; background: var(--surface-2); box-shadow: inset 0 0 0 1px var(--line); transition: box-shadow var(--d-2) var(--ease-out), background-color var(--d-2); }
  .composer:focus-within { background: var(--surface-3); box-shadow: inset 0 0 0 1px var(--line-3); }
  .composer.one { padding-left: 8px; }
  .to { display: inline-flex; align-items: center; gap: 6px; height: 32px; padding: 0 10px 0 7px; margin-bottom: 1px; border-radius: var(--r-full); background: color-mix(in srgb, var(--hue) 18%, transparent); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--hue) 45%, transparent); font-size: var(--t-sm); font-weight: var(--w-semi); white-space: nowrap; flex-shrink: 0; }
  .to :global(svg:last-child) { color: var(--ink-3); }
  /* The field and its hint lie on each other. A hint longer than the field ends in an ellipsis: a second line would be cut by the one-row field. */
  .field { flex: 1; min-width: 0; display: grid; }
  .field > * { grid-area: 1 / 1; min-width: 0; }
  .hint { align-self: start; padding: 7px 0 6px; font-size: var(--t-lg); line-height: 21px; color: var(--ink-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; pointer-events: none; }
  textarea { width: 100%; resize: none; border: 0; outline: none; box-shadow: none; background: transparent; padding: 7px 0 6px; min-height: 34px; max-height: 200px; font-size: var(--t-lg); line-height: 21px; color: var(--ink-1); user-select: text; -webkit-user-select: text; }
  .send { width: 34px; height: 34px; border-radius: 50%; display: grid; place-items: center; color: var(--ink-3); background: var(--surface-2); flex-shrink: 0; transition: background var(--d-2), color var(--d-2), transform var(--d-2) var(--spring); }
  .send:disabled { opacity: 1; }
  .has .send { background: var(--aurora); color: var(--on-aurora); transform: scale(1.04); }
  /* Every action is listed: in a low window the list scrolls, and never grows past the top of the page. */
  .mentions { position: absolute; left: 6px; bottom: calc(100% + 8px); width: min(360px, calc(100% - 12px)); max-height: min(440px, 55vh); overflow-y: auto; padding: 5px; border-radius: var(--r-lg); background: var(--pop); box-shadow: var(--pop-shadow); z-index: var(--z-pop); animation: rise-in var(--d-3) var(--spring); }
  /* An action's row also says what it does, or why it cannot run now, in full: a long sentence takes a second line. */
  .mentions.actions { width: min(480px, calc(100% - 12px)); }
  .actions .opt { height: auto; min-height: 36px; padding: 6px 10px; }
  .actions .ol { min-width: 0; line-height: 18px; }
  .actions .os { flex-shrink: 0; }
  .nosuch { position: absolute; left: 6px; bottom: calc(100% + 8px); width: min(480px, calc(100% - 12px)); padding: 8px 12px; border-radius: var(--r-lg); background: var(--pop); box-shadow: var(--pop-shadow); font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-2); z-index: var(--z-pop); }
  .give { align-self: center; flex-shrink: 0; }
  .mh { padding: 6px 10px 4px; }
  .mh.bad { font-size: var(--t-sm); color: var(--bad); }
  .opt { display: flex; align-items: center; gap: 9px; width: 100%; height: 36px; padding: 0 10px; border-radius: var(--r-md); text-align: left; }
  .opt.active { background: var(--surface-3); }
  .opt :global(svg) { color: var(--ink-2); }
  .ol { flex: 1; font-weight: var(--w-med); }
  .oh { margin-left: 8px; font-weight: var(--w-reg); color: var(--ink-3); }
  .opt.off .ol, .opt.off :global(svg) { color: var(--ink-3); }
  .os { max-width: 60%; font-size: var(--t-sm); color: var(--ink-3); }
</style>
