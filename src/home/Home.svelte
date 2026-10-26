<!--
  Home: the office behind, the conversation in a centered glass column in front, the crew on
  top. Each request you make is its own tab (its own Lead and plan, in parallel); the running
  plan of the request you are on is pinned above its conversation. In full-office mode (⌘1) the
  column steps aside and a small dock keeps you one click from the Lead and anything waiting.
-->
<script lang="ts">
  import { requests } from '../lib/state/requests.svelte';
  import { store } from '../lib/state/store.svelte';
  import { isRequestThread, ui } from '../lib/state/ui.svelte';
  import { oneLine } from '../lib/util/sanitize';
  import Icon from '../components/Icon.svelte';
  import PlanHeader from './cards/PlanHeader.svelte';
  import Conversation from './Conversation.svelte';
  import RequestTabs from './RequestTabs.svelte';

  const thread = $derived(isRequestThread(ui.thread) || store.threadAgent(ui.thread) ? ui.thread : ui.request);
  const isRequest = $derived(isRequestThread(thread));
  const pinned = $derived.by(() => {
    if (!isRequest) return null;
    void store.rev;
    const p = store.planOf(thread);
    if (!p) return null;
    if (p.status === 'running' || p.status === 'integrating') return p.id;
    if (p.status === 'done' && !store.planMerges[p.id] && !p.merged_sha && !p.kept && !ui.keptPlans.includes(p.id)) return p.id;
    return null;
  });
  /** Tabs once there is something to switch between (or one request with history). */
  const tabs = $derived(isRequest && (requests.list.length > 1 || requests.list.some((r) => r.phase !== 'new')));
  const waiting = $derived(store.pendingApprovals[0] ?? null);
  const waitingAgent = $derived(waiting ? store.agentMap.get(waiting.agent_id) : null);
  function review() {
    if (!waiting) return;
    ui.openRequest(store.approvalThread(waiting));
    ui.revealApproval = waiting.id;
  }
</script>

{#if ui.mode === 'office'}
  <div class="dock">
    {#if waiting}
      <button class="d-wait" onclick={review}>
        <Icon name="flag" size={13} /><b>{waitingAgent?.name ?? 'An agent'}</b> needs you:
        <code class="truncate">{waiting.subject.kind === 'command' ? oneLine(waiting.subject.command, 48) : waiting.subject.kind === 'question' ? 'a question' : waiting.subject.kind.replace('_', ' ')}</code>
        <span class="go">Review</span>
      </button>
    {/if}
    <button class="d-ask" onclick={() => (ui.mode = 'blend')}><Icon name="chat" size={14} />Talk to the Lead<kbd>⌘1</kbd></button>
  </div>
{:else}
  <div class="home">
    <section class="column glass" aria-label="Conversation">
      {#if tabs}<RequestTabs />{/if}
      {#if pinned}<PlanHeader planId={pinned} />{/if}
      <Conversation {thread} />
    </section>
  </div>
{/if}

<style>
  .home { flex: 1; min-height: 0; display: flex; justify-content: center; }
  .column { width: min(var(--col-w), 100%); flex: 0 1 var(--col-w); display: flex; flex-direction: column; min-height: 0; border-radius: var(--r-2xl); overflow: hidden; animation: column-in var(--d-4) var(--spring); }
  :global(.mode-work) .column { background: var(--glass-solid); }
  @keyframes column-in { from { opacity: 0; transform: translateY(14px); } to { opacity: 1; transform: none; } }
  .dock { position: absolute; left: 0; right: 0; bottom: 22px; display: flex; flex-direction: column; align-items: center; gap: 8px; pointer-events: none; }
  .dock > * { pointer-events: auto; }
  .d-ask, .d-wait { display: inline-flex; align-items: center; gap: 8px; height: 40px; padding: 0 16px; border-radius: var(--r-full); background: var(--glass); -webkit-backdrop-filter: var(--glass-blur); backdrop-filter: var(--glass-blur); box-shadow: var(--glass-edge), var(--glass-shadow); font-size: var(--t-md); color: var(--ink-1); animation: rise-in var(--d-4) var(--spring); max-width: min(620px, 90%); }
  .d-wait { background: color-mix(in srgb, var(--amber) 18%, var(--glass)); box-shadow: var(--glass-edge), inset 0 0 0 1px color-mix(in srgb, var(--amber) 50%, transparent), var(--glass-shadow); }
  .d-wait :global(svg) { color: var(--amber); }
  .d-wait code { font-size: 12px; color: var(--ink-1); }
  .go { margin-left: 6px; font-weight: var(--w-semi); color: var(--amber-ink); }
  .d-ask kbd { margin-left: 4px; }
</style>
