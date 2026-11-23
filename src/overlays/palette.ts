// The command palette's guard for rows that cannot be taken back (merge, cancel a running plan,
// answer an approval): the first Enter or click only arms the row and shows what it will do, the
// second one runs it. Any other key, or a different row, disarms. A held ↵ and a double-click
// are one decision, not two: their repeats do nothing on such a row. Also the rows built from
// the store's requests and the group "This agent", away from the component so they can be tested.
import { agentCommands, type AgentDo, type SlashContext } from '../home/composerActions';
import { filterSlash } from '../home/composerMenu';
import type { ApprovalSubject } from '../lib/gen/ApprovalSubject';
import type { Provider } from '../lib/gen/Provider';
import { permissionsAsked } from '../lib/util/permissionsAsked';
import { showsPlan } from '../lib/util/planRequest';
import { sandboxWants } from '../lib/util/sandbox';
import { fuzzy } from '../lib/util/fuzzy';
import { clean } from '../lib/util/sanitize';

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
 * matches only by its description ("merge" offers "Merge into main" before "Go to Knowledge",
 * whose description mentions merges). Within each group, by score; a recent row gets a small
 * lift. `hits`: the title's matched letters (none for a description match).
 */
export function rankRows<T extends { id: string; title: string; sub?: string }>(list: T[], q: string, recents: string[], limit: number): (T & { hits: number[] })[] {
  const lift = (r: T) => (recents.includes(r.id) ? 2 : 0);
  return list
    .map((r) => {
      const title = fuzzy(q, r.title);
      const sub = !title && r.sub ? fuzzy(q, r.sub) : null;
      return { r, byTitle: !!title, score: (title ?? sub)?.score ?? 0, hits: title?.hits ?? [], hit: !!(title ?? sub) };
    })
    .filter((x) => x.hit)
    .sort((a, b) => Number(b.byTitle) - Number(a.byTitle) || b.score + lift(b.r) - (a.score + lift(a.r)))
    .slice(0, limit)
    .map((x) => ({ ...x.r, hits: x.hits }));
}

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
