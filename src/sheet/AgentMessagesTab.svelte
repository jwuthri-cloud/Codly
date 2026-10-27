<!-- Messages: the conversation thread with this agent, and a reply box that sends to it. -->
<script lang="ts">
  import { tick } from 'svelte';
  import type { Agent } from '../lib/gen/Agent';
  import { sendChat } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { clock } from '../lib/util/format';
  import { clean } from '../lib/util/sanitize';
  import Icon from '../components/Icon.svelte';

  interface Props {
    agent: Agent;
  }
  let { agent }: Props = $props();

  const thread = $derived(store.threadOf(agent.id));
  type Line = { key: string; who: string; mine: boolean; text: string; ts: number; pending: boolean; error: string | null };
  const lines = $derived.by((): Line[] => {
    void store.rev;
    const who = (author: string) => (author === 'user' ? 'You' : author === 'codly' ? 'Codly' : (store.agentMap.get(author)?.name ?? agent.name));
    const sent: Line[] = store.chats
      .filter((c) => c.thread === thread)
      .map((c) => ({ key: c.id, who: who(c.author), mine: c.author === 'user', text: c.text, ts: c.ts, pending: false, error: null }));
    const queued: Line[] = store.outbox
      .filter((o) => o.thread === thread)
      .map((o) => ({ key: `o:${o.id}`, who: 'You', mine: true, text: o.text, ts: o.ts, pending: !o.error, error: o.error }));
    return [...sent, ...queued].slice(-80);
  });

  let draft = $state('');
  let list: HTMLDivElement | undefined = $state();
  $effect(() => {
    void lines.length;
    tick().then(() => list && (list.scrollTop = list.scrollHeight));
  });
  function send() {
    if (sendChat(thread, draft)) draft = '';
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      send();
    }
  }
</script>

<div class="msgs">
  <div class="list" bind:this={list}>
    {#each lines as l (l.key)}
      <div class="m" class:mine={l.mine} class:pending={l.pending}>
        <p class="who">{l.who} · <time class="num">{clock(l.ts)}</time>{#if l.pending} · sending{/if}{#if l.error} · <span class="bad">not sent</span>{/if}</p>
        <p class="tx selectable">{clean(l.text)}</p>
      </div>
    {:else}
      <p class="empty">No messages with {agent.name} yet.</p>
    {/each}
  </div>
  <div class="reply">
    <textarea class="input" rows="2" placeholder="Reply to {agent.name}…" bind:value={draft} onkeydown={onKey}></textarea>
    <button class="btn sm" disabled={!draft.trim()} onclick={send} aria-label="Send"><Icon name="send" size={13} />Send</button>
  </div>
</div>

<style>
  .msgs { display: flex; flex-direction: column; gap: 10px; }
  .list { display: flex; flex-direction: column; gap: 8px; max-height: 320px; overflow-y: auto; }
  .m { max-width: 92%; padding: 7px 10px; border-radius: var(--r-md); background: var(--surface-1); align-self: flex-start; }
  .m.mine { align-self: flex-end; background: var(--surface-2); }
  .m.pending { opacity: 0.6; }
  .who { font-size: var(--t-xs); color: var(--ink-3); }
  .bad { color: var(--bad); }
  .tx { margin-top: 2px; font-size: var(--t-md); line-height: var(--lh-md); color: var(--ink-1); white-space: pre-wrap; overflow-wrap: anywhere; }
  .empty { font-size: var(--t-sm); color: var(--ink-3); }
  .reply { display: flex; gap: 8px; align-items: flex-end; }
  .reply textarea { flex: 1; min-width: 0; height: auto; padding: 8px 12px; resize: none; }
</style>
