<!--
  The conversation: the spine of the product. Everything important lands here as a rich,
  actionable card, in order. Streaming text grows in place; the view stays pinned to the newest
  line while you are there, and never jumps while you read further up.
  You can always write: a message to a busy agent shows as Queued (in order) until its turn
  starts; the first one offers Send now (interrupt the turn, so it runs next).
-->
<script lang="ts">
  import { tick } from 'svelte';
  import { store } from '../lib/state/store.svelte';
  import { narrate } from '../lib/state/narration';
  import { ui } from '../lib/state/ui.svelte';
  import { clock } from '../lib/util/format';
  import { COMPANY_LABEL, PROVIDER_LABEL, prettyModel } from '../lib/util/labels';
  import EmptyState from '../components/EmptyState.svelte';
  import Icon from '../components/Icon.svelte';
  import Markdown from '../components/Markdown.svelte';
  import Portrait from '../components/Portrait.svelte';
  import Say from '../components/Say.svelte';
  import ApprovalCard from './cards/ApprovalCard.svelte';
  import AskCard from './cards/AskCard.svelte';
  import PlanCard from './cards/PlanCard.svelte';
  import PlanDoneCard from './cards/PlanDoneCard.svelte';
  import TaskDoneCard from './cards/TaskDoneCard.svelte';
  import WorkRow from './cards/WorkRow.svelte';
  import Composer from './Composer.svelte';
  import NextRow from './NextRow.svelte';
  import { buildSpine, type Item } from './spine';
  import { resendChat, sendChat, sendQueuedNow } from '../lib/state/actions';
  import { queued, type Sent } from '../lib/state/queue';
  import { isRequestThread } from '../lib/state/ui.svelte';

  interface Props {
    thread: string;
  }
  let { thread }: Props = $props();

  const owner = $derived(store.threadAgent(thread));
  const isRequest = $derived(isRequestThread(thread));
  const items = $derived.by((): Item[] => {
    void store.rev;
    void store.liveRev;
    void store.outbox;
    return buildSpine(thread);
  });

  // ── your messages that wait for the agent's current turn to end ──
  const waiting = $derived.by(() => {
    void ui.now;
    const sent: Sent[] = items.flatMap((it) => (it.kind === 'user' && !it.outgoing?.error ? [{ key: it.key, ts: it.ts, text: it.msg.text }] : []));
    return queued(thread, sent);
  });
  const ownerName = $derived(owner ? owner.name : 'the Lead');
  /** The message whose "Send now" you pressed (until it leaves the queue). */
  let stopping = $state<string | null>(null);
  $effect(() => {
    if (stopping && !waiting.has(stopping)) stopping = null;
  });
  async function sendNow(key: string, agentId: string) {
    stopping = key;
    await sendQueuedNow(agentId);
  }

  /** Consecutive messages from the same agent share one header. */
  function showHeader(i: number): boolean {
    const it = items[i];
    if (it.kind !== 'say') return false;
    for (let j = i - 1; j >= 0; j--) {
      const p = items[j];
      if (p.kind === 'work') continue;
      return !(p.kind === 'say' && p.agentId === it.agentId && it.ts - p.ts < 180_000);
    }
    return true;
  }

  function mentionParts(text: string): { t: string; agent?: string; task?: string }[] {
    return text.split(/([@#][\w.-]+)/g).filter(Boolean).map((t) => {
      if (t.startsWith('@')) {
        const a = store.agents.find((x) => x.name.toLowerCase() === t.slice(1).toLowerCase());
        if (a) return { t, agent: a.id };
      } else if (t.startsWith('#')) {
        const k = store.tasks.find((x) => x.key.toLowerCase() === t.slice(1).toLowerCase());
        if (k) return { t, task: k.id };
      }
      return { t };
    });
  }

  // ── scroll: stick to the newest line while you are there ──
  let scroller: HTMLDivElement | undefined = $state();
  let atBottom = $state(true);
  let unseen = $state(false);
  let lastCount = 0;
  function onScroll() {
    if (!scroller) return;
    atBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 60;
    if (atBottom) unseen = false;
  }
  function toBottom(smooth = false) {
    if (!scroller) return;
    scroller.scrollTo({ top: scroller.scrollHeight, behavior: smooth && !ui.reducedMotion ? 'smooth' : 'auto' });
  }
  $effect(() => {
    void items;
    const grew = items.length > lastCount;
    lastCount = items.length;
    if (atBottom) tick().then(() => toBottom());
    else if (grew) unseen = true;
  });
  $effect(() => {
    void thread;
    atBottom = true;
    unseen = false;
    tick().then(() => toBottom());
  });
  // What you just sent is always in view.
  let sentCount = 0;
  $effect(() => {
    const n = store.outbox.filter((o) => o.thread === thread).length;
    if (n > sentCount) {
      atBottom = true;
      tick().then(() => toBottom(true));
    }
    sentCount = n;
  });

  const ownerNow = $derived(owner && !isRequest ? narrate(owner.id) : null);
  const backTo = $derived(ui.request);
  const EXAMPLES = [
    'Add a dark mode toggle to Settings, persisted per user',
    'Find and fix the flaky checkout e2e test',
    'Explain how auth and sessions work here',
  ];
</script>

<div class="conv">
  {#if !isRequest && owner}
    <header class="one">
      <Portrait agent={owner} size={26} />
      <div class="o-txt">
        <p class="o-name">1:1 with <b>{owner.name}</b><span class="o-prov">{PROVIDER_LABEL[owner.provider]} ({COMPANY_LABEL[owner.provider]}){owner.model ? ` · ${prettyModel(owner.model)}` : ''}</span></p>
        {#if ownerNow}<p class="o-now truncate"><Say parts={ownerNow.parts} /></p>{/if}
      </div>
      <button class="btn ghost sm" onclick={() => ui.selectAgent(owner.id)}>Details</button>
      <button class="btn ghost sm" onclick={() => ui.openRequest(backTo)}><Icon name="arrow-left" size={12} />Back to the request</button>
    </header>
  {/if}

  <div class="scroll" bind:this={scroller} onscroll={onScroll}>
    {#if !items.length}
      {#if thread === 'lead'}
        <div class="hero">
          <EmptyState title="What should the crew build?" body="The Lead reads the repo, proposes a plan you approve, and routes each task to the right agent. Nothing lands on your branch without you.">
            {#each EXAMPLES as ex (ex)}<button class="ex" onclick={() => sendChat('lead', ex)}>{ex}</button>{/each}
          </EmptyState>
        </div>
      {:else if isRequest}
        <div class="hero">
          <EmptyState title="A new request, in parallel." body="A Lead of its own reads the repo and plans this one while the other requests keep running. Its plan, approvals and results stay in this tab.">
            {#each EXAMPLES as ex (ex)}<button class="ex" onclick={() => sendChat(thread, ex)}>{ex}</button>{/each}
          </EmptyState>
        </div>
      {:else}
        <EmptyState title="{owner?.name ?? 'This agent'} is listening." body="Messages here go straight to this agent, outside any plan." />
      {/if}
    {:else}
      <ol class="items">
        {#each items as it, i (it.key)}
          <li class="it k-{it.kind}">
            {#if it.kind === 'user'}
              {@const q = waiting.get(it.key)}
              {@const failed = it.outgoing?.error ?? null}
              <div class="user selectable" class:queued={!!q} class:failed={!!failed}>
                <p>{#each mentionParts(it.msg.text) as part, pi (pi)}{#if part.agent}<button class="mention" onclick={() => ui.selectAgent(part.agent!)}>{part.t}</button>{:else if part.task}<button class="mention" onclick={() => ui.selectTask(part.task!)}>{part.t}</button>{:else}{part.t}{/if}{/each}</p>
                {#if q}
                  <div class="qmeta" role="status">
                    <Icon name="clock" size={12} />
                    <span><b>Queued{q.total > 1 ? ` · ${q.position} of ${q.total}` : ''}</b> — {q.reason ? q.reason : q.position === 1 ? `runs when ${ownerName} finishes this turn` : 'runs after the one above'}</span>
                    {#if q.position === 1 && q.agentId}
                      <button class="now" disabled={stopping === it.key} onclick={() => q.agentId && sendNow(it.key, q.agentId)} data-tip="Interrupt {ownerName}’s turn so this runs next">{stopping === it.key ? `Stopping ${ownerName}…` : 'Send now'}</button>
                    {/if}
                  </div>
                {:else if failed}
                  <div class="qmeta bad" role="alert">
                    <Icon name="alert" size={12} /><span>Not sent — {failed}</span>
                    <button class="now" onclick={() => it.outgoing && resendChat(it.outgoing.id)}>Retry</button>
                  </div>
                {/if}
              </div>
            {:else if it.kind === 'system'}
              {#if it.card?.type === 'plan'}<PlanCard planId={it.card.planId} />
              {:else if it.card?.type === 'plan-done'}<PlanDoneCard planId={it.card.planId} />
              {:else}<p class="sys">{it.text}<time class="num">{clock(it.ts)}</time></p>{/if}
            {:else if it.kind === 'say'}
              {@const a = store.agentMap.get(it.agentId)}
              {#if showHeader(i)}
                <div class="who">
                  <button class="face" onclick={() => a && ui.selectAgent(a.id)} aria-label="Open {a?.name ?? 'agent'}"><Portrait agent={a} size={22} /></button>
                  <span class="name">{a?.name ?? 'Agent'}</span>
                  {#if it.replyTo}{@const r = store.agentMap.get(it.replyTo)}<span class="reply"><Icon name="arrow-right" size={11} />answering {r?.name ?? 'an agent'}</span>{/if}
                  {#if !it.streaming}<time class="num">{clock(it.ts)}</time>{/if}
                </div>
              {/if}
              <div class="say" class:commentary={it.phase === 'commentary'}>
                <Markdown text={it.text} streaming={it.streaming} />
              </div>
              {#if it.card?.type === 'plan'}<div class="card-gap"><PlanCard planId={it.card.planId} /></div>
              {:else if it.card?.type === 'plan-done'}<div class="card-gap"><PlanDoneCard planId={it.card.planId} /></div>{/if}
            {:else if it.kind === 'work'}
              <WorkRow segs={it.segs} thoughts={it.thoughts} thinking={it.thinking} />
            {:else if it.kind === 'approval'}
              <ApprovalCard approval={it.approval} />
            {:else if it.kind === 'ask'}
              <AskCard msg={it.msg} agentId={it.agentId} />
            {:else if it.kind === 'task-done'}
              <TaskDoneCard taskId={it.taskId} />
            {/if}
          </li>
        {/each}
      </ol>
    {/if}
  </div>

  {#if unseen}
    <button class="newer" onclick={() => toBottom(true)}><Icon name="chevron-down" size={13} />New activity</button>
  {/if}

  <div class="bottom">
    <NextRow {thread} queued={waiting.size} />
    <Composer {thread} />
  </div>
</div>

<style>
  .conv { position: relative; flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .one { display: flex; align-items: center; gap: 10px; padding: 12px 14px 12px 22px; box-shadow: inset 0 -1px 0 var(--line); }
  .o-txt { flex: 1; min-width: 0; }
  .o-name { font-size: var(--t-md); color: var(--ink-2); display: flex; align-items: baseline; gap: 8px; }
  .o-name b { color: var(--ink-1); font-weight: var(--w-semi); }
  .o-prov { font-size: var(--t-xs); color: var(--ink-3); }
  .o-now { font-size: var(--t-sm); color: var(--ink-3); }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; overflow-anchor: none; padding: 28px 0 12px; scroll-padding-bottom: 20px; mask-image: linear-gradient(180deg, transparent 0, #000 28px, #000 calc(100% - 10px), transparent 100%); }
  .hero { height: 100%; display: grid; place-items: center; }
  .ex { height: 32px; padding: 0 14px; border-radius: var(--r-full); background: var(--surface-2); color: var(--ink-2); font-size: var(--t-sm); }
  .ex:hover { background: var(--surface-3); color: var(--ink-1); }
  .items { display: flex; flex-direction: column; gap: 20px; padding: 0 32px; }
  .it { min-width: 0; animation: rise-in var(--d-4) var(--spring) both; }
  .k-work { margin: -8px 0; }
  .user { display: flex; flex-direction: column; align-items: flex-end; gap: 6px; }
  .user p { max-width: 82%; padding: 10px 16px 11px; border-radius: 20px 20px 6px 20px; background: var(--surface-3); font-size: var(--t-lg); line-height: var(--lh-lg); white-space: pre-wrap; overflow-wrap: anywhere; transition: background-color var(--d-3), box-shadow var(--d-3), color var(--d-3); }
  .user.queued p { background: transparent; box-shadow: inset 0 0 0 1px var(--line-3); color: var(--ink-2); }
  .user.failed p { background: transparent; box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--bad) 55%, transparent); }
  .qmeta { display: flex; align-items: center; gap: 7px; max-width: 82%; font-size: var(--t-sm); color: var(--ink-3); animation: fade-in var(--d-3) var(--ease-out); }
  .qmeta b { font-weight: var(--w-semi); color: var(--ink-2); }
  .qmeta :global(svg) { flex-shrink: 0; }
  .qmeta.bad { color: var(--bad); }
  .now { height: 26px; padding: 0 11px; margin-left: 2px; border-radius: var(--r-full); font-size: var(--t-sm); font-weight: var(--w-semi); color: var(--ink-1); background: var(--surface-3); box-shadow: inset 0 0 0 1px var(--line-2); white-space: nowrap; }
  .now:hover { background: var(--surface-2); box-shadow: inset 0 0 0 1px var(--line-3); }
  .now:disabled { opacity: 1; color: var(--ink-3); }
  .mention { display: inline; padding: 0 3px; margin: 0 -1px; border-radius: 4px; color: var(--ink-1); font-weight: var(--w-semi); background: var(--surface-3); font-size: inherit; line-height: inherit; }
  .sys { display: flex; justify-content: center; gap: 8px; font-size: var(--t-sm); color: var(--ink-3); }
  .sys time { color: var(--ink-4); }
  .who { display: flex; align-items: center; gap: 8px; margin-bottom: 6px; }
  .face { display: grid; border-radius: 50%; }
  .name { font-weight: var(--w-semi); font-size: var(--t-md); }
  .reply { display: inline-flex; align-items: center; gap: 4px; font-size: var(--t-sm); color: var(--ink-3); }
  .who time { font-size: var(--t-xs); color: var(--ink-3); }
  .say { max-width: 66ch; }
  .say.commentary :global(.md) { font-size: var(--t-md); line-height: var(--lh-md); color: var(--ink-2); }
  .card-gap { margin-top: 14px; }
  .newer { position: absolute; left: 50%; bottom: 104px; transform: translateX(-50%); display: inline-flex; align-items: center; gap: 6px; height: 30px; padding: 0 14px; border-radius: var(--r-full); background: var(--pop); box-shadow: var(--pop-shadow); font-size: var(--t-sm); color: var(--ink-1); animation: rise-in var(--d-3) var(--spring); z-index: 2; }
  .bottom { padding: 0 20px 18px; }
</style>
