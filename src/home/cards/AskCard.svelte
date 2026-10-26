<!-- A worker asked the Lead (codly_ask_lead): attributed, quoted, answered right below. -->
<script lang="ts">
  import type { ChatMessage } from '../../lib/gen/ChatMessage';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { clock } from '../../lib/util/format';
  import { idVar } from '../../lib/util/labels';
  import InlineText from '../../components/InlineText.svelte';
  import Portrait from '../../components/Portrait.svelte';

  interface Props {
    msg: ChatMessage;
    agentId: string;
  }
  let { msg, agentId }: Props = $props();
  const agent = $derived(store.agentMap.get(agentId) ?? null);
  const task = $derived(agent?.task_id ? store.taskMap.get(agent.task_id) : null);
</script>

<div class="ask" style:--hue={idVar(agent?.color ?? 0)}>
  <button class="who" onclick={() => agent && ui.selectAgent(agent.id)}><Portrait {agent} size={20} /><b>{agent?.name ?? 'An agent'}</b></button>
  <span class="meta">asked the Lead{task ? ` while on ${task.key}` : ''} · <time class="num">{clock(msg.ts)}</time></span>
  <p class="q selectable"><InlineText text={msg.text} /></p>
</div>

<style>
  .ask { display: grid; grid-template-columns: auto 1fr; align-items: center; gap: 4px 8px; padding: 12px 16px 13px; border-radius: var(--r-lg); background: color-mix(in srgb, var(--hue) 8%, transparent); box-shadow: inset 2px 0 0 var(--hue); }
  .who { display: inline-flex; align-items: center; gap: 7px; font-size: var(--t-md); }
  .who b { font-weight: var(--w-semi); }
  .meta { font-size: var(--t-sm); color: var(--ink-3); }
  .q { grid-column: 1 / -1; font-size: var(--t-lg); line-height: var(--lh-lg); color: var(--ink-1); }
</style>
