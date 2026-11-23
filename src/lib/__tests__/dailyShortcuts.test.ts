// The rules behind the daily shortcuts: ⇧⇥ in a 1:1, Retry where "Retry to resume" shows, the
// model list, the palette's ranking, the keys of an inline question, a Kill's line, a close
// while the Lead works, and the one-step start of an agent.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Agent } from '../gen/Agent';
import type { HireManifest } from '../gen/HireManifest';
import type { ModelOption } from '../gen/ModelOption';
import type { Task } from '../gen/Task';
import { MockTransport } from '../mock';
import { api } from '../api';
import { switchAgentModel } from '../state/actions';
import { offersAsk, rankRows } from '../../overlays/palette';
import { closeQuestion } from '../state/closing';
import { PERMISSION_MODES } from '../state/control.svelte';
import { nextWay, wayOf } from '../../home/agentPicker';
import { quickManifest } from '../state/hireWizard';
import { SHORTCUTS } from '../state/keys';
import type { Request } from '../state/requests.svelte';
import { killedAt, killLine, noteKill } from '../state/stops';
import { store, type AgentLive } from '../state/store.svelte';
import { confirmKey } from '../util/confirm';
import { firstRow, pickRows } from '../util/modelPick';
import { retryTarget } from '../util/retry';
import { endedByKill, KILL_GRACE } from '../../home/outcomes';

describe('⇧⇥ in a 1:1', () => {
  it('cycles the modes of the sheet’s select, in its order, round', () => {
    const order = PERMISSION_MODES.map((m) => m.id);
    expect(order).toEqual(['ask', 'accept_edits', 'plan']);
    expect(nextWay('ask', 'balanced')).toBe('accept_edits');
    expect(nextWay('accept_edits', 'balanced')).toBe('plan');
    expect(nextWay('plan', 'autonomous')).toBe('ask');
    // An agent with no mode set is in "ask for edits".
    expect(nextWay(wayOf({ role: 'solo', provider: 'claude', permission_mode: undefined }, false)!, 'balanced')).toBe('accept_edits');
  });

  it('skips "accept edits" under "Ask first", where the core refuses it, so "plan first" stays in reach', () => {
    expect(nextWay('ask', 'strict')).toBe('plan');
    expect(nextWay('plan', 'strict')).toBe('ask');
    // An agent left in "accept edits" before the permissions changed moves on.
    expect(nextWay('accept_edits', 'strict')).toBe('ask');
  });

  it('only where the picker has a way to switch: a 1:1 Claude agent in its own chat (wayOf)', () => {
    expect(wayOf({ provider: 'claude', role: 'solo', permission_mode: 'plan' }, false)).toBe('plan');
    // On a task its chat shows the task's request's picker: no way to switch.
    expect(wayOf({ provider: 'claude', role: 'solo', permission_mode: 'ask' }, true)).toBeNull();
    for (const provider of ['codex', 'gemini', 'opencode'] as const) expect(wayOf({ provider, role: 'solo', permission_mode: undefined }, false)).toBeNull();
    expect(wayOf({ provider: 'claude', role: 'worker', permission_mode: undefined }, false)).toBeNull();
    expect(wayOf(null, false)).toBeNull();
  });

  it('is in the shortcuts sheet', () => {
    const rows = SHORTCUTS.flatMap((g) => g.items);
    expect(rows.some(([k, label]) => k === '⇧⇥' && /1:1 with Claude/.test(label) && /accept edits/.test(label))).toBe(true);
  });
});

describe('Retry next to “Retry to resume”', () => {
  const task = (o: Partial<Task>): Pick<Task, 'id' | 'status' | 'status_reason'> => ({ id: 't1', status: 'blocked', status_reason: null, ...o });

  it('targets a stopped or failed task whose reason says the words (core and demo wording)', () => {
    expect(retryTarget(task({ status_reason: 'stopped by you; Retry to resume with Ada' }))).toBe('t1');
    expect(retryTarget(task({ status_reason: 'stopped by you; Retry to resume' }))).toBe('t1');
    expect(retryTarget(task({ status_reason: 'Interrupted by you. Retry to resume.' }))).toBe('t1');
    expect(retryTarget(task({ status: 'failed', status_reason: 'interrupted by you; the CLI did not stop, so Codly ended its process; Retry to resume with Bo' }))).toBe('t1');
  });

  it('offers nothing where the words are not shown', () => {
    expect(retryTarget(null)).toBeNull();
    expect(retryTarget(undefined)).toBeNull();
    expect(retryTarget(task({ status_reason: 'waiting for you: which database?' }))).toBeNull();
    expect(retryTarget(task({ status_reason: 'review stopped by you; Retry to re-check' }))).toBeNull();
    // Retried already: the words may linger on a task that is on its way again.
    expect(retryTarget(task({ status: 'ready', status_reason: 'stopped by you; Retry to resume' }))).toBeNull();
    expect(retryTarget(task({ status: 'running', status_reason: 'Retry to resume' }))).toBeNull();
  });
});

describe('the model list', () => {
  const opt = (id: string, name = id, is_default = false): ModelOption => ({ id, name, is_default, efforts: [], default_effort: null }) as unknown as ModelOption;
  const models = [opt('default', 'Default', true), opt('opus', 'Opus'), opt('sonnet', 'Sonnet'), opt('haiku', 'Haiku')];

  it('opens with every model in view and the current one highlighted', () => {
    const rows = pickRows(models, '', 'sonnet');
    expect(rows.map((r) => r.id)).toEqual(['', 'default', 'opus', 'sonnet', 'haiku']);
    expect(rows.filter((r) => r.current).map((r) => r.id)).toEqual(['sonnet']);
    expect(firstRow(rows, false)).toBe(3);
    // A current model the catalog does not list is a row of its own, highlighted.
    const own = pickRows(models, '', 'claude-sonnet-5-5');
    expect(own.slice(0, 2).map((r) => [r.id, r.current])).toEqual([['', false], ['claude-sonnet-5-5', true]]);
    expect(firstRow(own, false)).toBe(1);
    // No model set: the CLI's default row is the current one.
    expect(firstRow(pickRows(models, '', ''), false)).toBe(0);
  });

  it('narrows on what you type; a name it does not list is the last row, and ↵ starts on the best match', () => {
    const rows = pickRows(models, 'op', 'sonnet');
    expect(rows.map((r) => [r.id, r.typed])).toEqual([['opus', false], ['op', true]]);
    expect(firstRow(rows, true)).toBe(0);
    expect(pickRows(models, 'opus', 'sonnet').map((r) => r.id)).toEqual(['opus']); // listed: no "Use …" row
    expect(pickRows(models, 'claude-opus-4-1', '').map((r) => r.label)).toEqual(['Use “claude-opus-4-1”']);
  });
});

describe('the palette', () => {
  const row = (id: string, title: string, sub?: string) => ({ id, title, sub });

  it('a title that matches comes before every row that matches only by its description', () => {
    const list = [
      row('go:knowledge', 'Go to Knowledge', 'Memory, merged decisions and the code map'),
      row('request:pr', 'From pull request…', 'Its reviews, comments on the code and failing checks go to the Lead'),
      row('plan:merge', 'Merge into main: dark mode'),
    ];
    expect(rankRows(list, 'merge', [], 12).map((r) => r.id)).toEqual(['plan:merge', 'go:knowledge']);
    // A weak title match still beats a strong description match.
    const fuzzyTitle = [row('a', 'Read the docs', 'merge them'), row('b', 'Make every row great')];
    expect(rankRows(fuzzyTitle, 'merge', [], 12).map((r) => r.id)).toEqual(['b', 'a']);
  });

  it('within titles, by score, a recent row lifted a little; the hits are the title’s letters', () => {
    const list = [row('x', 'Theme: Night'), row('y', 'Theme: Day')];
    expect(rankRows(list, 'theme', ['y'], 12).map((r) => r.id)).toEqual(['y', 'x']);
    expect(rankRows(list, 'night', [], 12)[0].hits).toEqual([7, 8, 9, 10, 11]);
    expect(rankRows([row('k', 'Go to Knowledge', 'merged')], 'merge', [], 12)[0].hits).toEqual([]);
  });

  it('“Ask the Lead…” is offered for a query on a request, never in a 1:1 or another scope', () => {
    expect(offersAsk('fix the bug', null, true)).toBe(true);
    expect(offersAsk('fix the bug', '>', true)).toBe(true);
    expect(offersAsk('fix the bug', null, false)).toBe(false);
    expect(offersAsk('fix the bug', '@', true)).toBe(false);
    expect(offersAsk('  ', null, true)).toBe(false);
  });
});

describe('an inline question’s keys', () => {
  const k = (key: string, o: { isComposing?: boolean; repeat?: boolean } = {}) => ({ key, isComposing: false, repeat: false, ...o });
  it('Esc is the safe choice; Enter and the other keys do what the focused button does', () => {
    expect(confirmKey(k('Escape'))).toBe('keep');
    expect(confirmKey(k('Enter'))).toBeNull();
    expect(confirmKey(k(' '))).toBeNull();
    expect(confirmKey(k('Tab'))).toBeNull();
    // An Esc that ends a composition, or a held one, answers nothing.
    expect(confirmKey(k('Escape', { isComposing: true }))).toBeNull();
    expect(confirmKey(k('Escape', { repeat: true }))).toBeNull();
  });
});

describe('a Kill of yours', () => {
  it('is remembered with its time for the conversation, and forgotten when the core refused it', () => {
    const undo = noteKill('k1', 1000);
    noteKill('k1', 2000);
    expect(killedAt('k1')).toEqual([1000, 2000]);
    undo();
    expect(killedAt('k1')).toEqual([2000]);
    expect(killedAt('someone-else')).toEqual([]);
    expect(killLine('Eli')).toBe('You killed Eli');
  });

  it('takes the place of the “turn was interrupted” line of the turn it cut', () => {
    const kills = [10_000];
    // The turn started before the kill and ended after it.
    expect(endedByKill({ what: 'interrupted', ts: 11_000, turn: { start: 5_000, input: null } }, kills)).toBe(true);
    // No turn on record (the run ended first): an interruption soon after the kill.
    expect(endedByKill({ what: 'interrupted', ts: 12_000, turn: null }, kills)).toBe(true);
    expect(endedByKill({ what: 'interrupted', ts: 10_000 + KILL_GRACE + 1, turn: null }, kills)).toBe(false);
    // A later turn, an earlier interruption, or another trouble is reported as before.
    expect(endedByKill({ what: 'interrupted', ts: 40_000, turn: { start: 30_000, input: null } }, kills)).toBe(false);
    expect(endedByKill({ what: 'interrupted', ts: 8_000, turn: { start: 5_000, input: null } }, kills)).toBe(false);
    expect(endedByKill({ what: 'failed', ts: 11_000, turn: { start: 5_000, input: null } }, kills)).toBe(false);
    expect(endedByKill({ what: 'interrupted', ts: 11_000, turn: { start: 5_000, input: null } }, [])).toBe(false);
  });
});

describe('closing a request whose Lead is mid-turn', () => {
  const lead = { id: 'L1', name: 'Lead', role: 'lead', thread: 'lead:a', status: 'thinking', provider: 'claude', archived: false } as unknown as Agent;
  const request = (o: Partial<Request> = {}): Request => ({ thread: 'lead:a', title: 'Fix A', phase: 'planning', ...o }) as Request;
  afterEach(() => {
    store.agents = [];
    store.live = {};
  });

  it('asks first, with the inline question’s words', async () => {
    store.agents = [lead];
    store.live = { L1: { turns: [{ start: 1, end: null, input: 'plan it' }] } as unknown as AgentLive };
    expect(await closeQuestion(request())).toEqual({ ask: 'The Lead is working: close and stop it?', detail: 'Its turn stops where it is. Nothing is deleted.', yes: 'Close' });
  });

  it('asks nothing new once its turn is over', async () => {
    store.agents = [{ ...lead, status: 'idle' } as Agent];
    store.live = { L1: { turns: [{ start: 1, end: 5, input: 'plan it' }] } as unknown as AgentLive };
    expect(await closeQuestion(request({ phase: 'talking' }))).toBeNull();
  });
});

describe('starting an agent in one step', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date', 'performance'] });
    vi.spyOn(Math, 'random').mockReturnValue(0.5);
    vi.stubGlobal('OffscreenCanvas', class { getContext() { return null; } });
    vi.stubGlobal('location', { search: '?ws=1' });
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });
  async function call<T>(t: MockTransport, cmd: string, args: Record<string, unknown> = {}): Promise<T> {
    const answer = t.call<T>(cmd, args);
    answer.catch(() => {});
    await vi.advanceTimersByTimeAsync(200);
    return answer;
  }

  it('takes the applicant’s name and look, the CLI’s default model, no traits and no brief', () => {
    const m = quickManifest('codex', { name: 'Iris', color: 5, avatar: 'fox' });
    expect(m).toMatchObject({ provider: 'codex', name: 'Iris', color: 5, avatar: 'fox', model: null, command: null, traits: [], goal: '', description: '' });
  });

  it('the demo hires it, and its model switch answers as the core does', async () => {
    const t = new MockTransport();
    const snap = t.subscribe(() => {});
    await vi.advanceTimersByTimeAsync(50);
    const lead = (await snap).agents.find((x) => x.role === 'lead');
    const [a] = await call<{ name: string; color: number; avatar: string }[]>(t, 'hire_applicants', { n: 1 });
    const agent = await call<Agent>(t, 'hire_from_manifest', { manifest: quickManifest('claude', a) as HireManifest, confirmShell: false });
    expect(agent).toMatchObject({ name: a.name, role: 'solo', provider: 'claude' }); // the demo gives it the CLI's default model by name
    const switched = await call<Agent>(t, 'set_agent_model', { agentId: agent.id, model: 'opus' });
    expect(switched).toMatchObject({ id: agent.id, model: 'opus' });
    // The same model again is no change, and still answers with the agent: never an empty answer.
    expect(await call<Agent>(t, 'set_agent_model', { agentId: agent.id, model: 'opus' })).toMatchObject({ model: 'opus' });
    if (lead) await expect(call(t, 'set_agent_model', { agentId: lead.id, model: 'opus' })).rejects.toThrow("only a 1:1 agent's model can be changed");
    await expect(call(t, 'set_agent_model', { agentId: 'nobody', model: 'opus' })).rejects.toThrow('Unknown agent');
  });
});

describe('a model switch from the dialog', () => {
  const ada = { id: 'a1', name: 'Ada', role: 'solo', provider: 'claude', model: null } as unknown as Agent;
  afterEach(() => vi.restoreAllMocks());

  it('says why it did not happen: the core’s refusal, or an answer with no agent in it', async () => {
    vi.spyOn(api, 'setAgentModel').mockRejectedValue(new Error('Ada is still working: stop its turn first'));
    expect(await switchAgentModel(ada, 'opus')).toBe('Ada is still working: stop its turn first');
    vi.spyOn(api, 'setAgentModel').mockResolvedValue(null as unknown as Agent);
    expect(await switchAgentModel(ada, 'opus')).toBe('Codly got no answer for the switch. Try again.');
  });

  it('answers nothing when it switched', async () => {
    vi.spyOn(api, 'setAgentModel').mockResolvedValue({ ...ada, model: 'opus' } as Agent);
    expect(await switchAgentModel(ada, 'opus')).toBeNull();
  });
});
