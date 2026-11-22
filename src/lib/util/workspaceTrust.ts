// Workspace trust: what the open-repo question and Settings → Safety say and offer about the
// repo's own CLI configuration (the core's orchestrator/trust.rs decides what loads).

import type { Provider } from '../gen/Provider';
import type { WorkspaceTrust } from '../gen/WorkspaceTrust';
import { PROVIDER_LABEL } from './labels';

/** Is the question due on screen? Only while you have not decided about the files as they are
 *  now, and not again for files you closed the question on (`later` is their fingerprint). */
export function shouldAsk(t: WorkspaceTrust, later: string | null): boolean {
  return t.state === 'ask' && t.fingerprint !== later;
}

export interface TrustChoices {
  /** The fingerprint to send to trust the files shown; null = they are trusted already, there
   *  are none, or Codly cannot read them whole. */
  trust: string | null;
  /** "Open without" would change something: there are files, and that is not your choice yet. */
  without: boolean;
}

export function trustChoices(t: WorkspaceTrust): TrustChoices {
  const files = t.state !== 'nothing';
  return { trust: files && t.state !== 'trusted' && !t.problem ? t.fingerprint : null, without: files && t.state !== 'without' };
}

/** Where your choice stands, in one sentence. */
export function trustStatus(t: WorkspaceTrust): string {
  switch (t.state) {
    case 'nothing':
      return 'This repo has no configuration that an agent CLI loads by itself.';
    case 'trusted':
      return 'You trusted these files. Agents load them.';
    case 'without':
      return 'You opened this repo without these files. Agents start without them.';
    case 'ask':
      return t.changed
        ? 'These files changed since your choice. Agents start without them until you choose again.'
        : 'You have not chosen yet. Agents start without these files until you do.';
  }
}

function withoutTrustOf(cli: Provider): string {
  const label = PROVIDER_LABEL[cli];
  switch (cli) {
    case 'claude':
      return 'Claude Code starts with your own settings only: no settings, hooks or MCP servers of this repo. Your own MCP servers stay out too, because Claude Code has one switch for both.';
    case 'codex':
      return 'Codex decides by itself: it loads a repo’s .codex folder only in a folder you trusted in Codex. Codly does not change that.';
    default:
      return `${label} agents do not start in this repo: Codly cannot keep ${label} from loading these files.`;
  }
}

/** What each CLI that has files here does while you have not trusted them: one line per CLI,
 *  in the order of the files. */
export function withoutTrust(t: WorkspaceTrust): string[] {
  return [...new Set(t.files.map((f) => f.cli))].map(withoutTrustOf);
}
