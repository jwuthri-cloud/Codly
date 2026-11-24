// What helps after something went wrong in a turn, beside Retry (outcomes.ts `canRetry`): sign
// the CLI in when the core says it is not signed in, and go on with a turn you stopped. Pure.
import type { Provider } from '../lib/gen/Provider';
import type { TroubleKind } from './outcomes';
import { PROVIDER_LABEL } from '../lib/util/labels';

const NAMES = Object.entries(PROVIDER_LABEL) as [Provider, string][];
/** The core's sentence (adapters/startup.rs `not_signed_in`, orchestrator/runs.rs `run_account`):
 *  "Codex is not signed in: run `codex login` in a terminal", and for one account "the Codex
 *  account 'Work' is not signed in: run `CODEX_HOME=… codex login` in a terminal". */
const NOT_SIGNED_IN = new RegExp(`(?:^|[\\s:(])(?:the )?(${NAMES.map(([, l]) => l).join('|')}) (?:account '[^']*' )?is not signed in: run \`([^\`]+)\` in a terminal`);

/** The CLI that is not signed in and the command the reason names, or null for any other reason. */
export function signInOf(reason: string | null): { provider: Provider; command: string } | null {
  const m = reason ? NOT_SIGNED_IN.exec(reason) : null;
  const provider = m ? NAMES.find(([, l]) => l === m[1])?.[0] : undefined;
  return m && provider ? { provider, command: m[2] } : null;
}

/**
 * The command "Sign in…" types into a terminal: the one the reason names, only when Codly knows
 * it itself for that CLI (`known`: its own sign-in command and those of its accounts). Words
 * that only look like the core's sentence never reach a shell.
 */
export function signInCommand(reason: string | null, known: (provider: Provider) => readonly (string | null)[]): { provider: Provider; command: string } | null {
  const asked = signInOf(reason);
  return asked && known(asked.provider).includes(asked.command) ? asked : null;
}

/**
 * Whether "Continue" helps: the row is the end of a turn you stopped yourself (`what`
 * "interrupted", `rowTs` when it ended; `stops`: the starts of the turns you stopped), and that
 * turn is still the agent's last one: nothing ran since, as for Retry. `turns`: the agent's, oldest first.
 */
export function canContinue(what: TroubleKind, rowTs: number, turns: readonly { start: number; end: number | null }[], stops: readonly number[]): boolean {
  const last = turns[turns.length - 1];
  return what === 'interrupted' && !!last && last.end === rowTs && stops.includes(last.start);
}
