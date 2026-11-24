// The command palette's guard for rows that cannot be taken back (merge, cancel a running plan,
// answer an approval): the first Enter or click only arms the row and shows what it will do, the
// second one runs it. Any other key, or a different row, disarms. A held ↵ and a double-click
// are one decision, not two: their repeats do nothing on such a row. Also the rows built from
// the store's requests, the group "This agent" and the group "Said in conversations", away from
// the component so they can be tested.
import { agentCommands, type AgentDo, type SlashContext } from '../home/composerActions';
import { filterSlash } from '../home/composerMenu';
import type { ApprovalSubject } from '../lib/gen/ApprovalSubject';
import type { ChatMessage } from '../lib/gen/ChatMessage';
import type { Provider } from '../lib/gen/Provider';
import { permissionsAsked } from '../lib/util/permissionsAsked';
import { showsPlan } from '../lib/util/planRequest';
import { sandboxWants } from '../lib/util/sandbox';
import { fuzzy } from '../lib/util/fuzzy';
import { clean, oneLine } from '../lib/util/sanitize';
import { isClosedThread, isRequestThread, threadAgentId } from '../lib/state/ui.svelte';

/**
 * `armed`: the id of the row waiting for its second Enter (null: none). `again`: this press is
 * the auto-repeat of a held key, or a later click of a double-click.
 */
export function press(row: { id: string; confirm?: string }, armed: string | null, again: boolean): { run: boolean; armed: string | null } {
  if (!row.confirm) return { run: true, armed: null };
  if (again) return { run: false, armed };
  if (armed === row.id) return { run: true, armed: null };
  return { run: false, armed: row.id };
}

/**
 * The rows that match `q`, best first: a row whose title matches comes before every row that
 * matches only by its search words (`words`: "merge" finds the Changes room), and those before
 * every row that matches only by its description ("merge" offers "Merge into main" before "Go
 * to Knowledge", whose description mentions merges). Within each group, by score; a recent row
 * gets a small lift. `hits`: the title's matched letters (none for another match). `titleOnly`:
 * the title alone counts (an agent is found by its name, not by what it is doing).
 */
export function rankRows<T extends { id: string; title: string; sub?: string; words?: string }>(list: T[], q: string, recents: string[], limit: number, titleOnly = false): (T & { hits: number[] })[] {
  const lift = (r: T) => (recents.includes(r.id) ? 2 : 0);
  return list
    .map((r) => {
      const title = fuzzy(q, r.title);
      const words = !title && !titleOnly && r.words ? fuzzy(q, r.words) : null;
      const sub = !title && !words && !titleOnly && r.sub ? fuzzy(q, r.sub) : null;
      const match = title ?? words ?? sub;
      return { r, tier: title ? 2 : words ? 1 : 0, score: match?.score ?? 0, hits: title?.hits ?? [], hit: !!match };
    })
    .filter((x) => x.hit)
    .sort((a, b) => b.tier - a.tier || b.score + lift(b.r) - (a.score + lift(a.r)))
    .slice(0, limit)
    .map((x) => ({ ...x.r, hits: x.hits }));
}

/** Search words of the rooms' "Go to" rows, beside their titles. */
export const ROOM_WORDS: Partial<Record<string, string>> = { changes: 'merge diff review' };

/** The CLIs in the order the Add agent wizard lists them (AddAgentWizard.svelte): Claude Code first. */
export const CLI_ORDER: readonly Provider[] = ['claude', 'codex', 'gemini', 'opencode', 'cursor', 'copilot'];

/** `clis` in `CLI_ORDER`; a CLI it does not name goes last, in the order it came. */
export function inCliOrder<T extends { provider: string }>(clis: T[]): T[] {
  const at = (c: T) => {
    const i = CLI_ORDER.indexOf(c.provider as Provider);
    return i < 0 ? CLI_ORDER.length : i;
  };
  return [...clis].sort((a, b) => at(a) - at(b));
}

/**
 * What Esc does in the palette once no row waits for its second ↵: in a scope ("@", ">"…) with
 * text, the text goes and the scope stays; otherwise the palette closes.
 */
export const escStep = (scope: string | null, query: string): 'clear' | 'close' => (scope && query ? 'clear' : 'close');

/**
 * The question a row asks before it runs, if any (see `press`). `askFirst`: asked only when ↵
 * took the row, never when it was clicked ("Ask the Lead…" when nothing else matched: free text
 * and ↵ must not start a paid turn by accident). ⌘↵ runs the row's other action, which never
 * needs that question.
 */
export function guard(row: { confirm?: string; askFirst?: string }, byKey: boolean, alt: boolean): string | undefined {
  return row.confirm ?? (byKey && !alt ? row.askFirst : undefined);
}

/**
 * The question of "Ask the Lead…" when it is the only row (`others`: the rows above it). Once it
 * is asked (`asked`: the row waits for its second ↵) it stays: rows found a moment later come in
 * above it and do not take the question away under you.
 */
export const askFirst = (others: number, asked = false): string | undefined => (others && !asked ? undefined : 'Send this to the Lead? It starts a turn of the Lead.');

/** The hint in the palette's empty field: it finds what was said too, and offers the Lead only where "Ask the Lead…" is a row (`offersAsk`). */
export const paletteHint = (inRequest: boolean): string =>
  `Search agents, tasks, conversations and commands${inRequest ? ', or ask the Lead…' : '…'}`;

/** Whether the last row "Ask the Lead: …" is offered: for a query, with no scope or in commands,
 *  and only on a request (in a 1:1 there is no Lead you are talking to). */
export const offersAsk = (q: string, scope: string | null, inRequest: boolean): boolean => !!q.trim() && (!scope || scope === '>') && inRequest;

/**
 * Rows grouped by section, so each section's header shows once. The sections keep the order
 * of their best row, and the rows of a section keep their order: the best match stays first.
 */
export function bySection<T extends { section: string }>(rows: T[]): T[] {
  const groups = new Map<string, T[]>();
  for (const r of rows) {
    const g = groups.get(r.section);
    if (g) g.push(r);
    else groups.set(r.section, [r]);
  }
  return [...groups.values()].flat();
}

/**
 * What an approval asks for, in full, after "<agent> wants to" (a row's title shows 50 characters
 * of it). `provider`: the CLI of the agent that asks (none when Codly does not know the agent).
 */
export function wants(s: ApprovalSubject, provider: Provider | null | undefined): string {
  switch (s.kind) {
    case 'command':
      return `run ${clean(s.command)}${s.cwd ? `\nin ${clean(s.cwd)}` : ''}`;
    case 'file_change':
      return `change ${s.paths.map(clean).join(', ')}`;
    case 'tool':
      // A Claude agent's plan, or a host that a command in Claude Code's sandbox wants (the whole
      // host): said as that, not by the request's tool name.
      return showsPlan(s, provider) ? 'show you its plan' : (sandboxWants(s, provider) ?? `use the tool ${clean(s.name)}`);
    case 'question':
      return 'ask you a question';
    case 'permissions':
      return permissionsAsked(s.detail).wants;
  }
}

/**
 * The palette's "no" row for a waiting request of the agent `name`; `what`: the request in a few
 * words. `note`: the row answers nothing, it opens the note field on the request's card. A Claude
 * agent's plan gets that row: it has no plain "no", since without a note Claude Code shows the
 * same plan again.
 */
export function noRow(s: ApprovalSubject, provider: Provider | null | undefined, name: string, what: string): { title: string; note: boolean } {
  return showsPlan(s, provider) ? { title: `Send ${name}'s plan back with a note…`, note: true } : { title: `Deny ${name}: ${what}`, note: false };
}

/**
 * The palette's "yes" row for a waiting request of the agent `name`: its title (`what`: the
 * request in a few words) and the question its second ↵ answers (`full`: all of what it asks).
 * A Claude agent's plan is accepted, as on its card.
 */
export function okRow(s: ApprovalSubject, provider: Provider | null | undefined, name: string, what: string, full: string): { title: string; confirm: string } {
  return showsPlan(s, provider) ? { title: `Accept ${name}'s plan`, confirm: `Accept the plan? ${full}` } : { title: `Approve ${name}: ${what}`, confirm: `Approve once? ${full}` };
}

/** A row of the group "This agent": a control of the agent you talk to. One that cannot run has
 *  no `run`, and says why in `sub`. */
export type AgentRow = { id: string; section: string; title: string; sub: string; icon: string; run?: () => void };

/**
 * The group "This agent": the controls of the agent of the conversation you are on, the rows
 * the composer's `/` menu offers for it, from the same list (`agentCommands`). A row that
 * cannot run comes last with its reason, as in the menu; one that waits for the core's answer
 * keeps its place. No agent: no group.
 */
export function agentGroup(c: SlashContext, on: AgentDo): AgentRow[] {
  if (!c.agent) return [];
  return filterSlash(agentCommands(c, on), '').map((r) => ({
    id: `ctl:${c.agent!.id}:${r.name}`,
    section: 'This agent',
    title: r.label,
    icon: r.icon,
    ...('off' in r ? { sub: r.off } : { sub: r.hint, run: r.run }),
  }));
}

/** The group of messages that hold the query, under the commands. */
export const SAID = 'Said in conversations';
/** How many messages the group shows. */
export const SAID_MOST = 6;
/** The shortest query the group searches for. */
export const SAID_FROM = 3;

/** What `saidWhere` and `saidBy` know of the office. Null: not known. */
export type SaidNames = {
  /** A request's first message from you. */
  request: (thread: string) => string | null;
  /** A closed request's title (`closed_requests`). */
  closed: (thread: string) => string | null;
  /** The name an agent had in a closed request (`ClosedTranscript.authors`): its Lead is archived. */
  closedAuthor: (thread: string, id: string) => string | null;
  agent: (id: string) => string | null;
};

/** What the group knows of the closed requests among the messages found: their titles (read
 *  once) and who wrote in each (read once per request). */
export type SaidKnown = { titles: Map<string, string> | null; authors: Map<string, Record<string, string>> };

/** The core's reads the group makes (`api`). */
export type SaidReads = {
  searchChats: (q: string) => Promise<ChatMessage[]>;
  closedRequests: () => Promise<{ thread: string; title: string }[]>;
  closedRequest: (thread: string) => Promise<{ authors: Record<string, string> }>;
};

/**
 * The messages that hold `q`, and what names the closed requests among the first `SAID_MOST` of
 * them: the titles, and who wrote in each. A failed read of those names keeps the messages
 * found (their rows say "Closed request" and "An agent" for now, and the next search asks
 * again); only a failed search throws.
 */
export async function searchSaid(q: string, read: SaidReads, known: SaidKnown): Promise<{ hits: ChatMessage[]; known: SaidKnown }> {
  const hits = await read.searchChats(q);
  const closed = [...new Set(hits.slice(0, SAID_MOST).map((m) => m.thread).filter(isClosedThread))];
  if (!closed.length) return { hits, known };
  const [titles, ...authors] = await Promise.allSettled([
    known.titles ? Promise.resolve(null) : read.closedRequests(),
    ...closed.map((t) => (known.authors.has(t) ? Promise.resolve(null) : read.closedRequest(t))),
  ]);
  const byThread = new Map(known.authors);
  authors.forEach((r, i) => {
    if (r.status === 'fulfilled' && r.value) byThread.set(closed[i], r.value.authors);
  });
  const list = titles.status === 'fulfilled' ? titles.value : null;
  return { hits, known: { titles: list ? new Map(list.map((c) => [c.thread, c.title])) : known.titles, authors: byThread } };
}

/** The start of `text` on one line, cut at a word border near `max` characters. */
export function firstWords(text: string, max = 40): string {
  const line = clean(text).replace(/\s+/g, ' ').trim();
  if (line.length <= max) return line;
  const cut = line.lastIndexOf(' ', max);
  return `${line.slice(0, cut > max / 2 ? cut : max).trimEnd()}…`;
}

/** Where a message was said, from its thread. A request with no message from you is its Lead's. */
export function saidWhere(thread: string, names: SaidNames): string {
  if (isClosedThread(thread)) {
    const title = names.closed(thread);
    return title ? `Closed request: ${firstWords(title)}` : 'Closed request';
  }
  const agent = threadAgentId(thread);
  if (agent) return `1:1 with ${names.agent(agent) ?? 'an agent who left: not open to read'}`;
  const title = isRequestThread(thread) ? names.request(thread) : null;
  return title ? `Request: ${firstWords(title)}` : 'Lead';
}

/** Who said it (in `thread`): you, Codly, or an agent by its name, as it was in a closed request. */
export function saidBy(author: string, thread: string, names: SaidNames): string {
  if (author === 'user') return 'You';
  if (author === 'codly') return 'Codly';
  return (isClosedThread(thread) ? names.closedAuthor(thread, author) : null) ?? names.agent(author) ?? 'An agent';
}

/** A 1:1 chat opens only while its agent is in the office: one who left has no conversation to show. */
export function saidOpens(thread: string, names: SaidNames): boolean {
  const agent = threadAgentId(thread);
  return !agent || names.agent(agent) !== null;
}

/** How wide a found message's line is in its row: room stays for who said it and where. */
export const SAID_WIDTH = 44;

/**
 * The palette's scope and text from what it was opened with, or from what you typed: a first
 * `> @ # / +` picks a scope, unless the text is `plain` (searched as it is, like "/api/login"
 * sent by the find bar).
 */
export function seedOf(text: string, plain: boolean): { scope: string | null; query: string } {
  if (!plain && text && '>@#/+'.includes(text[0])) return { scope: text[0], query: text.slice(1) };
  return { scope: null, query: text };
}

/** Lower case for ASCII letters only, as the core's search compares: the length never changes. */
const fold = (s: string) => s.replace(/[A-Z]/g, (c) => c.toLowerCase());

/**
 * One line of `text` around the first place that holds `q` (ASCII case aside): only the line of
 * the message the hit is on (a hit in a code block keeps that one line), cut at word borders to
 * about `width` characters, with "…" where it was cut. `hits`: the letters of the hit in `line`;
 * none when the text does not hold `q`.
 */
export function snippet(text: string, q: string, width = 72): { line: string; hits: number[] } {
  const t = clean(text).replace(/[\t\r]/g, ' ');
  const at = q ? fold(t).indexOf(fold(q)) : -1;
  if (at < 0) return { line: oneLine(t, width), hits: [] };
  const lineStart = t.lastIndexOf('\n', at) + 1;
  const lineEnd = t.indexOf('\n', at + q.length);
  const raw = t.slice(lineStart, lineEnd < 0 ? t.length : lineEnd);
  const line = raw.trimStart();
  const pos = at - lineStart - (raw.length - line.length);
  const end = line.trimEnd().length;
  // `width` characters around the hit, a third of the room left before it.
  let from = Math.max(0, Math.min(pos - Math.floor(Math.max(0, width - q.length) / 3), end - width));
  let to = Math.min(end, Math.max(from + width, pos + q.length));
  if (from > 0 && line[from - 1] !== ' ') {
    const space = line.indexOf(' ', from);
    if (space >= 0 && space < pos) from = space + 1;
  }
  if (to < end && line[to] !== ' ') {
    const space = line.lastIndexOf(' ', to);
    if (space >= pos + q.length) to = space;
  }
  const head = from > 0 ? '…' : '';
  const start = head.length + pos - from;
  return { line: `${head}${line.slice(from, to).trimEnd()}${to < end ? '…' : ''}`, hits: Array.from({ length: q.length }, (_, i) => start + i) };
}
