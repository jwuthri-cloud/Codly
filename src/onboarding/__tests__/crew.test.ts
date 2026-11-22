import { describe, expect, it } from 'vitest';
import type { CliInfo } from '../../lib/gen/CliInfo';
import type { InstallHint } from '../../lib/gen/InstallHint';
import { cliFix, cliOk, cliState, installPlan, updateNote } from '../../lib/util/cli';
import { crewReady } from '../quest';

const hint = (over: Partial<InstallHint> = {}): InstallHint => ({ npm: null, native: null, login: null, docs: 'https://example.com', update: null, npm_available: true, update_to: null, ...over });
const cli = (over: Partial<CliInfo> = {}, install: Partial<InstallHint> = {}): CliInfo => ({
  provider: 'claude',
  path: '/bin/cli',
  version: '1.0.0',
  logged_in: true,
  billing: 'subscription',
  plan: null,
  problem: null,
  transport: 'stream-json',
  install: hint(install),
  default_model: null,
  ...over,
});

describe('an installed CLI with a problem', () => {
  it('a fresh Gemini CLI gets its reason and a sign-in, not a bare "Needs attention"', () => {
    const gemini = cli({ provider: 'gemini', logged_in: null, problem: 'sign-in not configured: run `gemini` once in a terminal to choose how to sign in' }, { login: 'gemini' });
    expect(cliState(gemini).text).toBe('Needs attention');
    expect(cliFix(gemini)).toEqual({ text: gemini.problem, login: 'gemini', update: null });
  });
  it('a signed-out CLI is offered its sign-in command', () => {
    const out = cli({ logged_in: false, problem: 'not signed in: run `claude auth login` in a terminal' }, { login: 'claude auth login' });
    expect(cliFix(out).login).toBe('claude auth login');
  });
  it('a CLI that says it is signed in is never told to sign in', () => {
    const slow = cli({ logged_in: true, problem: '`claude --version` timed out after 5 s' }, { login: 'claude auth login' });
    expect(cliFix(slow)).toEqual({ text: '`claude --version` timed out after 5 s', login: null, update: null });
  });
  it('a ready CLI needs nothing, and a missing one is not asked to sign in', () => {
    expect(cliFix(cli({}, { login: 'claude auth login' }))).toEqual({ text: null, login: null, update: null });
    const missing = cli({ path: null, logged_in: null, problem: 'not installed' }, { login: 'claude auth login' });
    expect(cliFix(missing).login).toBeNull();
  });
});

describe('a CLI older than Codly supports', () => {
  const old = cli({ provider: 'codex', version: '0.150.2' }, { update: 'npm install -g @openai/codex@latest', update_to: '0.161.0' });
  it('is told the version to reach and the command that updates it', () => {
    expect(cliFix(old).update).toEqual({ to: '0.161.0', command: 'npm install -g @openai/codex@latest' });
    expect(updateNote(old)).toBe('Update Codex to 0.161.0 or later: an older one can fail in the middle of a task.');
    expect(updateNote(cli())).toBeNull();
  });
  it('still counts as ready: the minimum is the oldest version tested, not a proven limit', () => {
    expect(cliOk(old)).toBe(true);
    expect(crewReady([old])).toBe(true);
  });
});

describe('Install for me', () => {
  const npm = 'npm install -g x';
  const native = 'curl -fsSL https://example.com/install | bash';
  it('runs the vendor’s own installer when there is one, and offers npm beside it', () => {
    expect(installPlan(hint({ npm, native }))).toEqual({ primary: { rung: 'native', command: native }, other: { rung: 'npm', command: npm } });
    expect(installPlan(hint({ native }))).toEqual({ primary: { rung: 'native', command: native }, other: null });
  });
  it('never offers npm on a Mac without npm', () => {
    expect(installPlan(hint({ npm, native, npm_available: false }))).toEqual({ primary: { rung: 'native', command: native }, other: null });
    // npm is the only way in: Node.js comes first.
    expect(installPlan(hint({ npm, npm_available: false }))).toEqual({ primary: null, other: null });
  });
  it('runs npm for a CLI that ships only that way', () => {
    expect(installPlan(hint({ npm }))).toEqual({ primary: { rung: 'npm', command: npm }, other: null });
  });
});
