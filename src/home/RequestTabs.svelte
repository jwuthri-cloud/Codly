<!--
  Your requests, side by side: one tab per thing you asked for, each with its own Lead and plan
  running in parallel. Title = your first words; the dot says where its plan is (planning ·
  waiting for you · running · done), amber when it needs you. A small dot after the title says
  that rows came since you last had it on screen (the amber wins). ⌘[ / ⌘] switch, ⌘N starts one.
  × closes a request (its Lead stops). It asks first when the close leaves work behind: a
  running plan (closing cancels it), finished work that is not merged, an open pull request,
  a draft whose composer holds text. The question takes the keyboard: it opens on Keep, Esc
  keeps too, and the keyboard is back on a tab after the answer. In a narrow column the
  question's text is cut, never its two buttons; the whole text, and what becomes of the work,
  is in its tooltip and is the description of the Close button.
  A close can be taken back: the tab leaves at once and a toast offers Undo for a few seconds
  (lib/state/closing.ts). The clock at the end opens the requests you closed, to read again.
-->
<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { closeRequest } from '../lib/state/actions';
  import { closeQuestion, closeWithUndo, type CloseQuestion } from '../lib/state/closing';
  import { requests, type Request } from '../lib/state/requests.svelte';
  import { store } from '../lib/state/store.svelte';
  import { CLOSED_LIST, isClosedThread, isRequestThread, ui } from '../lib/state/ui.svelte';
  import Icon from '../components/Icon.svelte';
  import { hasNews } from './away';
  import { forgetSeen, seenAt } from './lastSeen';

  const list = $derived(requests.list);
  const office = $derived(store.workspace?.id ?? null);
  /** By request: when its newest row that is not yours came (a message, a card that asks you). */
  const newest = $derived.by(() => {
    const at = new Map<string, number>();
    const note = (thread: string, ts: number) => at.set(thread, Math.max(at.get(thread) ?? 0, ts));
    for (const c of store.chats) if (c.author !== 'user' && isRequestThread(c.thread)) note(c.thread, c.ts);
    for (const ap of store.approvals) note(store.approvalThread(ap), ap.created_at);
    return at;
  });
  /** A request you closed leaves the map of when you last had it on screen (an Undo brings back the tab, and it is stamped again once you leave it). */
  let shown: { office: string | null; threads: string[] } = { office: null, threads: [] };
  $effect(() => {
    const now = { office, threads: list.map((r) => r.thread) };
    const gone = shown.office === now.office ? shown.threads.filter((t) => !now.threads.includes(t)) : [];
    shown = now;
    if (gone.length) untrack(() => forgetSeen(now.office, gone));
  });
  let bar: HTMLDivElement | undefined = $state();
  /** You are reading the closed requests: no tab is the open one. */
  const reading = $derived(isClosedThread(ui.thread));
  /** The request waiting for "Close" / "Keep" with its question, and the empty one being closed. */
  let asking = $state<({ thread: string } & CloseQuestion) | null>(null);
  let closing = $state<string | null>(null);
  /** The question's Keep button: the keyboard starts on it, so Enter keeps. */
  let keepBtn: HTMLButtonElement | undefined = $state();
  /** The requests whose × is being looked into: a second press meanwhile asks nothing new. */
  const checking = new Set<string>();

  /** ×: a close that leaves work behind asks first. */
  async function x(r: Request) {
    if (checking.has(r.thread)) return;
    checking.add(r.thread);
    const q = await closeQuestion(r);
    checking.delete(r.thread);
    if (q) asking = { thread: r.thread, ...q };
    else await close(r);
  }

  async function close(r: Request) {
    const asked = asking?.thread === r.thread;
    asking = null;
    // A new request that holds text you did not send can come back too.
    if (r.phase !== 'new' || ui.draftRequests.some((d) => d.thread === r.thread && d.kept)) closeWithUndo(r.thread, r.title);
    else {
      // Nothing was asked in it: there is nothing to bring back.
      closing = r.thread;
      await closeRequest(r.thread);
      closing = null;
    }
    // The question's buttons are gone with the tab: the keyboard goes to the tab you are on now.
    if (asked) await focusTab('[aria-selected="true"]');
  }

  /** Keep (the button, or Esc): the question goes and the keyboard is back on the request's tab. */
  async function keep() {
    const thread = asking?.thread;
    asking = null;
    if (thread) await focusTab(`[data-thread="${CSS.escape(thread)}"]`);
  }
  async function focusTab(which: string) {
    await tick();
    bar?.querySelector<HTMLElement>(`[role="tab"]${which}`)?.focus();
  }
  function askKey(e: KeyboardEvent) {
    if (e.key !== 'Escape') return;
    // This Esc answers the question: it closes no sheet and stops no turn.
    e.preventDefault();
    e.stopPropagation();
    void keep();
  }
  $effect(() => {
    void ui.request;
    bar?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  });
  // A question is wider than its tab was: it comes into view with its buttons, and takes the keyboard.
  $effect(() => {
    if (!asking) return;
    bar?.querySelector('.ask')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    keepBtn?.focus();
  });
</script>

<nav class="tabs" aria-label="Requests">
  <div class="list" role="tablist" bind:this={bar}>
    {#each list as r (r.thread)}
      {@const on = !reading && ui.request === r.thread}
      {@const news = hasNews({ onScreen: ui.thread === r.thread, needsYou: r.needsYou, seen: seenAt(office, r.thread), newest: newest.get(r.thread) ?? null })}
      {#if asking?.thread === r.thread}
        <div class="ask" role="group" aria-label="{r.title}: {asking.ask}">
          <span class="q" data-tip="{asking.ask} {asking.detail}">{asking.ask}</span>
          <span class="sr-only" id="close-detail">{asking.detail}</span>
          <button class="btn sm danger" aria-describedby="close-detail" data-tip={asking.detail} onclick={() => close(r)} onkeydown={askKey}>{asking.yes}</button>
          <button class="btn sm ghost" bind:this={keepBtn} onclick={keep} onkeydown={askKey}>Keep</button>
        </div>
      {:else}
        <div class="tab p-{r.phase}" class:on class:amber={r.needsYou} class:closing={closing === r.thread} role="presentation">
          <button
            role="tab"
            aria-selected={on}
            class="pick"
            data-thread={r.thread}
            onclick={() => ui.openRequest(r.thread)}
            aria-label="{r.title} — {r.status}{news ? ' · new since you looked' : ''}"
            data-tip="{r.lead && r.lead.name !== 'Lead' ? `${r.lead.name} · ` : ''}{r.status}"
            data-tip-kbd="⌘[ ⌘]"
          >
            <span class="dot" aria-hidden="true"></span>
            <span class="t">{r.title}</span>
            {#if r.waiting}<span class="n num">{r.waiting}</span>{:else if news}<span class="new-dot" aria-hidden="true"></span>{/if}
          </button>
          <button
            class="x"
            aria-label="Close {r.title}"
            data-tip={r.phase === 'running' ? 'Close — cancels its running plan' : r.phase === 'new' ? 'Close' : 'Close — its Lead stops. You can undo for a few seconds'}
            disabled={closing === r.thread}
            onclick={() => x(r)}
          >
            <Icon name="x" size={12} stroke={2} />
          </button>
        </div>
      {/if}
    {/each}
  </div>
  <button class="new" onclick={() => ui.newRequest()} aria-label="New request" data-tip="New request — its own Lead plans it while the others keep going" data-tip-kbd="⌘N">
    <Icon name="plus" size={14} stroke={1.8} />
  </button>
  <button class="new" class:on={reading} onclick={() => ui.openThread(CLOSED_LIST)} aria-label="Closed requests" aria-pressed={reading} data-tip="Closed requests — read them again">
    <Icon name="clock" size={14} stroke={1.6} />
  </button>
</nav>

<style>
  .tabs { display: flex; align-items: center; gap: 8px; height: 46px; padding: 0 12px 0 14px; box-shadow: inset 0 -1px 0 var(--line); flex-shrink: 0; }
  .list { flex: 1; min-width: 0; display: flex; align-items: center; gap: 4px; overflow-x: auto; scrollbar-width: none; mask-image: linear-gradient(90deg, #000 calc(100% - 18px), transparent); }
  .list::-webkit-scrollbar { display: none; }
  /* While a question is open nothing fades at the list's end: its Keep button can be there. */
  .list:has(.ask) { mask-image: none; }
  /* Tabs get narrower (their title ends in an ellipsis) until the title has room for 3 letters
     and the ellipsis (3em); only then the list scrolls. Beside the title a tab holds 54px: 10,
     the dot 8, a gap 8, the title, 4, the close button 20, 4. A badge adds a gap 8 and itself
     (16, or 22 with two digits). */
  .tab { display: inline-flex; align-items: center; height: 30px; padding-right: 4px; border-radius: var(--r-full); max-width: 250px; min-width: calc(54px + 3em); flex-shrink: 1; color: var(--ink-3); font-size: var(--t-sm); font-weight: var(--w-med); transition: background-color var(--d-2), color var(--d-2), opacity var(--d-2); }
  .tab:has(.n) { min-width: calc(84px + 3em); }
  .tab:has(.new-dot) { min-width: calc(68px + 3em); }
  .tab:hover { background: var(--surface-2); color: var(--ink-1); }
  .tab.closing { opacity: 0.45; }
  .pick { display: inline-flex; align-items: center; gap: 8px; height: 100%; min-width: 0; padding: 0 4px 0 10px; color: inherit; font: inherit; }
  .x { display: grid; place-items: center; width: 20px; height: 20px; border-radius: 50%; flex-shrink: 0; color: var(--ink-3); opacity: 0; transition: opacity var(--d-2), background-color var(--d-2); }
  .tab:hover .x, .tab.on .x, .x:focus-visible { opacity: 1; }
  .x:hover { background: var(--surface-3); color: var(--ink-1); }
  /* Never wider than the list: the question's text gives way, its two buttons do not. */
  .ask { display: inline-flex; align-items: center; gap: 6px; height: 30px; max-width: 100%; padding: 0 4px 0 12px; border-radius: var(--r-full); flex-shrink: 0; background: var(--bad-soft); color: var(--ink-1); font-size: var(--t-sm); white-space: nowrap; }
  .q { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .ask .btn { flex-shrink: 0; }
  .tab.on { background: var(--surface-3); color: var(--ink-1); box-shadow: inset 0 0 0 1px var(--line-2); }
  .tab.amber { color: var(--amber-ink); }
  .tab.amber.on { background: color-mix(in srgb, var(--amber) 16%, var(--surface-2)); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--amber) 45%, transparent); }
  .t { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .dot { width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0; box-shadow: inset 0 0 0 1.5px var(--ink-4); }
  .p-planning .dot { background: var(--aurora); box-shadow: 0 0 0 3px var(--live-soft); animation: breathe 1.8s var(--ease-out) infinite; }
  .p-running .dot { background: var(--live); box-shadow: 0 0 0 3px var(--live-soft); }
  .p-done .dot { background: var(--ok); box-shadow: none; }
  .p-merged .dot { box-shadow: inset 0 0 0 1.5px var(--ok); }
  .p-failed .dot { background: var(--bad); box-shadow: none; }
  .tab.amber .dot { background: var(--amber); box-shadow: 0 0 0 3px color-mix(in srgb, var(--amber) 25%, transparent); animation: none; }
  /* Rows came since you last had it on screen. */
  .new-dot { flex-shrink: 0; width: 6px; height: 6px; border-radius: 50%; background: var(--ink-2); }
  .n { flex-shrink: 0; min-width: 16px; height: 16px; padding: 0 4px; border-radius: 8px; background: var(--amber); color: #241504; font-size: 10px; font-weight: 700; line-height: 16px; text-align: center; }
  .new { display: grid; place-items: center; width: 30px; height: 30px; border-radius: var(--r-full); color: var(--ink-2); flex-shrink: 0; }
  .new:hover { background: var(--surface-2); color: var(--ink-1); }
  .new.on { background: var(--surface-3); color: var(--ink-1); box-shadow: inset 0 0 0 1px var(--line-2); }
  @keyframes breathe { 50% { box-shadow: 0 0 0 5px transparent; } }
  @media (prefers-reduced-motion: reduce) { .p-planning .dot { animation: none; } }
</style>
