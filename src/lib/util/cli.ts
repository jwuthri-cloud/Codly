// What a detected CLI means to a person: ready, signed out, missing. Shared by Welcome and onboarding.
import type { CliInfo } from '../gen/CliInfo';
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

const HUE: Record<Provider, number> = { claude: 1, codex: 4, gemini: 5, opencode: 6, cursor: 7, copilot: 3 };
/** A stand-in crew member for a CLI, so Portrait can draw it before any agent exists. */
export const drone = (p: Provider) => ({ id: `cli-${p}`, name: PROVIDER_LABEL[p], color: HUE[p], look_seed: p, provider: p, role: 'worker' as const });
