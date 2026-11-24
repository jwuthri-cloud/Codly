<!--
  The conversation: the spine of the product. Everything important lands here as a rich,
  actionable card, in order. Streaming text grows in place; the view stays pinned to the newest
  line while you are there, and never jumps while you read further up.
  You can always write: a message to a busy agent shows as Queued (in order) until the agent
  takes it (Claude Code and Codex after their current step, the other CLIs as the next turn);
  the first one offers Send now (interrupt the turn, so the agent reads it at once).
  An error or a turn that failed is a line of its own, where it happened, with the reason. Its
  Retry (also in the head of a 1:1 thread) sends your message again: that copy is a small
  "Sent again" line, not a second bubble (retryFold.ts).
  What the agent's CLI told about the run (a hook, a goal, an MCP server that did not start) is
  a quiet line; a click opens the agent's history there. All it told in one turn is one line
  that opens into the list (a hook that speaks on every tool call does not fill the thread).
  Hover a message (or tab to it) to copy it; one of yours can go back to the composer.
  A card that waits stays in view through a change of layout (a panel opens or closes, the
  window is resized): the thread brings it back once the layout is at rest. The "new below" pill
  never lies on a waiting card's buttons: it goes to the thread's edge, or above the buttons.
  When you come back after 10 minutes or more (the page hidden, the window without the focus, or
  a thread you open again), one row before the first new one says what came while you were away;
  each of its facts takes you to its row (away.ts, AwayRow.svelte). What you did to the agent (a
  compaction, its mode, its model, a context reading) is a quiet row too (controlRows.ts).
-->
<script module lang="ts">
  import { SvelteMap } from 'svelte/reactivity';
  import type { AwayWindow } from './away';
  /** By office and thread (`awayKey`): when you left the thread and came back, while its summary row stays (× takes it away). */
  const awayOf = new SvelteMap<string, AwayWindow>();
  const awayKey = (office: string | null, thread: string) => `${office ?? ''}\n${thread}`;
  /** The key of the summary's row. */
  const AWAY_KEY = 'away';
  /** How long a row the summary took you to stays marked, in ms. */
  const MARK_MS = 1600;
</script>

<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { store } from '../lib/state/store.svelte';
  import { narrate } from '../lib/state/narration';
  import { ui, wallNow } from '../lib/state/ui.svelte';
  import { clock } from '../lib/util/format';
  import { COMPANY_LABEL, PROVIDER_LABEL, prettyModel } from '../lib/util/labels';
  import EmptyState from '../components/EmptyState.svelte';
  import Icon from '../components/Icon.svelte';
  import Markdown from '../components/Markdown.svelte';
  import Portrait from '../components/Portrait.svelte';
  import Say from '../components/Say.svelte';
  import RetryButton from '../components/RetryButton.svelte';
  import ApprovalCard from './cards/ApprovalCard.svelte';
  import AskCard from './cards/AskCard.svelte';
  import NoticeRow from './cards/NoticeRow.svelte';
  import PlanCard from './cards/PlanCard.svelte';
  import PlanDoneCard from './cards/PlanDoneCard.svelte';
  import TaskDoneCard from './cards/TaskDoneCard.svelte';
  import TroubleRow from './cards/TroubleRow.svelte';
  import WorkRow from './cards/WorkRow.svelte';
  import AwayRow from './AwayRow.svelte';
  import { Presence, awayKeys, awaySummary, withAway, type AwayNames, type Spot } from './away';
  import { loadSeen, seenAt, stampSeen } from './lastSeen';
  import Composer from './Composer.svelte';
  import LoadEarlier from './LoadEarlier.svelte';
  import NextRow from './NextRow.svelte';
  import { buildSpine, keepRows, openStreams, streamText, thinkingText, type Item } from './spine';
  import { LAYOUT_REST_MS, meets, newBelowSorted, nextMove, oldestWaiting, pageKey, pillSpot, revealTop, scrolledUpByHand, unseenFrom, type Rect } from './threadScroll';
  import { headRetry, retryNow, sentAgain } from './retryFold';
  import { sysMarks } from './sysMarks';
  import { boxOf, partsOf } from './cards/cardParts';
  import { todoNow } from './todoLine';
  import { copyText, resendChat, sendChat, sendQueuedNow } from '../lib/state/actions';
  import { busy, queued, queuedHint, sendNowTip, type Sent } from '../lib/state/queue';
  import { isRequestThread } from '../lib/state/ui.svelte';

  interface Props {
    thread: string;
  }
  let { thread }: Props = $props();

  const owner = $derived(store.threadAgent(thread));
  const isRequest = $derived(isRequestThread(thread));
  // The rows are built again when something of this thread changed: a message, an approval, a
  // task, a stored event of its agent, a message or a thought its agent starts or ends. A row
  // that shows the same thing keeps its object, so only the rows that changed are updated.
  // Another agent's stream, and each new word of what this one writes, build nothing.
  const streams = $derived(owner ? openStreams(owner.id) : '');
  let rows: Item[] = [];
  const spine = $derived.by((): Item[] => {
    void streams;
    if (owner) void store.revBy.get(owner.id);
    return (rows = keepRows(rows, buildSpine(thread)));
  });

  // ── what came while you were away: one row, before the first row newer than your leave ──
  const office = $derived(store.workspace?.id ?? null);
  /** Names for the summary, read without tracking: the rows change when what they name does.
   *  A plan's merge is read with tracking: its done row stays the same when you merge it. */
  const names: AwayNames = {
    agent: (id) => untrack(() => store.agentMap.get(id)?.name ?? 'An agent'),
    task: (id) => untrack(() => {
      const t = store.taskMap.get(id);
      return t ? { key: t.key, assignee: t.assignee } : null;
    }),
    planMerged: (id) => {
      // This session's merges are no reactive state: `rev` moves with each.
      void store.rev;
      return !!store.mergeOf(id);
    },
  };
  const away = $derived.by(() => {
    const w = awayOf.get(awayKey(office, thread));
    const summary = w ? awaySummary(spine, w, names) : null;
    return w && summary ? { w, summary } : null;
  });
  const items = $derived(away ? withAway(spine, { kind: 'away', key: AWAY_KEY, ts: away.w.since, summary: away.summary }) : spine);
  /** The page shows and the window has the focus: the thread on screen is looked at. */
  const isPresent = () => document.visibilityState === 'visible' && document.hasFocus();
  const presence = new Presence(
    (s) => seenAt(s.office, s.thread),
    (s, at) => stampSeen(s.office, s.thread, at),
  );
  /** You came back to the thread after `w`: its summary row stays until × or your next return. */
  function cameBack(w: AwayWindow | null, s: Spot | null) {
    if (w && s) awayOf.set(awayKey(s.office, s.thread), w);
  }
  // The thread is on screen until you open another one (or leave Home).
  $effect(() => {
    const t = thread;
    const o = office;
    untrack(() => {
      if (o) loadSeen(o);
      const s = o ? { office: o, thread: t } : null;
      cameBack(presence.show(s, isPresent(), wallNow()), s);
    });
    return () => untrack(() => presence.show(null, false, wallNow()));
  });
  // While you look, the thread is stamped once a minute (on the clock's tick).
  $effect(() => {
    void ui.now;
    untrack(() => presence.tick(wallNow()));
  });
  // The page hidden, or the window without the focus, is a leave too; the page going away as well.
  $effect(() => {
    const change = () => {
      const s = office ? { office, thread } : null;
      cameBack(presence.setPresent(isPresent(), wallNow()), s);
    };
    const gone = () => presence.setPresent(false, wallNow());
    document.addEventListener('visibilitychange', change);
    window.addEventListener('focus', change);
    window.addEventListener('blur', change);
    window.addEventListener('pagehide', gone);
    return () => {
      document.removeEventListener('visibilitychange', change);
      window.removeEventListener('focus', change);
      window.removeEventListener('blur', change);
      window.removeEventListener('pagehide', gone);
    };
  });
  /** A fact of the summary was pressed: its row comes into view, marked for a moment. */
  let marked = $state<string | null>(null);
  let markTimer: ReturnType<typeof setTimeout> | undefined;
  function jumpTo(key: string) {
    const el = rowEl(key);
    if (!el) return;
    el.scrollIntoView({ block: 'center', behavior: ui.reducedMotion ? 'auto' : 'smooth' });
    marked = key;
    clearTimeout(markTimer);
    markTimer = setTimeout(() => (marked = null), MARK_MS);
  }
  $effect(() => () => clearTimeout(markTimer));

  // ── your messages that the agent has not taken yet ──
  /** Your messages that were sent, from the rows (the clock's tick does not make them again). */
  const sent = $derived(spine.flatMap((it): Sent[] => (it.kind === 'user' && !it.outgoing?.error ? [{ key: it.key, ts: it.ts, text: it.msg.text }] : [])));
  const waiting = $derived.by(() => {
    // What `queued` reads, but the clock: the time only lets a message go (an agent idle a few
    // seconds has taken it), so the clock counts only while a message waits.
    void store.threadAgent(thread);
    void store.rev;
    void store.chats;
    const out = untrack(() => queued(thread, sent));
    if (out.size) void ui.now;
    return out;
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

  // ── Retry on a failed turn: only while nothing was said after it, and the agent is free ──
  const isBusy = $derived(busy(thread));
  /** The same Retry in the head of a 1:1 thread (one on a task has the task's Retry there). */
  const headAgain = $derived(isRequest ? null : headRetry(spine, isBusy));
  /** Your messages that send the one before them again: a small row each (retryFold.ts). */
  const sentAgainKeys = $derived(sentAgain(spine));

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
  let lastFirst: string | undefined;
  let lastTop = 0;
  /** Text is streaming into the newest message: it grows below the fold without a new item,
   *  and a smooth scroll to it would end short (so "New activity" jumps there instead). */
  const streamingLast = $derived.by(() => {
    const last = items[items.length - 1];
    return last?.kind === 'say' && last.streaming;
  });
  // ── while a card waits for you: it stays where it is (see threadScroll.ts) ──
  /** The oldest card that waits for your answer: new rows do not move it. */
  const waitingCard = $derived(oldestWaiting(items));
  /** The card the thread holds now, and its top in the scroller's box (px from the box's top). */
  let heldKey: string | null = null;
  let heldAt = 0;
  /** You were at the newest line when the thread began to hold a card, and did not scroll up since: it goes back there after the last one. */
  let followed = false;
  /** The rows there were when you were last at the newest line: the others below the view are counted. */
  let seen = new Set<string>();
  /** The rows `seen` was last made from (`seeAll`): while they are the same, it is not made again. */
  let seenOf: Item[] | null = null;
  /** What the summary of your time away last added to `seen`: its window and the rows it read. */
  let toldOf: { w: AwayWindow; spine: Item[] } | null = null;
  let below = $state(0);
  const rowEl = (key: string) => list?.querySelector<HTMLElement>(`:scope > li[data-key="${CSS.escape(key)}"]`) ?? null;
  // From the layout, not the screen box: the newest row rises in (a transform) and must not count.
  // A row and the scroller have the same offset parent (`.conv`).
  const offsetOf = (el: HTMLElement) => (scroller ? el.offsetTop - scroller.offsetTop - scroller.scrollTop : 0);
  const atEnd = () => !!scroller && scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 2;
  /** All rows are seen: you are at the newest line. */
  function seeAll() {
    if (seenOf === items) return;
    seenOf = items;
    seen = new Set(items.map((it) => it.key));
    newFrom = null;
  }
  /** The rows' keys and tops in the scroller's content (a scroll does not move them), read once
   *  and kept until a row comes or goes or a size changes (`forgetRows`). */
  let rowsAt: { keys: string[]; tops: number[] } | null = null;
  /** Per row, how many of the rows from it on are not in `seen` (`unseenFrom`). */
  let newFrom: number[] | null = null;
  const forgetRows = () => {
    rowsAt = null;
    newFrom = null;
  };
  function countBelow() {
    if (!scroller || !list) return (below = 0);
    if (!rowsAt) {
      const kids = [...list.children] as HTMLElement[];
      const base = scroller.offsetTop;
      rowsAt = { keys: kids.map((li) => li.dataset.key ?? ''), tops: kids.map((li) => li.offsetTop - base) };
      newFrom = null;
    }
    // The summary of your time away shows: the rows it speaks of are told, and not counted as new.
    const summary = away ? rowEl(AWAY_KEY) : null;
    if (away && summary && (!toldOf || toldOf.w !== away.w || toldOf.spine !== spine) && meets({ top: 0, bottom: scroller.clientHeight }, { top: offsetOf(summary), bottom: offsetOf(summary) + summary.offsetHeight })) {
      toldOf = { w: away.w, spine };
      seen = new Set([...seen, ...awayKeys(spine, away.w)]);
      newFrom = null;
    }
    newFrom ??= unseenFrom(rowsAt.keys, seen);
    // The bottom padding fades out: a row that begins there is not in view yet.
    below = newBelowSorted(rowsAt.tops, newFrom, scroller.scrollTop + scroller.clientHeight - parseFloat(getComputedStyle(scroller).paddingBottom));
    cancelAnimationFrame(pillFrame);
    pillFrame = requestAnimationFrame(placePill);
  }
  /** A scroll counts the rows below at most once per frame. */
  let countFrame = 0;
  function countSoon() {
    if (countFrame) return;
    countFrame = requestAnimationFrame(() => {
      countFrame = 0;
      countBelow();
    });
  }
  $effect(() => () => cancelAnimationFrame(countFrame));
  // A row that grows or shrinks moves the rows after it.
  $effect(() => {
    if (!list) return;
    const ro = new ResizeObserver(forgetRows);
    ro.observe(list);
    return () => ro.disconnect();
  });

  // ── the pill keeps off the waiting card's buttons ──
  let bottomEl: HTMLDivElement | undefined = $state();
  let pill: HTMLButtonElement | undefined = $state();
  /** Where the pill is (see `pillSpot`): 0 centred, 1 at the thread's edge, 2 there and `lift` px higher, above the buttons. */
  let pillAt = $state({ spot: 0, lift: 0 });
  let pillFrame = 0;
  /** Its gap to what it sits over, and to the thread's edge, in px. */
  const PILL_GAP = 8;
  const PILL_EDGE = 12;
  function placePill() {
    const card = waitingCard ? rowEl(waitingCard)?.querySelector<HTMLElement>('article.card') : null;
    const parts = card ? partsOf(card) : null;
    if (!pill || !bottomEl || !parts) {
      pillAt = { spot: 0, lift: 0 };
      return;
    }
    const boxes = parts.map(boxOf);
    const buttons = boxes[2];
    const b = bottomEl.getBoundingClientRect();
    const w = pill.offsetWidth;
    const h = pill.offsetHeight;
    const base = b.top - PILL_GAP;
    const at = (left: number, bottom: number): Rect => ({ left, right: left + w, top: bottom - h, bottom });
    const edgeLeft = b.right - PILL_EDGE - w;
    const lift = Math.max(0, base - (buttons.top - PILL_GAP));
    const spot = pillSpot([at(b.left + (b.width - w) / 2, base), at(edgeLeft, base), at(edgeLeft, base - lift)], boxes, buttons);
    pillAt = { spot, lift: spot === 2 ? lift : 0 };
  }
  /** The held card showed in the view, where it was last held or scrolled to. */
  const heldShows = (el: HTMLElement) => !!scroller && meets({ top: 0, bottom: scroller.clientHeight }, { top: heldAt, bottom: heldAt + el.offsetHeight });
  /** The held card back where it was, after rows came in or grew above it. Only when it moved:
   *  a write to scrollTop ends a smooth scroll on its way (to a card, to the newest line). */
  function hold(el: HTMLElement) {
    if (!scroller) return;
    const moved = offsetOf(el) - heldAt;
    if (moved) scrollOwn(scroller.scrollTop + moved);
  }
  /** The card that became the oldest waiting one, whole in view: its command and its buttons. */
  function reveal(el: HTMLElement) {
    if (!scroller) return;
    const top = offsetOf(el);
    const box = { top, bottom: top + el.offsetHeight };
    const css = getComputedStyle(scroller);
    // The thread's bottom padding is the room of the "new below" pill: the card stays above it.
    scrollOwn(revealTop(scroller.scrollTop, { top: 0, bottom: scroller.clientHeight }, box, parseFloat(css.paddingTop), parseFloat(css.paddingBottom)));
  }
  /** Where the thread last scrolled itself to, as the scroller took it: the scroll event that
   *  comes of it is not yours (see `scrolledUpByHand`). */
  let ownTop: number | null = null;
  function scrollOwn(top: number) {
    if (!scroller) return;
    scroller.scrollTop = top;
    ownTop = scroller.scrollTop;
  }
  /** After the rows changed: hold the waiting card, or follow the newest line while you are there (see `nextMove`). */
  function settle(added: boolean) {
    if (!scroller) return;
    const key = waitingCard;
    const held = heldKey !== null;
    if (key && !held) followed = atBottom;
    const el = key ? rowEl(key) : null;
    const move = nextMove({ card: !!el, fresh: key !== heldKey, held, inView: !!el && heldShows(el), atBottom, followed, added: added || streamingLast });
    heldKey = key;
    if (!key) followed = false;
    if (el && move === 'hold') hold(el);
    else if (el && move === 'reveal') reveal(el);
    else if (move === 'bottom') {
      atBottom = true;
      toBottom();
    } else if (move === 'mark') unseen = true;
    if (el) heldAt = offsetOf(el);
    // Rows that came in below the held card: you are not at the newest line any more.
    if (move === 'hold' || move === 'reveal') atBottom = atEnd();
    if (atBottom) seeAll();
    countBelow();
  }
  function onScroll() {
    if (!scroller) return;
    // Any scroll up lets go of the newest line; coming back down within 60px takes it again.
    const d = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight;
    atBottom = d < 2 || (scroller.scrollTop >= lastTop && d < 60);
    // You scrolled up yourself while a card was held: the thread does not take you back to the
    // newest line after the last answer.
    if (scrolledUpByHand({ before: lastTop, now: scroller.scrollTop, own: ownTop, atEnd: d < 2 })) followed = false;
    ownTop = null;
    lastTop = scroller.scrollTop;
    if (atBottom) {
      unseen = false;
      seeAll();
    }
    // Where you scrolled the held card to is where it stays.
    const el = heldKey ? rowEl(heldKey) : null;
    if (el) heldAt = offsetOf(el);
    countSoon();
  }
  function toBottom(smooth = false) {
    if (!scroller) return;
    scroller.scrollTo({ top: scroller.scrollHeight, behavior: smooth && !ui.reducedMotion ? 'smooth' : 'auto' });
    // A smooth scroll gets there in steps: each goes down, and only a scroll up can be yours.
    ownTop = scroller.scrollHeight - scroller.clientHeight;
  }
  // A message or a thought being written grows its row without a change of the list: while the
  // thread's agent writes, the height of the list says when to follow, at most once per frame
  // (or, while a card waits, to keep the card where it is: the row may grow above it).
  // Not at any other time: a row you open yourself also grows the list, and must stay in place.
  let list: HTMLOListElement | undefined = $state();
  $effect(() => {
    if (!list || !streams) return;
    const grown = new ResizeObserver(() => {
      const el = heldKey ? rowEl(heldKey) : null;
      if (el && heldShows(el)) {
        hold(el);
        forgetRows();
        countBelow();
      } else if (atBottom) toBottom();
    });
    grown.observe(list);
    return () => grown.disconnect();
  });
  $effect(() => {
    void items;
    // Rows that came in above ("Load earlier") are not new activity.
    const grew = items.length > lastCount && items[0]?.key === lastFirst;
    lastCount = items.length;
    lastFirst = items[0]?.key;
    tick().then(() => {
      forgetRows();
      settle(grew);
    });
  });
  // A change of layout (a panel opens or closes, the window is resized) moves the rows: once
  // the thread's size has stayed the same for a moment, the waiting card that showed before is
  // brought back into view.
  $effect(() => {
    if (!scroller) return;
    const sc = scroller;
    let size = [sc.clientWidth, sc.clientHeight];
    let shown = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const ro = new ResizeObserver(() => {
      if (sc.clientWidth === size[0] && sc.clientHeight === size[1]) return;
      size = [sc.clientWidth, sc.clientHeight];
      if (timer === undefined) {
        // Where it was held before the change: did it show?
        const el = heldKey ? rowEl(heldKey) : null;
        shown = !!el && heldShows(el);
      }
      clearTimeout(timer);
      timer = setTimeout(() => {
        timer = undefined;
        const el = heldKey ? rowEl(heldKey) : null;
        if (shown && el) {
          reveal(el);
          heldAt = offsetOf(el);
          atBottom = atEnd();
        }
        forgetRows();
        countBelow();
      }, LAYOUT_REST_MS);
    });
    ro.observe(sc);
    return () => {
      ro.disconnect();
      clearTimeout(timer);
    };
  });
  $effect(() => {
    void thread;
    atBottom = true;
    unseen = false;
    heldKey = null;
    followed = false;
    tick().then(() => {
      forgetRows();
      settle(false);
      // What the thread held when you opened it is not new, wherever the view is.
      seenOf = null;
      seeAll();
      countBelow();
    });
  });
  // What you just sent is in view, unless a waiting card shows: the card stays where it is (its
  // keys aim at it), and what you sent is counted below.
  let sentCount = 0;
  $effect(() => {
    const n = store.outbox.filter((o) => o.thread === thread).length;
    const held = untrack(() => (heldKey ? rowEl(heldKey) : null));
    if (n > sentCount && !(held && heldShows(held))) {
      atBottom = true;
      // While a card waits out of view, at once: a smooth scroll would end where the card is held.
      tick().then(() => toBottom(!heldKey));
    }
    sentCount = n;
  });

  // PageDown, Space, PageUp, Home and End scroll the thread while the focus is on the page (on
  // no field, button or card: those take their own keys).
  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!scroller || e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.target !== document.body && e.target !== document.documentElement) return;
      const to = pageKey(e.key, e.shiftKey, scroller.scrollTop, scroller.clientHeight, scroller.scrollHeight);
      if (to == null) return;
      e.preventDefault();
      scroller.scrollTo({ top: to, behavior: ui.reducedMotion ? 'auto' : 'smooth' });
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  /** The wheel over a box in a row that keeps the wheel (a card's body that scrolls) and is at
   *  its end: the browser moves nothing, so the thread takes the wheel. */
  function onWheel(e: WheelEvent) {
    if (!scroller || e.ctrlKey || !e.deltaY) return;
    for (let el = e.target as HTMLElement | null; el && el !== scroller; el = el.parentElement) {
      if (el.scrollHeight <= el.clientHeight) continue;
      const css = getComputedStyle(el);
      if (!/auto|scroll/.test(css.overflowY)) continue;
      // A box that passes the wheel on does so by itself.
      if (css.overscrollBehaviorY === 'auto') return;
      const atItsEnd = e.deltaY > 0 ? el.scrollTop + el.clientHeight >= el.scrollHeight - 1 : el.scrollTop <= 0;
      if (atItsEnd) scroller.scrollTop += e.deltaMode === 1 ? e.deltaY * 16 : e.deltaMode === 2 ? e.deltaY * scroller.clientHeight : e.deltaY;
      return;
    }
  }

  const ownerNow = $derived(owner && !isRequest ? narrate(owner.id) : null);
  /** Its own todo list while it works, and greyed while it waits for you: "Step 3 of 7 · <the step it is on>". */
  const ownerTodo = $derived.by(() => {
    if (!owner) return null;
    void store.revBy.get(owner.id);
    return todoNow(ownerNow?.tone, store.live[owner.id]?.todo ?? []);
  });
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
        {#if ownerNow}<p class="o-now truncate">{#if ownerTodo}<span class="o-todo num" class:waits={ownerTodo.waiting}>{`${ownerTodo.text} · `}</span>{/if}<Say parts={ownerNow.parts} /></p>{/if}
      </div>
      <RetryButton task={owner.task_id ? store.taskMap.get(owner.task_id) : null} />
      {#if headAgain && !owner.task_id}{@const again = headAgain}<button class="btn sm" onclick={() => sendChat(thread, again)} data-tip="Send your message again">Retry</button>{/if}
      <button class="btn ghost sm" onclick={() => ui.selectAgent(owner.id)}>Details</button>
      <button class="btn ghost sm" onclick={() => ui.openRequest(backTo)}><Icon name="arrow-left" size={12} />Back to the request</button>
    </header>
  {/if}

  <div class="scroll" bind:this={scroller} onscroll={onScroll} onwheel={onWheel}>
    <LoadEarlier {thread} {scroller} />
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
      <ol class="items" bind:this={list}>
        {#each items as it, i (it.key)}
          <li class="it k-{it.kind}" class:marked={marked === it.key} data-key={it.key}>
            {#if it.kind === 'user'}
              {@const q = waiting.get(it.key)}
              {@const failed = it.outgoing?.error ?? null}
              {#if !q && sentAgainKeys.has(it.key)}
                <p class="again">Sent again<time class="num">{clock(it.ts)}</time></p>
              {:else}
              <div class="user selectable" class:queued={!!q} class:failed={!!failed}>
                <p>{#each mentionParts(it.msg.text) as part, pi (pi)}{#if part.agent}<button class="mention" onclick={() => ui.selectAgent(part.agent!)}>{part.t}</button>{:else if part.task}<button class="mention" onclick={() => ui.selectTask(part.task!)}>{part.t}</button>{:else}{part.t}{/if}{/each}</p>
                {#if q}
                  <div class="qmeta" role="status">
                    <Icon name="clock" size={12} />
                    <span><b>Queued{q.total > 1 ? ` · ${q.position} of ${q.total}` : ''}</b> — {q.reason ? q.reason : q.position === 1 ? queuedHint(ownerName, owner?.provider) : 'runs after the one above'}</span>
                    {#if q.position === 1 && q.agentId}
                      <button class="now" disabled={stopping === it.key} onclick={() => q.agentId && sendNow(it.key, q.agentId)} data-tip={sendNowTip(ownerName, owner?.provider)}>{stopping === it.key ? `Stopping ${ownerName}…` : 'Send now'}</button>
                    {/if}
                  </div>
                {:else if failed}
                  <div class="qmeta bad" role="alert">
                    <Icon name="alert" size={12} /><span>Not sent — {failed}</span>
                    <button class="now" onclick={() => it.outgoing && resendChat(it.outgoing.id)}>Retry</button>
                  </div>
                {/if}
              </div>
              {#if !q && !failed}
                <div class="acts mine">
                  <button class="act" onclick={() => copyText(it.msg.text, 'Message copied')}><Icon name="copy" size={11} />Copy</button>
                  <button class="act" onclick={() => ui.seedComposer(thread, it.msg.text, true)} data-tip="Put it back in the composer, to change and send again">Edit</button>
                </div>
              {/if}
              {/if}
            {:else if it.kind === 'system'}
              {#if it.card?.type === 'plan'}<PlanCard planId={it.card.planId} />
              {:else if it.card?.type === 'plan-done'}<PlanDoneCard planId={it.card.planId} />
              {:else}<p class="sys"><span>{#each sysMarks(it.text) as m, mi (mi)}{#if m.code}<code>{m.t}</code>{:else if m.bold}<b>{m.t}</b>{:else}{m.t}{/if}{/each}</span><time class="num">{clock(it.ts)}</time></p>{/if}
            {:else if it.kind === 'say'}
              {@const a = store.agentMap.get(it.agentId)}
              {#if it.header}
                <div class="who">
                  <button class="face" onclick={() => a && ui.selectAgent(a.id)} aria-label="Open {a?.name ?? 'agent'}"><Portrait agent={a} size={22} /></button>
                  <span class="name">{a?.name ?? 'Agent'}</span>
                  {#if it.replyTo}{@const r = store.agentMap.get(it.replyTo)}<span class="reply"><Icon name="arrow-right" size={11} />answering {r?.name ?? 'an agent'}</span>{/if}
                  {#if !it.streaming}<time class="num">{clock(it.ts)}</time>{/if}
                </div>
              {/if}
              <div class="say" class:commentary={it.phase === 'commentary'}>
                <Markdown text={streamText(it)} streaming={it.streaming} />
              </div>
              {#if it.card?.type === 'plan'}<div class="card-gap"><PlanCard planId={it.card.planId} /></div>
              {:else if it.card?.type === 'plan-done'}<div class="card-gap"><PlanDoneCard planId={it.card.planId} /></div>{/if}
              {#if !it.streaming}
                <div class="acts">
                  <button class="act" onclick={() => copyText(it.text, 'Message copied')}><Icon name="copy" size={11} />Copy</button>
                </div>
              {/if}
            {:else if it.kind === 'trouble'}
              {@const retry = retryNow(items, i, isBusy)}
              <TroubleRow agentId={it.agentId} eventId={it.eventId} ts={it.ts} what={it.what} reason={it.reason} onretry={retry ? () => sendChat(thread, retry) : undefined} />
            {:else if it.kind === 'notice' || it.kind === 'control'}
              <NoticeRow agentId={it.agentId} notes={it.notes} />
            {:else if it.kind === 'away'}
              <AwayRow summary={it.summary} onjump={jumpTo} ondismiss={() => awayOf.delete(awayKey(office, thread))} />
            {:else if it.kind === 'work'}
              <WorkRow segs={it.segs} thoughts={it.thoughts} thinking={thinkingText(it)} helpers={it.helpers} />
            {:else if it.kind === 'approval'}
              <ApprovalCard approval={it.approval} twins={it.twins} />
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

  <div class="bottom" bind:this={bottomEl}>
    {#if below}
      <button class="newer" class:edge={pillAt.spot > 0} style:--lift="{pillAt.lift}px" bind:this={pill} onclick={() => toBottom(!streamingLast && !heldKey)}><Icon name="chevron-down" size={13} /><span class="num">{below}</span> new below</button>
    {:else if unseen}
      <button class="newer" class:edge={pillAt.spot > 0} style:--lift="{pillAt.lift}px" bind:this={pill} onclick={() => toBottom(!streamingLast)}><Icon name="chevron-down" size={13} />New activity</button>
    {/if}
    <NextRow {thread} queued={waiting.size} />
    <Composer {thread} />
  </div>
</div>

<style>
  .conv { position: relative; flex: 1; min-height: 0; display: flex; flex-direction: column; }
  /* In a narrow panel (a small window with the sheet open) the last button goes to a second row,
     so the name keeps 170px or more and nothing lies on anything. */
  .one { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 10px; padding: 12px 14px 12px 22px; box-shadow: inset 0 -1px 0 var(--line); }
  .o-txt { flex: 1 1 170px; min-width: 0; }
  /* The name stays on one line: the CLI and model give way first, then the name, each with an ellipsis.
     The name itself does not shrink: it is whole up to the room of the line less "1:1 with" and
     the two gaps (5em), and cut only past that. */
  .o-name { font-size: var(--t-md); color: var(--ink-2); display: flex; align-items: baseline; gap: 8px; min-width: 0; white-space: nowrap; }
  .o-name b { flex: 0 0 auto; max-width: calc(100% - 5em); overflow: hidden; text-overflow: ellipsis; color: var(--ink-1); font-weight: var(--w-semi); }
  .o-prov { flex: 0 1000 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; font-size: var(--t-xs); color: var(--ink-3); }
  .one > .btn { flex-shrink: 0; }
  .one > .btn:last-child { margin-left: auto; }
  .o-now { font-size: var(--t-sm); color: var(--ink-3); }
  .o-todo { color: var(--ink-2); }
  .o-todo.waits { color: var(--ink-3); }
  /* The bottom padding is the room of the "new below" pill (a gap of 8, its 30, a gap of 8): the
     last row, or a card brought into view, ends above it (round 7: the pill lay 10 px on a card). */
  .scroll { flex: 1; min-height: 0; overflow-y: auto; overflow-anchor: none; padding: 28px 0 46px; scroll-padding-bottom: 20px; mask-image: linear-gradient(180deg, transparent 0, #000 28px, #000 calc(100% - 10px), transparent 100%); }
  .hero { height: 100%; display: grid; place-items: center; }
  .ex { height: 32px; padding: 0 14px; border-radius: var(--r-full); background: var(--surface-2); color: var(--ink-2); font-size: var(--t-sm); }
  .ex:hover { background: var(--surface-3); color: var(--ink-1); }
  .items { display: flex; flex-direction: column; gap: 20px; padding: 0 32px; }
  .it { position: relative; min-width: 0; }
  /* ponytail: only the newest item rises in, so a thread switch does not replay the whole history. Two items added at once animate only the last; animating each needs a "new since mount" flag per item. */
  .it:last-child { animation: rise-in var(--d-4) var(--spring) both; }
  .k-work { margin: -8px 0; }
  /* A row the summary of your time away took you to, for a moment. */
  .it.marked::after { content: ''; position: absolute; inset: -8px -12px; border-radius: var(--r-md); background: var(--surface-3); opacity: 0; pointer-events: none; z-index: -1; animation: marked 1.6s var(--ease-out); }
  @keyframes marked { 15%, 60% { opacity: 1; } }
  @media (prefers-reduced-motion: reduce) { .it.marked::after { animation: none; opacity: 1; } }
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
  /* Message actions sit in the gap under the message, so showing them moves nothing. */
  .acts { position: absolute; top: 100%; left: -7px; z-index: 1; display: flex; gap: 2px; opacity: 0; transition: opacity var(--d-2); }
  .acts.mine { left: auto; right: 0; }
  .it:hover > .acts, .acts:focus-within { opacity: 1; }
  /* A work row sits 8px closer (.k-work): give the actions their 20px back, so they never lie on it. */
  .it:has(> .acts):has(+ .k-work) { margin-bottom: 8px; }
  .act { display: inline-flex; align-items: center; gap: 4px; height: 20px; padding: 0 7px; border-radius: var(--r-full); font-size: var(--t-xs); font-weight: var(--w-med); color: var(--ink-3); }
  .act:hover { background: var(--surface-3); color: var(--ink-1); }
  .mention { display: inline; padding: 0 3px; margin: 0 -1px; border-radius: 4px; color: var(--ink-1); font-weight: var(--w-semi); background: var(--surface-3); font-size: inherit; line-height: inherit; }
  .sys { display: flex; justify-content: center; gap: 8px; font-size: var(--t-sm); color: var(--ink-3); }
  .sys time { color: var(--ink-4); }
  .sys code { font-family: var(--font-mono); font-size: 0.92em; padding: 0 4px; border-radius: 4px; background: var(--surface-2); color: var(--ink-2); }
  .sys b { font-weight: var(--w-semi); color: var(--ink-2); }
  /* Your message sent again (Retry): a small line, not a second bubble. */
  .again { display: flex; justify-content: flex-end; gap: 8px; margin-top: -12px; font-size: var(--t-sm); color: var(--ink-3); }
  .again time { color: var(--ink-4); }
  .who { display: flex; align-items: center; gap: 8px; margin-bottom: 6px; }
  .face { display: grid; border-radius: 50%; }
  .name { font-weight: var(--w-semi); font-size: var(--t-md); }
  .reply { display: inline-flex; align-items: center; gap: 4px; font-size: var(--t-sm); color: var(--ink-3); }
  .who time { font-size: var(--t-xs); color: var(--ink-3); }
  .say { max-width: 66ch; }
  .say.commentary :global(.md) { font-size: var(--t-md); line-height: var(--lh-md); color: var(--ink-2); }
  .card-gap { margin-top: 14px; }
  .newer { position: absolute; left: 0; right: 0; bottom: calc(100% + 8px + var(--lift, 0px)); width: fit-content; margin: 0 auto; display: inline-flex; align-items: center; gap: 6px; height: 30px; padding: 0 14px; border-radius: var(--r-full); background: var(--pop); box-shadow: var(--pop-shadow); font-size: var(--t-sm); color: var(--ink-1); animation: rise-in var(--d-3) var(--spring); z-index: 2; }
  /* Off a waiting card's buttons (see `pillSpot`): at the thread's edge, 12 px in. */
  .newer.edge { left: auto; right: 12px; margin: 0; }
  .bottom { position: relative; padding: 0 20px 18px; }
</style>
