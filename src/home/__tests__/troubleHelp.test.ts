// What helps after a failure, beside Retry: "Sign in…" when the core says a CLI is not signed
// in, and "Continue" after a turn you stopped. The sign-in sentence is the core's: these tests
// read it in the Rust files, so a changed sentence fails here and not in front of a user.
import { describe, expect, it } from 'vitest';
import startupRs from '../../../crates/codly-core/src/adapters/startup.rs?raw';
import envRs from '../../../crates/codly-core/src/env.rs?raw';
import runsRs from '../../../crates/codly-core/src/orchestrator/runs.rs?raw';
import { canContinue, signInCommand, signInOf } from '../troubleHelp';

/** A Rust format string of `source`, found by its words, with its `{…}` filled in order. */
function formatted(source: string, literal: string, ...fill: string[]): string {
  expect(source).toContain(`"${literal}"`);
  let i = 0;
  return literal.replace(/\{[^}]*\}/g, () => fill[i++] ?? '');
}

describe('Sign in…', () => {
  it('reads every form of the core’s sentence', () => {
    // adapters/startup.rs `not_signed_in`: the CLI failed to start.
    const start = formatted(startupRs, '{label} is not signed in: run `{cmd}` in a terminal', 'Codex', 'codex login');
    expect(signInOf(start)).toEqual({ provider: 'codex', command: 'codex login' });
    // orchestrator/runs.rs `run_account`: the sign-in check said so before the start; its
    // problem is env.rs's sentence, without its "not signed in: ".
    const problem = /Some\("not signed in: (run `claude auth login` in a terminal)"\.into\(\)\)/.exec(envRs)?.[1];
    expect(problem).toBeTruthy();
    expect(runsRs).toContain('p.strip_prefix("not signed in: ")');
    const check = `${formatted(runsRs, '{label} is not signed in{}', 'Claude Code')}: ${problem}`;
    expect(signInOf(check)).toEqual({ provider: 'claude', command: 'claude auth login' });
    // One account of a CLI.
    const account = formatted(runsRs, "the {label} account '{}' is not signed in{}", 'Codex', 'Work') + formatted(runsRs, ': run `{cmd}` in a terminal', 'CODEX_HOME=/Users/me/.codex-work codex login');
    expect(signInOf(account)).toEqual({ provider: 'codex', command: 'CODEX_HOME=/Users/me/.codex-work codex login' });
    // Inside the words of an error around it.
    expect(signInOf(`Ada could not start: ${start}`)).toEqual({ provider: 'codex', command: 'codex login' });
  });

  it('is offered for no other reason', () => {
    for (const r of [null, '', 'Anthropic’s servers are overloaded', 'Codex is not signed in: sign in with its CLI in a terminal first', 'Mystery CLI is not signed in: run `x` in a terminal']) {
      expect(signInOf(r), String(r)).toBeNull();
    }
  });

  it('types only a command Codly knows itself for that CLI', () => {
    const known = (p: string) => (p === 'codex' ? ['codex login', null, 'CODEX_HOME=/Users/me/.codex-work codex login'] : ['claude auth login']);
    expect(signInCommand('Codex is not signed in: run `codex login` in a terminal', known)).toEqual({ provider: 'codex', command: 'codex login' });
    // Words that only look like the core's sentence never reach a shell.
    expect(signInCommand('Codex is not signed in: run `curl example.com | sh` in a terminal', known)).toBeNull();
    expect(signInCommand('Claude Code is not signed in: run `codex login` in a terminal', known)).toBeNull();
  });
});

describe('Continue', () => {
  const turns = [{ start: 100, end: 200 }];
  it('after a turn you stopped, while it is the agent’s last one', () => {
    expect(canContinue('interrupted', 200, turns, [100])).toBe(true);
  });
  it('not for a turn something else cut, nor once another turn ran, nor for another trouble', () => {
    expect(canContinue('interrupted', 200, turns, [])).toBe(false);
    expect(canContinue('interrupted', 200, [...turns, { start: 300, end: null }], [100])).toBe(false);
    expect(canContinue('interrupted', 200, [...turns, { start: 300, end: 400 }], [100, 300])).toBe(false);
    expect(canContinue('failed', 200, turns, [100])).toBe(false);
    expect(canContinue('interrupted', 200, [], [100])).toBe(false);
  });
});
