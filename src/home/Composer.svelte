<!--
  The composer: one line that grows. ↵ sends, ⇧↵ breaks the line. It never locks: the message
  shows at once, and if the agent is mid-turn it queues (the conversation says so). Start with
  `@agent` to talk to one agent 1:1 (the thread switches and shows as a chip); `@` mid-sentence
  mentions, `#` refers to a task.
-->
<script lang="ts">
  import { tick } from 'svelte';
  import { sendChat, setRequestPrefs } from '../lib/state/actions';
  import { busy } from '../lib/state/queue';
  import { store } from '../lib/state/store.svelte';
  import { isRequestThread, ui } from '../lib/state/ui.svelte';
  import { fuzzy } from '../lib/util/fuzzy';
  import { idVar, PROVIDER_LABEL, TASK_STATUS_LABEL } from '../lib/util/labels';
  import Icon from '../components/Icon.svelte';
  import Portrait from '../components/Portrait.svelte';
  import LeadControls, { MODES } from './LeadControls.svelte';

  interface Props {
    thread: string;
  }
  let { thread }: Props = $props();

  const drafts: Record<string, string> = {};
  let text = $state('');
  let ta: HTMLTextAreaElement | undefined = $state();
  let lastThread = '';

  $effect(() => {
    if (thread !== lastThread) {
      if (lastThread) drafts[lastThread] = text;
      text = drafts[thread] ?? '';
      lastThread = thread;
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
    const t = text.trim();
    if (!t) return;
    sendChat(thread, t);
    text = '';
    drafts[thread] = '';
    mention = null;
    tick().then(resize);
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
    if (e.key === 'Backspace' && !text && oneToOne) {
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

<div class="wrap">
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
  <div class="composer" class:has={!!text.trim()} class:one={oneToOne} style:--hue={idVar(agent?.color ?? 0)}>
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
      onblur={() => setTimeout(() => (mention = null), 120)}
    ></textarea>
    <button class="send" disabled={!text.trim()} onclick={send} aria-label={isBusy ? 'Queue' : 'Send'} data-tip={isBusy ? 'Send — it queues until the turn ends' : 'Send'} data-tip-kbd="↵">
      <Icon name="send" size={15} stroke={1.8} />
    </button>
  </div>
  {#if !oneToOne}<LeadControls {thread} />{/if}
</div>

<style>
  .wrap { position: relative; }
  .composer { display: flex; align-items: flex-end; gap: 8px; min-height: 48px; padding: 7px 7px 7px 18px; border-radius: 24px; background: var(--surface-2); box-shadow: inset 0 0 0 1px var(--line); transition: box-shadow var(--d-2) var(--ease-out), background-color var(--d-2); }
  .composer:focus-within { background: var(--surface-3); box-shadow: inset 0 0 0 1px var(--line-3); }
  .composer.one { padding-left: 8px; }
  .to { display: inline-flex; align-items: center; gap: 6px; height: 32px; padding: 0 10px 0 7px; margin-bottom: 1px; border-radius: var(--r-full); background: color-mix(in srgb, var(--hue) 18%, transparent); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--hue) 45%, transparent); font-size: var(--t-sm); font-weight: var(--w-semi); white-space: nowrap; flex-shrink: 0; }
  .to :global(svg:last-child) { color: var(--ink-3); }
  textarea { flex: 1; resize: none; border: 0; outline: none; background: transparent; padding: 7px 0 6px; min-height: 34px; max-height: 200px; font-size: var(--t-lg); line-height: 21px; color: var(--ink-1); user-select: text; -webkit-user-select: text; }
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
