<!--
  Something went wrong in the agent's turn, said where it happened: what (in plain words), the
  agent's own reason in full, and what you can do: send your message again, copy the reason,
  or open the agent's history at that event.
-->
<script lang="ts">
  import { copyText } from '../../lib/state/actions';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { clock } from '../../lib/util/format';
  import { clean } from '../../lib/util/sanitize';
  import Icon from '../../components/Icon.svelte';
  import { troubleTitle, type TroubleKind } from '../outcomes';

  interface Props {
    agentId: string;
    eventId: number;
    ts: number;
    what: TroubleKind;
    reason: string | null;
    /** Send your message again (absent: nothing to send again here). */
    onretry?: () => void;
  }
  let { agentId, eventId, ts, what, reason, onretry }: Props = $props();
  const name = $derived(store.agentMap.get(agentId)?.name ?? 'The agent');
  const why = $derived(clean(reason).trim());
  const tone = $derived(what === 'failed' || what === 'stopped' ? 'bad' : what === 'interrupted' ? 'quiet' : 'warn');
</script>

<div class="trouble t-{tone}">
  <Icon name={what === 'interrupted' ? 'pause' : 'alert'} size={14} />
  <div class="body">
    <p class="head"><b>{troubleTitle(what, name)}</b><time class="num">{clock(ts)}</time></p>
    {#if why}<p class="why selectable">{why}</p>{/if}
    <div class="acts">
      {#if onretry}<button class="act" onclick={onretry}>Retry</button>{/if}
      {#if why}<button class="act" onclick={() => copyText(why, 'Error copied')}>Copy</button>{/if}
      <button class="act" onclick={() => ui.selectAgent(agentId, eventId)}>Details</button>
    </div>
  </div>
</div>

<style>
  .trouble { display: flex; align-items: flex-start; gap: 10px; padding: 10px 14px; border-radius: var(--r-md); background: var(--surface-2); box-shadow: inset 0 0 0 1px var(--line); color: var(--ink-3); }
  .trouble > :global(svg) { flex-shrink: 0; margin-top: 2px; }
  .t-bad { background: var(--bad-soft); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--bad) 40%, transparent); color: var(--bad); }
  .t-warn { color: var(--amber-ink); }
  .body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .head { display: flex; align-items: baseline; gap: 8px; font-size: var(--t-md); }
  .head b { font-weight: var(--w-semi); color: var(--ink-1); }
  .head time { font-size: var(--t-xs); color: var(--ink-3); }
  .why { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-2); white-space: pre-wrap; overflow-wrap: anywhere; }
  .acts { display: flex; gap: 4px; margin: 2px 0 0 -8px; }
  .act { height: 24px; padding: 0 8px; border-radius: var(--r-full); font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-2); }
  .act:hover { background: var(--surface-3); color: var(--ink-1); }
</style>
