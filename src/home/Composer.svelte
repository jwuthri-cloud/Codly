<!--
  The composer: one line that grows. ↵ sends, ⇧↵ breaks the line. It never locks: the message
  shows at once, and if the agent is mid-turn it queues (the conversation says so). Start with
  `@agent` to talk to one agent 1:1 (the thread switches and shows as a chip); `@` mid-sentence
  mentions, `#` refers to a task.
-->
<script lang="ts">
  import { tick } from 'svelte';
  import { api } from '../lib/api';
  import { sendChat, setRequestPrefs } from '../lib/state/actions';
  import { toast } from '../lib/state/toasts.svelte';
  import { busy } from '../lib/state/queue';
  import { store } from '../lib/state/store.svelte';
  import { isRequestThread, ui } from '../lib/state/ui.svelte';
  import { fuzzy } from '../lib/util/fuzzy';
  import { idVar, PROVIDER_LABEL, TASK_STATUS_LABEL } from '../lib/util/labels';
  import Icon from '../components/Icon.svelte';
  import DictationButton from './DictationButton.svelte';
  import { appendTranscript } from '../lib/util/freeflow';
  import Portrait from '../components/Portrait.svelte';
  import LeadControls, { MODES } from './LeadControls.svelte';
  import AgentControls from './AgentControls.svelte';

  interface Props {
    thread: string;
  }
  let { thread }: Props = $props();

  const drafts: Record<string, string> = {};
  let text = $state('');

  // ── attachments: pasted, dropped or picked files, saved in the repo; the message names their paths ──
  interface Att { id: number; name: string; path: string | null; url: string | null; failed: boolean }
  const fileDrafts: Record<string, Att[]> = {};
  let files = $state<Att[]>([]);
  let picker: HTMLInputElement | undefined = $state();
  let dragging = $state(false);
  let seq = 0;
  const saving = $derived(files.some((f) => !f.path && !f.failed));
  const ready = $derived(files.filter((f) => f.path));
  const canSend = $derived(!saving && (!!text.trim() || ready.length > 0));

  function addFiles(list: File[]) {
    for (const f of list) {
      files.push({ id: ++seq, name: f.name || 'pasted file', path: null, url: f.type.startsWith('image/') ? URL.createObjectURL(f) : null, failed: false });
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
  /** Where a draft lives: thread names repeat across offices, so a draft is its office's too. */
  const slot = $derived(`${store.workspace?.id ?? ''}|${thread}`);
  let lastSlot = '';

  $effect(() => {
    if (slot !== lastSlot) {
      if (lastSlot) {
        drafts[lastSlot] = text;
        fileDrafts[lastSlot] = files;
      }
      text = drafts[slot] ?? '';
      files = fileDrafts[slot] ?? [];
      lastSlot = slot;
      tick().then(resize);
    }
  });
  $effect(() => {
    const seed = ui.composerSeed;
    if (seed && seed.thread === thread) {
      text = seed.text;
      ui.composerSeed = null;
      tick().then(() => {
        resize();
        ta?.focus();
        ta?.setSelectionRange(text.length, text.length);
      });
    }
  });

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
  const placeholder = $derived(
    isBusy ? `${agent?.name ?? 'The agent'} is mid-turn — your message queues and runs next`
    : oneToOne ? `Message ${agent?.name ?? 'the agent'} directly…`
    : fresh && thread !== 'lead' ? 'Describe the new request — its own Lead plans it in parallel…'
    : 'Ask the Lead to build, fix or explain something…',
  );

  // ── mentions ──
  type Opt = { id: string; kind: 'agent' | 'task'; insert: string; label: string; sub: string; agentId?: string };
  let mention = $state<{ trigger: '@' | '#'; start: number; query: string; atStart: boolean } | null>(null);
  let mi = $state(0);
  const options = $derived.by((): Opt[] => {
    if (!mention) return [];
    const pool: Opt[] =
      mention.trigger === '@'
        ? store.agents.filter((a) => a.status !== 'offline').map((a) => ({ id: a.id, kind: 'agent' as const, insert: `@${a.name} `, label: a.name, sub: a.role === 'lead' ? 'Lead' : PROVIDER_LABEL[a.provider], agentId: a.id }))
        : store.tasks.map((t) => ({ id: t.id, kind: 'task' as const, insert: `#${t.key} `, label: `${t.key} · ${t.title}`, sub: TASK_STATUS_LABEL[t.status] }));
    return pool
      .map((o) => ({ o, m: fuzzy(mention!.query, o.label) }))
      .filter((x) => x.m)
      .sort((a, b) => b.m!.score - a.m!.score)
      .slice(0, 6)
      .map((x) => x.o);
  });

  function detect() {
    if (!ta) return;
    const pos = ta.selectionStart ?? 0;
    const before = text.slice(0, pos);
    const m = /(^|\s)([@#])([\w.-]*)$/.exec(before);
    if (m) {
      const start = pos - m[3].length - 1;
      mention = { trigger: m[2] as '@' | '#', start, query: m[3], atStart: m[2] === '@' && before.slice(0, start).trim() === '' };
      mi = 0;
    } else mention = null;
  }

  function pick(o: Opt) {
    if (!mention || !ta) return;
    const end = ta.selectionStart ?? text.length;
    if (o.kind === 'agent' && mention.atStart) {
      // "@Ada" at the start: talk to Ada 1:1.
      const rest = text.slice(end).trimStart();
      mention = null;
      text = '';
      ui.openThread(store.threadOf(o.agentId!));
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

  function resize() {
    if (!ta) return;
    ta.style.height = 'auto';
    ta.style.height = Math.min(200, ta.scrollHeight) + 'px';
  }

  /** Never blocks: clear the line, show the message, send in the background. */
  function send() {
    if (!canSend) return;
    const list = ready.map((f) => `- ${f.path}`).join('\n');
    const t = [text.trim() || (list ? 'See the attached files.' : ''), list && `Attached files (read them by path):\n${list}`].filter(Boolean).join('\n\n');
    sendChat(thread, t);
    for (const f of files) if (f.url) URL.revokeObjectURL(f.url);
    files = [];
    fileDrafts[slot] = [];
    text = '';
    drafts[slot] = '';
    mention = null;
    tick().then(resize);
  }

  /** A transcript from Free Flow joins the draft for review. It is never sent for you. */
  function dictated(forSlot: string, transcript: string) {
    if (forSlot !== slot) {
      drafts[forSlot] = appendTranscript(drafts[forSlot] ?? '', transcript);
      return;
    }
    text = appendTranscript(text, transcript);
    tick().then(() => {
      resize();
      ta?.focus();
      ta?.setSelectionRange(text.length, text.length);
    });
  }

  function key(e: KeyboardEvent) {
    if (e.isComposing) return; // IME: Enter confirms the composition, it doesn't send
    if (mention && options.length) {
      if (e.key === 'ArrowDown') { e.preventDefault(); mi = (mi + 1) % options.length; return; }
      if (e.key === 'ArrowUp') { e.preventDefault(); mi = (mi - 1 + options.length) % options.length; return; }
      if (e.key === 'Enter' || e.key === 'Tab') { e.preventDefault(); pick(options[mi]); return; }
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); mention = null; return; }
    }
    // ⇧⇥ cycles the request's mode, like Claude Code.
    if (e.key === 'Tab' && e.shiftKey && !oneToOne) {
      e.preventDefault();
      const now = store.requestPrefs[thread]?.mode ?? (store.settings.auto_run_plans ? 'auto' : 'plan');
      setRequestPrefs(thread, { mode: MODES[(MODES.findIndex((m) => m.id === now) + 1) % MODES.length].id });
      return;
    }
    if (e.key === 'Backspace' && !text && oneToOne && !e.repeat) {
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
  {#if mention && options.length}
    <ul class="mentions" role="listbox" aria-label={mention.trigger === '@' ? (mention.atStart ? 'Talk to' : 'Mention an agent') : 'Mention a task'}>
      {#if mention.atStart}<li class="mh eyebrow">Talk 1:1 with</li>{/if}
      {#each options as o, i (o.id)}
        <li>
          <button class="opt" class:active={i === mi} role="option" aria-selected={i === mi} onpointerenter={() => (mi = i)} onmousedown={(e) => { e.preventDefault(); pick(o); }}>
            {#if o.agentId}<Portrait agent={store.agentMap.get(o.agentId)} size={20} />{:else}<Icon name="plan" size={14} />{/if}
            <span class="ol truncate">{o.label}</span>
            <span class="os">{o.sub}</span>
          </button>
        </li>
      {/each}
    </ul>
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
  <div class="composer" class:has={canSend} class:one={oneToOne} style:--hue={idVar(agent?.color ?? 0)}>
    {#if oneToOne && agent}
      <button class="to" onclick={() => ui.openRequest(ui.request)} data-tip="Back to the request (⌫ on an empty line)">
        <Portrait {agent} size={18} />{agent.name}<Icon name="x" size={11} stroke={2} />
      </button>
    {/if}
    <textarea
      bind:this={ta}
      bind:value={text}
      rows="1"
      {placeholder}
      spellcheck="true"
      aria-label={placeholder}
      oninput={() => { resize(); detect(); }}
      onkeydown={key}
      onclick={detect}
      onpaste={paste}
      onblur={() => setTimeout(() => (mention = null), 120)}
    ></textarea>
    <input bind:this={picker} type="file" multiple hidden onchange={() => { addFiles([...(picker?.files ?? [])]); if (picker) picker.value = ''; }} />
    <button class="attach" onclick={() => picker?.click()} aria-label="Attach files" data-tip="Attach images or files — or paste or drop them"><Icon name="paperclip" size={15} stroke={1.6} /></button>
    <DictationButton target={ta} thread={slot} onText={dictated} />
    <button class="send" disabled={!canSend} onclick={send} aria-label={isBusy ? 'Queue' : 'Send'} data-tip={isBusy ? 'Send — it queues until the turn ends' : 'Send'} data-tip-kbd="↵">
      <Icon name="send" size={15} stroke={1.8} />
    </button>
  </div>
  {#if !oneToOne}<LeadControls {thread} />{:else if workerRequest}<AgentControls thread={workerRequest} request />{:else if agent?.role === 'solo'}<AgentControls {thread} />{/if}
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
  textarea { flex: 1; resize: none; border: 0; outline: none; box-shadow: none; background: transparent; padding: 7px 0 6px; min-height: 34px; max-height: 200px; font-size: var(--t-lg); line-height: 21px; color: var(--ink-1); user-select: text; -webkit-user-select: text; }
  .send { width: 34px; height: 34px; border-radius: 50%; display: grid; place-items: center; color: var(--ink-3); background: var(--surface-2); flex-shrink: 0; transition: background var(--d-2), color var(--d-2), transform var(--d-2) var(--spring); }
  .send:disabled { opacity: 1; }
  .has .send { background: var(--aurora); color: var(--on-aurora); transform: scale(1.04); }
  .mentions { position: absolute; left: 6px; bottom: calc(100% + 8px); width: min(360px, calc(100% - 12px)); padding: 5px; border-radius: var(--r-lg); background: var(--pop); box-shadow: var(--pop-shadow); z-index: var(--z-pop); animation: rise-in var(--d-3) var(--spring); }
  .mh { padding: 6px 10px 4px; }
  .opt { display: flex; align-items: center; gap: 9px; width: 100%; height: 36px; padding: 0 10px; border-radius: var(--r-md); text-align: left; }
  .opt.active { background: var(--surface-3); }
  .opt :global(svg) { color: var(--ink-2); }
  .ol { flex: 1; font-weight: var(--w-med); }
  .os { font-size: var(--t-sm); color: var(--ink-3); white-space: nowrap; }
</style>
