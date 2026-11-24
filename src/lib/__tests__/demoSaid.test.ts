// The browser demo puts what you said with an answer on record the way the core does: an
// `approval_said` row just before the answer, and only when the answer carried words.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { denyNoteMessage, saidWith } from '../demo/said';
import { REPO, Sim } from '../demo/sim';
import { Story, type StoryOptions } from '../demo/story';
import leadRs from '../../../crates/codly-core/src/orchestrator/lead.rs?raw';
import type { Approval } from '../gen/Approval';
import type { EventKind } from '../gen/EventKind';
import { DEFAULT_SETTINGS } from '../state/store.svelte';

function demo(story: StoryOptions) {
  const sim = new Sim({ speed: 1, skipMs: 0, autoplay: false }, { ...DEFAULT_SETTINGS });
  const s = new Story(sim, story);
  s.openWorkspace(REPO);
  return { sim, story: s };
}

/** Let the script play until `done`, for `seconds` of its time at most. */
async function until(done: () => boolean, seconds = 400) {
  for (let i = 0; i < seconds * 2 && !done(); i++) await vi.advanceTimersByTimeAsync(500);
  expect(done(), 'the script got there in time').toBe(true);
}

const requests = (sim: Sim, test: (a: Approval) => boolean) => [...sim.approvals.values()].filter(test);
/** The rows about one request's answer, in order: what you said, then how it ended. */
const rows = (sim: Sim, id: string): EventKind[] =>
  sim.log.map((e) => e.kind).filter((k) => (k.type === 'approval_said' || k.type === 'approval_resolved') && k.approval_id === id);

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date', 'performance'] });
  // Every pause and every length of a step is the same in each run.
  vi.spyOn(Math, 'random').mockReturnValue(0.5);
  // The story draws one picture on a canvas; a test has none.
  vi.stubGlobal('OffscreenCanvas', class { getContext() { return null; } });
});
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('what you said with an answer, in the demo', () => {
  it('a plan sent back with a note, then accepted with your changes, says both on record', async () => {
    const { sim, story } = demo({ planMode: true, yourRequestFirst: true });
    const plans = () => requests(sim, (a) => a.subject.kind === 'tool' && a.subject.name === 'ExitPlanMode');
    await until(() => plans().length === 1);

    const first = plans()[0];
    story.resolveApproval(first.id, 'deny', '  Say which file changes.  ');
    expect(rows(sim, first.id)).toEqual([
      { type: 'approval_said', approval_id: first.id, note: 'Say which file changes.', answer: null, plan_changed: false },
      { type: 'approval_resolved', approval_id: first.id, status: 'denied', by: 'user' },
    ]);

    await until(() => plans().length === 2);
    const second = plans()[1];
    story.resolveApproval(second.id, 'allow_once', null, { plan: '1. Change README.md only.' });
    expect(rows(sim, second.id)).toEqual([
      { type: 'approval_said', approval_id: second.id, note: null, answer: null, plan_changed: true },
      { type: 'approval_resolved', approval_id: second.id, status: 'approved', by: 'user' },
    ]);

    // An answer with no words is one row, as before.
    const edits = () => requests(sim, (a) => a.subject.kind === 'file_change');
    await until(() => edits().length === 1);
    story.resolveApproval(edits()[0].id, 'allow_once', null);
    expect(rows(sim, edits()[0].id)).toEqual([{ type: 'approval_resolved', approval_id: edits()[0].id, status: 'approved', by: 'user' }]);
  });

  it('says what the core says: an answer to a question on one line, and no note for a CLI that does not pass it on', () => {
    const request = (subject: Approval['subject']) =>
      ({ id: 'ap', agent_id: 'a', run_id: 'r', task_id: null, call_id: null, subject, reason: null, choices: ['allow_once', 'deny'], risks: [], status: 'pending', decided_by: null, created_at: 0 }) as Approval;
    const question = request({ kind: 'question', questions: [{ question: 'Which sizes?', multiSelect: true }, { question: 'Colour?' }] });
    const command = request({ kind: 'command', command: 'npm install', cwd: null });
    const plan = request({ kind: 'tool', name: 'ExitPlanMode', input: { plan: '1. Add the line' } });

    expect(saidWith(question, 'claude', 'allow_once', null, { 'Which sizes?': ['Small', 'Fast'], 'Colour?': 'Red' })).toEqual({ note: null, answer: 'Small, Fast; Red', plan_changed: false });
    expect(saidWith(question, 'claude', 'deny', null, null)).toBeNull();
    expect(saidWith(command, 'claude', 'deny', 'use pnpm', null)).toEqual({ note: 'use pnpm', answer: null, plan_changed: false });
    // Another CLI gets the note of a deny as a message: on record too. The note of a stop never reaches it.
    expect(saidWith(command, 'codex', 'deny', 'use pnpm', null)).toEqual({ note: 'use pnpm', answer: null, plan_changed: false });
    expect(saidWith(command, 'codex', 'cancel', 'use pnpm', null)).toBeNull();
    expect(saidWith(command, 'claude', 'cancel', 'use pnpm', null)).toEqual({ note: 'use pnpm', answer: null, plan_changed: false });
    expect(saidWith(command, 'claude', 'allow_once', null, null)).toBeNull();
    expect(saidWith(command, 'claude', 'deny', '   ', null)).toBeNull();
    // The agent's own text back is no change, and only Claude Code's plan request takes one.
    expect(saidWith(plan, 'claude', 'allow_once', null, { plan: '1. Add the line' })).toBeNull();
    expect(saidWith(plan, 'codex', 'allow_once', null, { plan: '1. Add two lines' })).toBeNull();
    expect(saidWith(command, 'claude', 'deny', 'x'.repeat(3000), null)?.note).toHaveLength(2000);
  });

  it('the note of a deny to an agent of another CLI becomes a message, as the core sends it', () => {
    const noa = { id: 'noa', provider: 'codex', role: 'worker', thread: null } as const;
    expect(denyNoteMessage(noa, 'deny', '  Keep the old heading  ')).toEqual({ thread: 'agent:noa', author: 'user', text: 'About the request I just denied: Keep the old heading' });
    expect(denyNoteMessage({ ...noa, role: 'lead', thread: 'lead:x' }, 'deny', 'Not now')).toEqual({ thread: 'lead:x', author: 'codly', text: 'Your note with the deny: Not now' });
    // Claude Code takes the note with its answer; a stop, or no note, sends nothing.
    expect(denyNoteMessage({ ...noa, provider: 'claude' }, 'deny', 'Not now')).toBeNull();
    expect(denyNoteMessage(noa, 'cancel', 'Not now')).toBeNull();
    expect(denyNoteMessage(noa, 'deny', '  ')).toBeNull();
  });

  it('pins the words of the core', () => {
    expect(leadRs).toContain('format!("About the request I just denied: {note}")');
    expect(leadRs).toContain('&format!("Your note with the deny: {note}")');
  });
});
