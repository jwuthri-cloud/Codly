// The browser demo's answer to "From issue…" and "From pull request…". There is no gh behind
// it: any number, or any link to an issue or a pull request on github.com, "reads" a made-up one
// about dark mode. Your message lands in the request and the demo's Lead answers it as it
// answers any message.
import type { Intake } from '../gen/Intake';
import type { IntakeKind } from '../gen/IntakeKind';

const LINK = /^(?:https?:\/\/)?github\.com\/([\w.-]+)\/([\w.-]+)\/(issues|pull)\/(\d+)(?:[/?#].*)?$/;

/** `say`: put your message in the request and let its Lead answer. */
export function demoIntake(a: Record<string, unknown>, say: (thread: string, text: string) => void): Intake {
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
  say(thread, note ? `${ask}\n\n${note}` : ask);
  return {
    source: { kind, host: 'github.com', owner, repo, number, url, title: pull ? 'Keep dark mode across reloads' : 'Dark mode resets on reload' },
    comments: pull ? 1 : 2,
    review_comments: pull ? 2 : 0,
    failing_checks: pull ? 1 : 0,
    notes: [],
  };
}
