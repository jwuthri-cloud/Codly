import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../../lib/api';
import type { ChatMessage } from '../../lib/gen/ChatMessage';
import type { EventKind } from '../../lib/gen/EventKind';
import type { EventRow } from '../../lib/gen/EventRow';
import { interruptAgent, stopThread, stopThreadByEsc } from '../../lib/state/actions';
import { ESC_STOP_WINDOW } from '../../lib/state/esc';
import { installNotifications } from '../../lib/state/notify';
import { stopping } from '../../lib/state/stops';
import { store } from '../../lib/state/store.svelte';
import { toasts } from '../../lib/state/toasts.svelte';
import { ui } from '../../lib/state/ui.svelte';
import { mkAgent } from '../../office/demo/script';
import { draftSlot, loadDraft, nextAttId, saveDraft, seedDraft, withoutSeed, type Att } from '../drafts';
import { canRetry, messageOf, troublesOf, troubleTitle } from '../outcomes';
import { recall } from '../recall';
import { buildSpine } from '../spine';

let eid = 100;
const row = (kind: unknown, o: Partial<EventRow> = {}): EventRow => ({ id: ++eid, ts: eid, agent_id: 'l1', task_id: null, run_id: null, kind: kind as EventKind, ...o });
const started = (input: string | null, o: Partial<EventRow> = {}) => row({ type: 'turn_started', input }, o);
const ended = (status: string, o: Partial<EventRow> = {}) => row({ type: 'turn_completed', status, result_text: null, duration_ms: null }, o);
const error = (message: string, fatal = false, o: Partial<EventRow> = {}) => row({ type: 'error', message, fatal }, o);
const notYou = () => false;

describe('troublesOf', () => {
  it('a failed turn is one line: the turn failed, with the last error of that turn as its reason', () => {
    const evs = [started('fix it', { ts: 10 }), error('first', false, { ts: 11 }), error('codex refused the turn: no', false, { id: 7, ts: 12 }), ended('failed', { ts: 13 })];
    const t = troublesOf(evs, 'l1', notYou);
    expect(t.map((x) => [x.what, x.reason])).toEqual([['problem', 'first'], ['failed', 'codex refused the turn: no']]);
    // Keyed by the error's own event (the row does not remount when the turn then fails), and it knows its turn.
    expect(t[1]).toMatchObject({ eventId: 7, ts: 12, turn: { start: 10, input: 'fix it' } });
  });

  it('a turn that failed without saying why still gets its line', () => {
    const end = ended('failed');
    expect(troublesOf([started('x'), end], 'l1', notYou)).toMatchObject([{ what: 'failed', reason: null, eventId: end.id }]);
  });

  it('a fatal error says the agent stopped, and a failed turn after it adds nothing', () => {
    const t = troublesOf([started('x'), error('claude exited unexpectedly (code 1)', true), ended('failed')], 'l1', notYou);
    expect(t.map((x) => x.what)).toEqual(['stopped']);
  });

  it('an error in a turn that completed is a problem the agent got past', () => {
    expect(troublesOf([started('x'), error('model gpt-9 is not offered by this agent'), ended('completed')], 'l1', notYou).map((x) => x.what)).toEqual(['problem']);
  });

  it('an interrupted turn is reported unless you stopped it yourself', () => {
    const evs = [started('a', { ts: 10 }), ended('interrupted', { ts: 20 }), started('b', { ts: 30 }), ended('interrupted', { ts: 40 })];
    expect(troublesOf(evs, 'l1', notYou).map((x) => [x.what, x.ts])).toEqual([['interrupted', 20], ['interrupted', 40]]);
    expect(troublesOf(evs, 'l1', (start) => start === 10).map((x) => x.ts)).toEqual([40]);
  });

  it('a turn that used all its steps says so; a completed turn says nothing', () => {
    expect(troublesOf([started('x'), ended('max_turns'), started('y'), ended('completed')], 'l1', notYou).map((x) => x.what)).toEqual(['limit']);
  });

  it("another agent's errors are not this agent's, and an error between turns has no turn", () => {
    const evs = [started('x'), error('theirs', false, { agent_id: 'w1' }), ended('completed'), error('mine, between turns')];
    expect(troublesOf(evs, 'l1', notYou)).toMatchObject([{ reason: 'mine, between turns', turn: null }]);
  });

  it('says it in plain words, and offers Retry only when the turn or the agent gave up', () => {
    expect(troubleTitle('failed', 'Mira')).toBe('Mira’s turn failed');
    expect(troubleTitle('stopped', 'Mira')).toBe('Mira stopped');
    expect((['problem', 'stopped', 'failed', 'interrupted', 'limit'] as const).filter(canRetry)).toEqual(['stopped', 'failed']);
  });
});

describe('messageOf', () => {
  const yours = [{ ts: 5, text: 'Fix the login' }, { ts: 8, text: 'Add  tests' }, { ts: 50, text: 'Add tests' }];

  it('finds the message of yours that the turn carried, however Codly wrapped it', () => {
    expect(messageOf({ start: 10, input: '# Board\nnothing yet\n\nFix the\nlogin' }, yours)).toBe('Fix the login');
    // The newest one sent before the turn started: a later copy is not what that turn ran.
    expect(messageOf({ start: 10, input: 'Add tests' }, yours)).toBe('Add  tests');
  });

  it('a turn Codly started for another reason has no message of yours to send again', () => {
    expect(messageOf({ start: 10, input: 'A worker asks: which port?' }, yours)).toBeNull();
    expect(messageOf({ start: 10, input: null }, yours)).toBeNull();
    expect(messageOf(null, yours)).toBeNull();
  });
});

describe('the conversation', () => {
  const lead = mkAgent('l1', 'Mira', 'claude', 'lead', 0, 'working', 'lead');
  const chat = (id: string, ts: number, text: string): ChatMessage => ({ id, thread: 'lead', author: 'user', text, attachment: null, ts });
  const apply = (...evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);
  const troubles = () => buildSpine('lead').flatMap((i) => (i.kind === 'trouble' ? [i] : []));

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    for (const t of toasts.list) toasts.dismiss(t.id);
    store.agents = [];
    store.chats = [];
    store.events = [];
    store.live = {};
    ui.escAsk = null;
  });

  it('a failed turn shows where it happened, with its reason and your message to send again', () => {
    store.agents = [lead];
    store.chats = [chat('c1', 1, 'Fix the login')];
    apply(started('# Board\n\nFix the login', { ts: 2 }), error('codex refused the turn: usage limit', false, { ts: 3 }), ended('failed', { ts: 4 }));
    expect(troubles()).toMatchObject([{ agentId: 'l1', what: 'failed', reason: 'codex refused the turn: usage limit', retry: 'Fix the login', ts: 3 }]);
    // In order: your message, then what went wrong with it.
    expect(buildSpine('lead').map((i) => i.kind)).toEqual(['user', 'trouble']);
  });

  it('Stop interrupts the running turn once, says "Stopping" until it ends, and that end is not a problem', () => {
    const core = vi.spyOn(api, 'interruptAgent').mockResolvedValue();
    store.agents = [lead];
    expect(stopThread('lead')).toBe(false); // nothing runs: Esc keeps its old meaning
    apply(started('do it'));
    expect(stopThread('lead')).toBe(true);
    expect(stopThread('lead')).toBe(true); // a second Esc while the CLI lets go
    expect(core).toHaveBeenCalledTimes(1);
    expect(stopping('l1')).toBe(true);
    apply(ended('interrupted'));
    expect(stopping('l1')).toBe(false);
    expect(troubles()).toEqual([]);
    expect(stopThread('lead')).toBe(false);
  });

  it('Esc asks first, stops on the second press inside the window, and the thread then says the turn was interrupted', () => {
    vi.useFakeTimers();
    const core = vi.spyOn(api, 'interruptAgent').mockResolvedValue();
    store.agents = [lead];
    expect(stopThreadByEsc('lead')).toBe(false); // nothing runs: Esc keeps its old meaning
    apply(started('do it'));
    // One press stops nothing: the composer row asks.
    expect(stopThreadByEsc('lead')).toBe(true);
    expect(core).not.toHaveBeenCalled();
    expect(ui.escAsk?.thread).toBe('lead');
    // The question goes by itself, and the next press asks again.
    vi.advanceTimersByTime(ESC_STOP_WINDOW);
    expect(ui.escAsk).toBeNull();
    expect(stopThreadByEsc('lead')).toBe(true);
    expect(core).not.toHaveBeenCalled();
    // The second press inside the window stops the turn.
    vi.advanceTimersByTime(ESC_STOP_WINDOW / 2);
    expect(stopThreadByEsc('lead')).toBe(true);
    expect(core).toHaveBeenCalledTimes(1);
    expect(ui.escAsk).toBeNull();
    expect(stopping('l1')).toBe(true);
    expect(stopThreadByEsc('lead')).toBe(true); // while the CLI lets go: nothing new is asked
    expect(core).toHaveBeenCalledTimes(1);
    apply(ended('interrupted'));
    expect(troubles().map((t) => t.what)).toEqual(['interrupted']);
  });

  it('an interrupt the core refused is taken back: a later interruption is reported', async () => {
    vi.spyOn(api, 'interruptAgent').mockRejectedValue(new Error("that agent's process is gone"));
    store.agents = [lead];
    apply(started('do it'));
    expect(await interruptAgent('l1')).toBe(false);
    expect(stopping('l1')).toBe(false);
    apply(ended('interrupted'));
    expect(troubles().map((t) => t.what)).toEqual(['interrupted']);
  });

  it("what the agent's CLI told about the run is a line where it was told, and only in its own thread", () => {
    store.agents = [lead];
    store.chats = [chat('c1', 1, 'Fix the login')];
    const notice = (text: string, o: Partial<EventRow> = {}) => row({ type: 'notice', text }, o);
    const begun = started('Fix the login', { ts: 2 });
    const said = notice('MCP server linear did not start, so this agent works without its tools: connection refused', { ts: 3 });
    apply(begun, said, notice('Goal: ship it', { ts: 4, agent_id: 'w1' }), ended('completed', { ts: 5 }));
    const notices = buildSpine('lead').flatMap((i) => (i.kind === 'notice' ? [i] : []));
    expect(notices).toMatchObject([{ agentId: 'l1', eventId: said.id, ts: 3, text: 'MCP server linear did not start, so this agent works without its tools: connection refused' }]);
    expect(buildSpine('lead').map((i) => i.kind)).toEqual(['user', 'notice']);
  });

  it('a fatal error toasts only when you are not reading that conversation', () => {
    const off = installNotifications();
    store.agents = [lead];
    ui.room = 'home';
    ui.mode = 'blend';
    ui.thread = 'lead';
    apply(error('claude exited unexpectedly (code 1)', true));
    expect(toasts.list).toHaveLength(0); // the line in the conversation says it
    ui.room = 'plan';
    apply(error('claude exited unexpectedly (code 1)', true));
    expect(toasts.list.map((t) => t.title)).toEqual(['Mira stopped']);
    off();
    ui.room = 'home';
  });
});

describe('composer drafts', () => {
  const file = (name: string): Att => ({ id: nextAttId(), name, path: null, url: null, failed: false });

  it('a draft is kept per office and thread, with its files, and an upload still on its way lands in it', () => {
    const a = draftSlot('office-a', 'lead');
    const b = draftSlot('office-b', 'lead');
    const files = [file('shot.png')];
    saveDraft(a, 'half a prompt', files);
    expect(loadDraft(b)).toEqual({ text: '', files: [] });
    // The composer left the screen and came back: same text, same file objects.
    files[0].path = '.codly/attachments/shot.png';
    expect(loadDraft(a).text).toBe('half a prompt');
    expect(loadDraft(a).files[0]).toBe(files[0]);
    expect(loadDraft(a).files[0].path).toBe('.codly/attachments/shot.png');
    saveDraft(a, '', []);
    expect(loadDraft(a)).toEqual({ text: '', files: [] });
  });

  it('attachment ids never repeat, so kept files and new ones can share a list', () => {
    expect(new Set([file('a').id, file('b').id, file('c').id]).size).toBe(3);
  });

  it('Edit on an earlier message never replaces a draft; other seeds do, as before', () => {
    const edit = { text: 'the old message', keep: true };
    expect(seedDraft('half a prompt', edit)).toEqual({ text: 'half a prompt', kept: true });
    expect(seedDraft('', edit)).toEqual({ text: 'the old message', kept: false });
    expect(seedDraft('  \n', edit)).toEqual({ text: 'the old message', kept: false });
    // Edit pressed twice on the same message: nothing to warn about.
    expect(seedDraft('the old message', edit)).toEqual({ text: 'the old message', kept: false });
    expect(seedDraft('half a prompt', { text: 'Change the plan: ', keep: false })).toEqual({ text: 'Change the plan: ', kept: false });
  });

  it('a seed that replaces an earlier one keeps what you wrote around it', () => {
    const again = { text: 'Comments (2)', keep: false, replaces: 'Comments (1)' };
    // The earlier text is still there: only that part changes.
    expect(seedDraft('Comments (1)', again)).toEqual({ text: 'Comments (2)', kept: false });
    expect(seedDraft('Before.\n\nComments (1)\n\nAlso run the tests.', again)).toEqual({ text: 'Before.\n\nComments (2)\n\nAlso run the tests.', kept: false });
    // You rewrote it: your words stay, and the new text comes under them.
    expect(seedDraft('My own words.\n', again)).toEqual({ text: 'My own words.\n\nComments (2)', kept: false });
    // An empty composer just takes the new text.
    expect(seedDraft('', again)).toEqual({ text: 'Comments (2)', kept: false });
    expect(seedDraft(' \n', again)).toEqual({ text: 'Comments (2)', kept: false });
  });

  it('a seed taken out again leaves what you wrote around it, and only a seed that is still as it was put', () => {
    expect(withoutSeed('Comments (1)', 'Comments (1)')).toBe('');
    expect(withoutSeed('Before.\n\nComments (1)\n\nAlso run the tests.', 'Comments (1)')).toBe('Before.\n\nAlso run the tests.');
    expect(withoutSeed('Comments (1)\n\nAlso run the tests.', 'Comments (1)')).toBe('Also run the tests.');
    // You changed it: it is your text now, and nothing is taken out.
    expect(withoutSeed('Comments (one)', 'Comments (1)')).toBeNull();
    expect(withoutSeed('', 'Comments (1)')).toBeNull();
  });
});

describe('recall (↑ in an empty composer)', () => {
  const sent = ['first', 'second', 'third'];

  it('walks back from the newest message and stops at the oldest', () => {
    expect(recall(sent, null, 'older')).toEqual({ at: 0, text: 'third' });
    expect(recall(sent, 0, 'older')).toEqual({ at: 1, text: 'second' });
    expect(recall(sent, 2, 'older')).toBeNull();
    expect(recall([], null, 'older')).toBeNull();
  });

  it('comes forward again, and past the newest the line is empty', () => {
    expect(recall(sent, 2, 'newer')).toEqual({ at: 1, text: 'second' });
    expect(recall(sent, 0, 'newer')).toEqual({ at: null, text: '' });
    expect(recall(sent, null, 'newer')).toBeNull(); // not recalling: ↓ moves the caret as usual
  });
});
