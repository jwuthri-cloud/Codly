// What a detected CLI means to a person: ready, signed out, missing. Shared by Welcome and onboarding.
import type { CliInfo } from '../gen/CliInfo';
import type { InstallHint } from '../gen/InstallHint';
import type { InstallRung } from '../gen/InstallRung';
import type { Provider } from '../gen/Provider';
import { PROVIDER_LABEL } from './labels';

export const cliOk = (c: CliInfo) => !!c.path && c.logged_in !== false && !c.problem;
/** Ready first, then installed-but-unusable, then missing. */
export const cliRank = (c: CliInfo) => (cliOk(c) ? 0 : c.path ? 1 : 2);

export function cliState(c: CliInfo): { tone: 'done' | 'wait' | 'idle' | 'blocked'; text: string } {
  if (!c.path) return { tone: 'idle', text: 'Not installed' };
  if (c.logged_in === false) return { tone: 'wait', text: 'Not signed in' };
  if (c.problem) return { tone: 'blocked', text: 'Needs attention' };
  if (c.logged_in == null) return { tone: 'done', text: 'Installed · sign-in not checked' };
  return { tone: 'done', text: 'Signed in' };
}

/** `code` spans in a CLI's problem text. */
export const codeParts = (s: string) => s.split(/(`[^`]+`)/g).map((p) => ({ code: p.startsWith('`'), t: p.replace(/`/g, '') }));

/** One line `install_cli` can run: the rung it takes and the command the core types for it. */
export type Rung = { rung: InstallRung; command: string };

/**
 * How to install a missing CLI. `primary` is what "Install for me" runs: the vendor's own
 * installer when there is one (it needs no Node.js), else npm. `other` is the second way, when
 * both can run on this Mac. `primary: null` = it installs with npm only and npm is missing:
 * Node.js comes first.
 */
export function installPlan(h: InstallHint): { primary: Rung | null; other: Rung | null } {
  const native: Rung | null = h.native ? { rung: 'native', command: h.native } : null;
  const npm: Rung | null = h.npm && h.npm_available ? { rung: 'npm', command: h.npm } : null;
  return native ? { primary: native, other: npm } : { primary: npm, other: null };
}

/** How to update a CLI that is older than Codly supports. */
export type UpdateFix = {
  /** The version it must reach. */
  to: string;
  /** The line "Update" runs; null when Codly has none that can run on this Mac. */
  command: string | null;
  /** What else to know: the line needs npm and there is none, or it only updates an npm install. */
  how: string | null;
};

function updateFix(h: InstallHint): UpdateFix | null {
  if (!h.update_to) return null;
  const byNpm = !!h.update?.startsWith('npm ');
  if (byNpm && !h.npm_available) {
    return { to: h.update_to, command: null, how: 'Codly cannot update it for you: its update line needs npm, and this Mac has none. Update it the way you installed it (Homebrew, for example).' };
  }
  return {
    to: h.update_to,
    command: h.update,
    how: byNpm ? 'This line updates a copy installed with npm. If you installed it another way (Homebrew, for example), update it that way.' : null,
  };
}

/**
 * What an installed CLI still needs: `text` says why it is not ready (the core's words),
 * `login` is the sign-in command to offer, `update` how to bring an old one up to date. A CLI
 * that says it is signed in is never offered a sign-in.
 */
export function cliFix(c: CliInfo): { text: string | null; login: string | null; update: UpdateFix | null } {
  const unready = c.logged_in === false || !!c.problem;
  return {
    text: c.problem,
    login: c.path && unready && c.logged_in !== true ? c.install.login : null,
    update: updateFix(c.install),
  };
}

/** "Update Codex to 0.161.0 or later…" for a CLI older than Codly supports; null when it is new enough. */
export const updateNote = (c: CliInfo) =>
  c.install.update_to ? `Update ${PROVIDER_LABEL[c.provider]} to ${c.install.update_to} or later: an older one can fail in the middle of a task.` : null;

const HUE: Record<Provider, number> = { claude: 1, codex: 4, gemini: 5, opencode: 6, cursor: 7, copilot: 3 };
/** A stand-in crew member for a CLI, so Portrait can draw it before any agent exists. */
export const drone = (p: Provider) => ({ id: `cli-${p}`, name: PROVIDER_LABEL[p], color: HUE[p], look_seed: p, provider: p, role: 'worker' as const });
