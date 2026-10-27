<!--
  One "ask me" card: who asks, whether it blocks their work or is a heads-up, the question in
  plain language, and a reply box. "Respond & unblock" sends your answer to the waiting agent
  (⌘↵); a heads-up needs nothing and is dismissed with "Got it". Answered cards show the reply.
-->
<script lang="ts">
  import type { Blocker } from '../lib/gen/Blocker';
  import { blockers, isOverdue, waitedDays } from '../lib/state/blockers.svelte';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { ago } from '../lib/util/format';
  import { idVar } from '../lib/util/labels';
  import Icon from '../components/Icon.svelte';
  import Markdown from '../components/Markdown.svelte';
  import Portrait from '../components/Portrait.svelte';

  interface Props {
    blocker: Blocker;
  }
  let { blocker }: Props = $props();

  const agent = $derived(store.agentMap.get(blocker.agent_id) ?? null);
  const task = $derived(blocker.task_id ? (store.taskMap.get(blocker.task_id) ?? null) : null);
  const open = $derived(blocker.status === 'open');
  const blocking = $derived(blocker.severity === 'blocking');
  const overdue = $derived(isOverdue(blocker, ui.now));
  const days = $derived(waitedDays(blocker, ui.now));

  let text = $state('');
  let busy = $state(false);
  let error = $state<string | null>(null);

  async function run(f: () => Promise<void>) {
    if (busy) return;
    busy = true;
    error = null;
    try {
      await f();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
  const send = () => text.trim() && run(() => blockers.answer(blocker.id, text.trim()));
  const dismiss = () => run(() => blockers.dismiss(blocker.id));
  function key(e: KeyboardEvent) {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      send();
    }
  }
</script>

<article class="card" class:blocking class:done={!open} style:--hue={idVar(agent?.color ?? 0)} aria-label={blocker.title}>
  <header>
    {#if agent}<button class="who" onclick={() => ui.selectAgent(agent.id)} aria-label="Open {agent.name}"><Portrait {agent} size={24} /></button>{/if}
    <div class="meta">
      <span class="by"><b>{agent?.name ?? 'An agent'}</b>{#if task} · {task.key}{/if}</span>
      <span class="when num">{ago(blocker.created_at, ui.now)}</span>
    </div>
    {#if !open}
      <span class="chip ok"><Icon name="check" size={11} stroke={2.2} />{blocker.status === 'answered' ? 'Answered' : blocking ? 'Skipped' : 'Read'}</span>
    {:else if blocking}
      <span class="chip warn"><Icon name="flag" size={11} />{task ? `Blocks ${task.key}` : 'Waiting for you'}</span>
    {:else}
      <span class="chip" data-tip="Nothing is waiting on this">Heads-up · no action needed today</span>
    {/if}
  </header>

  <h3>{blocker.title}</h3>
  <div class="body"><Markdown text={blocker.body} /></div>

  {#if overdue}
    <p class="late"><Icon name="alert" size={12} />{days >= 1 ? `Waiting ${days} ${days === 1 ? 'day' : 'days'}` : 'Waiting a while'}{blocking ? ': the agent is still stopped' : ''}</p>
  {/if}

  {#if open}
    {#if blocking}
      <textarea class="input" rows="2" placeholder="Your answer…" bind:value={text} onkeydown={key} disabled={busy} spellcheck="true"></textarea>
      <footer>
        <button class="btn primary" disabled={busy || !text.trim()} onclick={send}>Respond &amp; unblock<span class="k">⌘↵</span></button>
        <button class="btn ghost" disabled={busy} onclick={dismiss} data-tip="The agent carries on by its best judgment">Skip</button>
      </footer>
    {:else}
      <footer>
        <button class="btn" disabled={busy} onclick={dismiss}>Got it</button>
        <input class="input reply" placeholder="Or reply to {agent?.name ?? 'the agent'}…" bind:value={text} disabled={busy}
          onkeydown={(e) => e.key === 'Enter' && (e.preventDefault(), send())} />
        {#if text.trim()}<button class="btn" disabled={busy} onclick={send}>Send</button>{/if}
      </footer>
    {/if}
    {#if error}<p class="err">{error}</p>{/if}
  {:else if blocker.answer}
    <p class="answer"><Icon name="chat" size={12} /><span class="selectable">{blocker.answer}</span></p>
  {/if}
</article>

<style>
  .card { display: flex; flex-direction: column; gap: 10px; padding: 14px 16px; border-radius: var(--r-lg); background: var(--surface-1); box-shadow: inset 0 0 0 1px var(--line-2); }
  .card.blocking { background: linear-gradient(180deg, var(--amber-soft), transparent 70%), var(--surface-1); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--amber) 30%, transparent); }
  .card.done { opacity: 0.72; }
  header { display: flex; align-items: center; gap: 10px; min-width: 0; }
  .who { display: grid; border-radius: 50%; flex: none; }
  .meta { flex: 1; min-width: 0; display: flex; flex-direction: column; font-size: var(--t-sm); line-height: 16px; color: var(--ink-3); }
  .by { color: var(--ink-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .by b { font-weight: var(--w-semi); color: var(--ink-1); }
  .chip { flex: none; display: inline-flex; align-items: center; gap: 5px; height: 22px; padding: 0 9px; border-radius: var(--r-full); font-size: var(--t-xs); font-weight: var(--w-semi); color: var(--ink-2); background: var(--surface-2); white-space: nowrap; }
  .chip.warn { color: var(--amber-ink); background: var(--amber-soft); }
  .chip.ok { color: var(--ok); }
  h3 { font-size: var(--t-lg); line-height: var(--lh-lg); font-weight: var(--w-semi); color: var(--ink-1); }
  .body { font-size: var(--t-md); line-height: var(--lh-md); color: var(--ink-2); }
  .late { display: flex; align-items: center; gap: 6px; font-size: var(--t-sm); font-weight: var(--w-med); color: var(--amber-ink); }
  textarea.input { width: 100%; }
  footer { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .reply { flex: 1; min-width: 140px; }
  .k { margin-left: 8px; opacity: 0.7; font-size: var(--t-xs); }
  .answer { display: flex; gap: 8px; align-items: flex-start; padding: 8px 10px; border-radius: var(--r-md); background: var(--surface-2); font-size: var(--t-md); line-height: var(--lh-md); color: var(--ink-1); }
  .answer :global(svg) { margin-top: 4px; color: var(--ink-3); flex: none; }
  .err { font-size: var(--t-sm); color: var(--bad); }
</style>
