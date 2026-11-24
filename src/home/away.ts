// What happened in a thread while you were away, in one sentence, and whether a request you are
// not on has news. Built from the thread's rows only: no model writes it. Pure.
//
// You were away when the page was hidden or the window had no focus for 10 minutes or more, or
// when you open a thread that you last had on screen 10 minutes ago or more.
import type { Approval } from '../lib/gen/Approval';
import type { EventRow } from '../lib/gen/EventRow';
import { endedByKill, troubleTitle, troublesOf } from './outcomes';
import type { Item } from './spine';

/** How long you must be gone before the thread sums up what came, in ms. */
export const AWAY_MS = 10 * 60_000;
/** Fewer new rows than this need no summary: you read them at a glance. */
const MIN_ROWS = 3;
/** The facts one summary says at most (the last one may be "N more rows"). */
const MAX_FACTS = 5;
/** The words of a trouble's reason that the summary keeps. */
const REASON_WORDS = 8;

/** One fact of the summary and the row it is about (the oldest one, for a group). */
export type AwayFact = { key: string; text: string };
export type Away = { minutes: number; facts: AwayFact[] };

/** The time you left a thread and the time you came back to it. */
export type AwayWindow = { since: number; back: number };

/**
 * Where a plan stands, for the summary's words: it waits for your go, it was merged into your
 * branch, its branch was kept, it is finished with a conflict, or with a merge its checks
 * refused, it is ready to merge, or none of those (it runs, it was cancelled…).
 */
export type PlanEnd = 'waits' | 'merged' | 'kept' | 'conflict' | 'checks' | 'ready' | 'other';

/** What the summary needs to name things. */
export type AwayNames = {
  agent: (id: string) => string;
  task: (id: string) => { key: string; assignee: string | null } | null;
  plan: (planId: string) => PlanEnd;
};

/** A finished plan in the summary, in the words of its card's head. */
export function planDoneWords(end: PlanEnd): string {
  switch (end) {
    case 'merged': return 'the plan was merged';
    case 'ready': return 'the plan is ready to merge';
    case 'kept': return 'the plan finished, and its branch is kept';
    case 'conflict': return 'the plan finished, with a conflict to resolve';
    case 'checks': return 'the plan finished, and its checks stopped the merge';
    default: return 'the plan finished';
  }
}

/** How often the thread you look at is stamped: a quit or a crash with no leave event loses at most this much. */
export const STAMP_EVERY_MS = 60_000;

/** Whether being gone from `left` until `now` counts as away. */
export const wasAway = (left: number | null | undefined, now: number): boolean => left != null && now - left >= AWAY_MS;

/** "47 min", "1 h 12 min", "2 h". */
export function awayFor(minutes: number): string {
  if (minutes < 60) return `${minutes} min`;
  const h = Math.floor(minutes / 60);
  return minutes % 60 ? `${h} h ${minutes % 60} min` : `${h} h`;
}

/** "Ada", "Ada and Bo", "Ada, Bo and Cy". */
function joined(words: string[]): string {
  return words.length < 2 ? (words[0] ?? '') : `${words.slice(0, -1).join(', ')} and ${words[words.length - 1]}`;
}

function firstWords(s: string, n: number): string {
  const w = s.replace(/\s+/g, ' ').trim().split(' ');
  return w.length > n ? `${w.slice(0, n).join(' ')}…` : w.join(' ');
}

/** A fact as it is built: how much it needs you (lower first), when its row came, and the new rows it covers. */
type Draft = AwayFact & { rank: number; ts: number; covers: Item[] };

/** What the cards that wait for you ask: an answer (all are questions), an approval (none is), or both. */
function waitWords(cards: Approval[]): string {
  const questions = cards.filter((a) => a.subject.kind === 'question').length;
  return questions === cards.length ? 'your answer' : questions ? 'you' : 'your approval';
}

/**
 * The summary of what came in a thread between `w.since` and `w.back`, from its rows (`items`:
 * in their order on screen), or null when there is nothing worth one: fewer than 3 new rows, or
 * less than 10 minutes gone. Your own messages are not news. At most five facts, the ones that
 * need you first: a plan that waits for your go and the cards that still wait for you (every
 * one that came before your return and waits, also one older than your leave: it is what the
 * thread holds), trouble, requests answered without you (by a saved rule or by the agent's
 * mode), the tasks that finished, a plan that finished (in the words of where it stands), and
 * how many other rows came. A fact that does not fit is counted with the other rows.
 */
export function awaySummary(items: readonly Item[], w: AwayWindow, names: AwayNames): Away | null {
  if (!wasAway(w.since, w.back)) return null;
  const fresh = (it: Item) => it.ts > w.since && it.ts <= w.back;
  const news = items.filter((it) => fresh(it) && it.kind !== 'user' && it.kind !== 'away');
  if (news.length < MIN_ROWS) return null;

  const drafts: Draft[] = [];
  const add = (rank: number, rows: Item[], text: string) => drafts.push({ key: rows[0].key, text, rank, ts: rows[0].ts, covers: rows.filter(fresh) });

  // A plan that waits for your go, like a card that waits: also one that came before your leave.
  for (const it of items) {
    if ((it.kind !== 'system' && it.kind !== 'say') || it.card?.type !== 'plan' || it.ts > w.back) continue;
    if (names.plan(it.card.planId) === 'waits') add(0, [it], 'the plan waits for your go');
  }

  const waiting = items.filter((it): it is Item & { kind: 'approval' } => it.kind === 'approval' && it.approval.status === 'pending' && it.ts <= w.back);
  if (waiting.length) {
    const cards = waiting.flatMap((it) => [it.approval, ...it.twins]);
    const who = [...new Set(cards.map((a) => names.agent(a.agent_id)))];
    add(0, waiting, `${joined(who)} ${who.length > 1 ? 'wait' : 'waits'} for ${waitWords(cards)}`);
  }

  for (const it of news) {
    if (it.kind !== 'trouble') continue;
    const head = troubleTitle(it.what, names.agent(it.agentId));
    add(1, [it], it.reason ? `${head}: ${firstWords(it.reason, REASON_WORDS)}` : head);
  }

  const answered = news.filter(
    (it): it is Item & { kind: 'approval' } =>
      it.kind === 'approval' && (it.approval.status === 'approved' || it.approval.status === 'denied') && it.approval.decided_by != null && it.approval.decided_by !== 'user',
  );
  if (answered.length) add(2, answered, `${answered.length === 1 ? 'a request was' : `${answered.length} requests were`} answered without you`);

  const finishedBy = new Map<string | null, (Item & { kind: 'task-done' })[]>();
  for (const it of news) {
    if (it.kind !== 'task-done') continue;
    const assignee = names.task(it.taskId)?.assignee ?? null;
    finishedBy.set(assignee, [...(finishedBy.get(assignee) ?? []), it]);
  }
  for (const [assignee, rows] of finishedBy) {
    const keys = joined(rows.map((r) => names.task(r.taskId)?.key ?? 'a task'));
    add(3, rows, assignee ? `${names.agent(assignee)} finished ${keys}` : `${keys} ${rows.length > 1 ? 'are' : 'is'} done`);
  }

  for (const it of news) {
    if ((it.kind !== 'system' && it.kind !== 'say') || it.card?.type !== 'plan-done') continue;
    add(4, [it], planDoneWords(names.plan(it.card.planId)));
  }

  drafts.sort((a, b) => a.rank - b.rank || a.ts - b.ts);
  const rest = (shown: Draft[]) => {
    const covered = new Set(shown.flatMap((d) => d.covers));
    return news.filter((it) => !covered.has(it));
  };
  let shown = drafts;
  let others = rest(shown);
  if (shown.length + (others.length ? 1 : 0) > MAX_FACTS) {
    shown = drafts.slice(0, MAX_FACTS - 1);
    others = rest(shown);
  }
  const facts: AwayFact[] = shown.map(({ key, text }) => ({ key, text }));
  if (others.length) facts.push({ key: others[0].key, text: others.length === 1 ? '1 more row' : `${others.length} more rows` });
  return { minutes: Math.round((w.back - w.since) / 60_000), facts };
}

/** The keys of the rows a summary speaks of: the ones that came between your leave and your return. */
export function awayKeys(items: readonly Item[], w: AwayWindow): Set<string> {
  return new Set(items.filter((it) => it.ts > w.since && it.ts <= w.back).map((it) => it.key));
}

/**
 * The rows with the summary's row (`row`) in its place: before the first row newer than your
 * leave (`row.ts`; `items`: in their order on screen). With no newer row it has no place.
 */
export function withAway(items: Item[], row: Item & { kind: 'away' }): Item[] {
  const at = items.findIndex((it) => it.ts > row.ts);
  return at < 0 ? items : [...items.slice(0, at), row, ...items.slice(at)];
}

/** A row of a request that lights its tab's dot: when it came, in which request. */
export type News = { thread: string; ts: number };

/**
 * By request: when its newest row that is not yours came (`hasNews`): a message that is not yours
 * (`chats`; the Lead's, Codly's line that a plan is ready or finished), a card that asks you
 * (`asks`), a task that finished (`finished`), something that went wrong in its Lead's turn
 * (`troubles`). `isRequest`: a thread that is a request (a 1:1 chat has no tab).
 */
export function requestNews(
  s: { chats: readonly { thread: string; author: string; ts: number }[]; asks: readonly News[]; finished: readonly News[]; troubles: readonly News[] },
  isRequest: (thread: string) => boolean,
): Map<string, number> {
  const at = new Map<string, number>();
  const note = (n: News) => at.set(n.thread, Math.max(at.get(n.thread) ?? 0, n.ts));
  for (const c of s.chats) if (c.author !== 'user' && isRequest(c.thread)) note(c);
  for (const n of [...s.asks, ...s.finished, ...s.troubles]) note(n);
  return at;
}

/**
 * By request: when something went wrong in its Lead's turns (`requestNews`'s `troubles`), by the
 * rule of the rows the thread shows (outcomes.ts): an error, a turn that failed or hit its step
 * limit, a turn that something other than you cut. A stop of yours (`stops`: the starts of the
 * turns you stopped, by button or key) and a turn you killed (`kills`) are not news. One pass
 * over the log for all the Leads.
 */
export function leadTroubles(events: readonly EventRow[], leads: readonly { thread: string; agentId: string }[], stops: (agentId: string) => number[], kills: (agentId: string) => number[]): News[] {
  const byLead = new Map<string, EventRow[]>(leads.map((l) => [l.agentId, []]));
  for (const ev of events) if (ev.agent_id) byLead.get(ev.agent_id)?.push(ev);
  return leads.flatMap((l) => {
    const yours = stops(l.agentId);
    const killed = kills(l.agentId);
    return troublesOf(byLead.get(l.agentId) ?? [], l.agentId, (start) => yours.includes(start))
      .filter((t) => !endedByKill(t, killed))
      .map((t) => ({ thread: l.thread, ts: t.ts }));
  });
}

/**
 * A request's tab shows a dot: it is not on screen, and something came in it (`newest`: when its
 * newest row that is not yours came) since you last had it on screen (`seen`; unknown: no dot).
 * The amber of a request that needs you wins over the dot.
 */
export function hasNews(t: { onScreen: boolean; needsYou: boolean; seen: number | undefined; newest: number | null }): boolean {
  return !t.onScreen && !t.needsYou && t.seen != null && t.newest != null && t.newest > t.seen;
}

/** A thread of an office. */
export type Spot = { office: string; thread: string };

/**
 * When you have the thread on screen and when you came back to it. The thread is stamped when it
 * leaves the screen (another thread, no thread, the page hidden, the window without the focus),
 * when you look at it again, and at most once a minute while you look at it (`tick`), so the time
 * kept is never much older than your last look, also after a quit or a crash that gave no leave
 * event. Each call that brings you back returns the time to sum up, if you were away long enough.
 * Pure: the caller gives the clock, reads (`seen`) and keeps (`stamp`) the times.
 */
export class Presence {
  private spot: Spot | null = null;
  private present = false;
  private stamped = -Infinity;

  constructor(
    private readonly seen: (spot: Spot) => number | undefined,
    private readonly stamp: (spot: Spot, at: number) => void,
  ) {}

  /** `spot` is the thread on screen now (null: none); `present`: the page shows and the window has the focus. */
  show(spot: Spot | null, present: boolean, now: number): AwayWindow | null {
    this.leave(now);
    this.spot = spot;
    this.present = present;
    return this.arrive(now);
  }

  /** The page was shown or hidden, the window got or lost the focus. */
  setPresent(present: boolean, now: number): AwayWindow | null {
    if (present === this.present) return null;
    if (!present) {
      this.leave(now);
      this.present = false;
      return null;
    }
    this.present = true;
    return this.arrive(now);
  }

  /** Time passes: the thread you look at is stamped once a minute. */
  tick(now: number) {
    if (this.spot && this.present && now - this.stamped >= STAMP_EVERY_MS) this.put(this.spot, now);
  }

  private arrive(now: number): AwayWindow | null {
    if (!this.spot || !this.present) return null;
    const left = this.seen(this.spot);
    this.put(this.spot, now);
    return left != null && wasAway(left, now) ? { since: left, back: now } : null;
  }

  private leave(now: number) {
    if (this.spot && this.present) this.put(this.spot, now);
  }

  private put(spot: Spot, now: number) {
    this.stamp(spot, now);
    this.stamped = now;
  }
}
