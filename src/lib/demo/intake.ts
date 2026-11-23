// The browser demo's answer to "From issue…" and "From pull request…". There is no gh behind
// it: any number, or any link to an issue or a pull request on github.com, "reads" a made-up one
// about dark mode. Your message lands in the request, with the issue or pull request it started
// from (the pull-request step ends its description with "Closes #N" from it, as the core does),
// and the demo's Lead answers it as it answers any message.
import type { Intake } from '../gen/Intake';
import type { IntakeKind } from '../gen/IntakeKind';
import type { IntakeSource } from '../gen/IntakeSource';

const LINK = /^(?:https?:\/\/)?github\.com\/([\w.-]+)\/([\w.-]+)\/(issues|pull)\/(\d+)(?:[/?#].*)?$/;

/** `say`: put your message in the request, with what it started from, and let its Lead answer. */
export function demoIntake(a: Record<string, unknown>, say: (thread: string, text: string, source: IntakeSource) => void): Intake {
  const thread = String(a.thread);
  const reference = String(a.reference).trim();
  const note = String(a.note ?? '').trim();
  if (thread !== 'lead' && !thread.startsWith('lead:')) throw new Error(`${thread} is not a request; start from an issue or a pull request in a request to the Lead`);
  const link = LINK.exec(reference);
  // A link says by itself whether it is an issue or a pull request.
  const kind: IntakeKind = link ? (link[3] === 'pull' ? 'pull_request' : 'issue') : (a.kind as IntakeKind);
  const pull = kind === 'pull_request';
  const number = Number(link ? link[4] : /^#?(\d+)$/.exec(reference)?.[1]);
  if (!number) {
    throw new Error(`'${reference}' is not a GitHub ${pull ? 'pull request' : 'issue'}: paste its link (https://github.com/owner/repository/${pull ? 'pull' : 'issues'}/12) or type its number`);
  }
  const [owner, repo] = link ? [link[1], link[2]] : ['acme', 'web'];
  const url = `https://github.com/${owner}/${repo}/${pull ? 'pull' : 'issues'}/${number}`;
  const of = `#${number} of ${owner}/${repo} (${url})`;
  const ask = pull ? `Work on pull request ${of}: address its review comments and failing checks.` : `Work on issue ${of}.`;
  const source: IntakeSource = { kind, host: 'github.com', owner, repo, number, url, title: pull ? 'Keep dark mode across reloads' : 'Dark mode resets on reload' };
  say(thread, note ? `${ask}\n\n${note}` : ask, source);
  return {
    source,
    comments: pull ? 1 : 2,
    review_comments: pull ? 2 : 0,
    failing_checks: pull ? 1 : 0,
    notes: [],
    lead_problem: null,
  };
}
