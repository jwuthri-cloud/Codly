// The approval keys through the app's key map (installKeys): ⌘↵, ⌘⌫ and a digit answer only a
// card that is in view and still. For a card out of view they answer nothing and bring the card
// into view (`ui.revealApproval`). The page is a stand-in: a key pressed outside every text field.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Agent } from '../gen/Agent';
import type { Approval } from '../gen/Approval';
import type { KeyAct } from '../util/approvalCard';
import { resolveApprovals } from '../state/actions';
import { installKeys, registerCardView, registerQuestionKeys } from '../state/keys';
import { store } from '../state/store.svelte';
import { ui } from '../state/ui.svelte';

vi.mock('../state/actions', async (actual) => ({ ...(await actual<typeof import('../state/actions')>()), resolveApprovals: vi.fn(async () => true) }));

let onKey: ((e: KeyboardEvent) => void) | null = null;
let off: () => void = () => {};
/** The clock of `performance.now`: each test starts long after the answer of the one before. */
let clock = 0;

/** A key pressed on the page's body. */
function press(key: string, mod = false) {
  const target = { tagName: 'BODY', isContentEditable: false, closest: () => null };
  onKey!({ key, repeat: false, metaKey: mod, ctrlKey: false, altKey: false, shiftKey: false, defaultPrevented: false, target, preventDefault() {} } as unknown as KeyboardEvent);
}

const ada = { id: 'ada', name: 'Ada', role: 'solo', status: 'working', provider: 'claude', task_id: null, worktree: '/wt/ada' } as Agent;
const waiting = (id: string, subject: Approval['subject']) =>
  ({ id, agent_id: 'ada', run_id: 'r', task_id: null, call_id: null, subject, reason: null, choices: ['allow_once', 'deny'], risks: [], status: 'pending', decided_by: null, created_at: 1 }) as Approval;
const command = waiting('ap1', { kind: 'command', command: 'npm test', cwd: null });
const question = waiting('q1', { kind: 'question', questions: [{ question: 'Keep the fallback?' }] });

/** The card of `id` shows, and says what a key does to it now. */
let unregister: (() => void)[] = [];
const card = (id: string, act: KeyAct) => unregister.push(registerCardView([id], () => act));

beforeEach(() => {
  clock += 100_000;
  vi.spyOn(performance, 'now').mockImplementation(() => clock);
  vi.stubGlobal('window', { addEventListener: (_: string, fn: (e: KeyboardEvent) => void) => (onKey = fn), removeEventListener: () => (onKey = null) });
  vi.stubGlobal('document', { querySelectorAll: () => [] });
  off = installKeys();
  ui.go('home');
  ui.mode = 'blend';
  store.agents = [ada];
  ui.thread = 'agent:ada';
  ui.revealApproval = null;
});

afterEach(() => {
  off();
  for (const u of unregister) u();
  unregister = [];
  store.approvals = [];
  store.agents = [];
  ui.thread = 'lead';
  ui.revealApproval = null;
  vi.mocked(resolveApprovals).mockClear();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('a card that is not in view', () => {
  it('⌘↵ answers nothing and brings the card into view', () => {
    store.approvals = [command];
    card('ap1', 'reveal');
    press('Enter', true);
    expect(resolveApprovals).not.toHaveBeenCalled();
    expect(ui.revealApproval).toBe('ap1');
  });

  it('⌘⌫ answers nothing and brings the card into view', () => {
    store.approvals = [command];
    press('Backspace', true); // no card shows it at all: out of view too
    expect(resolveApprovals).not.toHaveBeenCalled();
    expect(ui.revealApproval).toBe('ap1');
  });

  it('a digit picks nothing and brings the question into view', () => {
    store.approvals = [question];
    const pick = vi.fn();
    unregister.push(registerQuestionKeys('q1', { options: () => 2, pick, send: vi.fn() }));
    card('q1', 'reveal');
    press('1');
    expect(pick).not.toHaveBeenCalled();
    expect(ui.revealApproval).toBe('q1');
  });
});

describe('the same card in view', () => {
  it('⌘↵ allows it once', () => {
    store.approvals = [command];
    card('ap1', 'answer');
    press('Enter', true);
    expect(resolveApprovals).toHaveBeenCalledWith(['ap1'], 'allow_once');
    expect(ui.revealApproval).toBeNull();
  });

  it('⌘⌫ says no', () => {
    store.approvals = [command];
    card('ap1', 'answer');
    press('Backspace', true);
    expect(resolveApprovals).toHaveBeenCalledWith(['ap1'], 'deny');
  });

  it('a digit picks its option', () => {
    store.approvals = [question];
    const pick = vi.fn();
    unregister.push(registerQuestionKeys('q1', { options: () => 2, pick, send: vi.fn() }));
    card('q1', 'answer');
    press('2');
    expect(pick).toHaveBeenCalledWith(1);
  });

  it('while the card settles a key does nothing, and brings nothing into view', () => {
    store.approvals = [command];
    card('ap1', 'wait');
    press('Enter', true);
    expect(resolveApprovals).not.toHaveBeenCalled();
    expect(ui.revealApproval).toBeNull();
  });
});
