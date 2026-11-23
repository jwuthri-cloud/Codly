// Workspace trust: what the open-repo question and Settings → Safety say and offer about the
// repo's own CLI configuration (the core's orchestrator/trust.rs decides what loads).

import type { CliInfo } from '../gen/CliInfo';
import type { Provider } from '../gen/Provider';
import type { Settings } from '../gen/Settings';
import type { TrustFile } from '../gen/TrustFile';
import type { WorkspaceTrust } from '../gen/WorkspaceTrust';
import { PROVIDER_LABEL } from './labels';

/** The sets of files the question is about, by their fingerprints: your checkout's while you
 *  have not decided about it, and each worktree's that differs from it. After "open without"
 *  the question asks nothing more: Settings → Safety still shows the worktrees. */
export function asked(t: WorkspaceTrust): string[] {
  if (t.state === 'without') return [];
  const checkout = t.state === 'ask' && t.fingerprint ? [t.fingerprint] : [];
  return [...checkout, ...t.worktrees.map((w) => w.fingerprint)];
}

/** Is the question due on screen? Only for files you have not decided about as they are now,
 *  and not again for files you closed the question on (`later`: their fingerprints). */
export function shouldAsk(t: WorkspaceTrust, later: readonly string[]): boolean {
  return asked(t).some((fingerprint) => !later.includes(fingerprint));
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

/** What does not wait for the choice, because it is yours and not the repo's (the core's
 *  runs.rs: a Codex agent's own servers follow "Use my own CLI setup" alone). */
export const CODEX_OWN_SERVERS =
  'A Codex agent starts the MCP servers of your own Codex config in every folder, trusted or not: they are yours, not this repo’s. To keep them out, turn off “Use my own CLI setup” in Settings.';

/** Do Codex agents start the MCP servers of your own Codex config? Only then is there
 *  something to say about them: "Use my own CLI setup" is on and Codex is installed. */
export function startsOwnCodexServers(settings: Pick<Settings, 'use_my_claude_setup'>, clis: readonly Pick<CliInfo, 'provider' | 'path'>[]): boolean {
  return settings.use_my_claude_setup && clis.some((c) => c.provider === 'codex' && c.path !== null);
}

/** What each CLI that has files here does while you have not trusted them: one line per CLI,
 *  in the order of the files. Then, when Codex agents start your own servers (`codexOwn`),
 *  the line that says they do so in every folder: the one thing a Claude agent loses without
 *  trust and a Codex agent keeps. */
export function withoutTrust(t: WorkspaceTrust, codexOwn: boolean): string[] {
  const lines = [...new Set(t.files.map((f) => f.cli))].map(withoutTrustOf);
  return codexOwn && lines.length > 0 ? [...lines, CODEX_OWN_SERVERS] : lines;
}

/** How many files a list shows. A repo can carry thousands (skills with their assets). */
export const MAX_SHOWN_FILES = 200;

/** The files a list shows, and how many more there are: the ones Codly read something in
 *  first (what they run, allow or start), then the rest, each in the order given. */
export function shownFiles(files: TrustFile[]): { shown: TrustFile[]; more: number } {
  const ordered = [...files.filter((f) => f.what.length > 0), ...files.filter((f) => f.what.length === 0)];
  return { shown: ordered.slice(0, MAX_SHOWN_FILES), more: Math.max(0, files.length - MAX_SHOWN_FILES) };
}
