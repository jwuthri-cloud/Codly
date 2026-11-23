import { afterEach, describe, expect, it } from 'vitest';
import type { Approval } from '../../gen/Approval';
import { yourWords } from '../../state/answers';
import { answerText, askedLine, firstQuestion, outcomeBy, outcomeWord, settledLine, settledWord, wordsOf } from '../outcome';

const command = { subject: { kind: 'command', command: 'pnpm install', cwd: null } } satisfies Pick<Approval, 'subject'>;
const question = {
  subject: { kind: 'question', questions: { questions: [{ question: 'Keep the localStorage fallback?', header: 'Fallback', options: [{ label: 'Keep it' }, { label: 'Drop it' }] }] } },
} satisfies Pick<Approval, 'subject'>;

const plan = { subject: { kind: 'tool', name: 'ExitPlanMode', input: { plan: '1. Add the line' } } } satisfies Pick<Approval, 'subject'>;

describe('how a settled request reads', () => {
  it('a permission is approved or denied; a question is answered or skipped', () => {
    expect((['approved', 'denied', 'cancelled', 'expired', 'pending'] as const).map((s) => outcomeWord(s, command))).toEqual(['Approved', 'Denied', 'Stopped', 'Closed', '']);
    expect((['approved', 'denied', 'cancelled', 'expired', 'pending'] as const).map((s) => outcomeWord(s, question))).toEqual(['Answered', 'Skipped', 'Stopped', 'Closed', '']);
  });

  it('says who settled it', () => {
    expect(['user', 'policy', 'codly', 'provider', 'Mira', null].map(outcomeBy)).toEqual(['by you', 'by Codly’s rules', 'by Codly', 'by its CLI', 'by Mira', '']);
  });

  it('puts the answers a question card sends on one line', () => {
    expect(answerText({ 'Keep the localStorage fallback?': 'Keep it' })).toBe('Keep it');
    expect(answerText({ Colour: 'Red', Sizes: ['Small', 'Fast'] })).toBe('Red; Small, Fast');
    expect(answerText({ answer: '  use the settings API  ' })).toBe('use the settings API');
    // Nothing picked, nothing typed, or no answers at all (an approval, a skipped question).
    expect([answerText({ a: '', b: [] }), answerText(null), answerText(undefined), answerText('yes'), answerText(['yes'])]).toEqual([null, null, null, null, null]);
  });

  it('reads the first question of a card, in either shape a CLI sends', () => {
    expect(firstQuestion(question)).toBe('Keep the localStorage fallback?');
    expect(firstQuestion({ subject: { kind: 'question', questions: [{ prompt: 'Which colour?' }] } })).toBe('Which colour?');
    expect(firstQuestion({ subject: { kind: 'question', questions: [] } })).toBeNull();
    expect(firstQuestion(command)).toBeNull();
  });
});

describe('an agent’s history', () => {
  it('says what was asked: an approval, or a question in its own words', () => {
    expect(askedLine(command)).toBe('Asked for approval: pnpm install');
    expect(askedLine({ subject: { kind: 'file_change', paths: ['a.ts'], diff: null } })).toBe('Asked for approval');
    expect(askedLine(question)).toBe('Asked you: Keep the localStorage fallback?');
    expect(askedLine({ subject: { kind: 'question', questions: [] } })).toBe('Asked you a question');
    expect(askedLine(plan, 'claude')).toBe('Asked you to read its plan');
    // Only a Claude agent shows a plan this way: from another CLI it is an approval like any other.
    expect(askedLine(plan, 'codex')).toBe('Asked for approval');
    expect(askedLine(plan)).toBe('Asked for approval');
  });

  it('an answered question reads "Answered by you" with the answer, never "Approval approved"', () => {
    expect(settledLine('approved', 'user', question, 'Keep it')).toBe('Answered by you: Keep it');
    // Answered before the core kept a copy of the answer.
    expect(settledLine('approved', 'user', question, null)).toBe('Answered by you');
    expect(settledLine('denied', 'user', question, null)).toBe('Skipped by you');
    expect(settledLine('cancelled', 'provider', question, 'Keep it')).toBe('Stopped by its CLI');
  });

  it('an approval reads as its closed card does, also when the request itself is no longer held', () => {
    expect(settledLine('approved', 'user', command, null)).toBe('Approved by you');
    expect(settledLine('denied', 'policy', command, null)).toBe('Denied by Codly’s rules');
    expect(settledLine('approved', 'user', null, null)).toBe('Approved by you');
  });

  it('a request that went away with its agent is closed, and says who closed it', () => {
    // You killed the agent: Codly closes what it had asked.
    expect(settledLine('expired', 'codly', command, null)).toBe('Closed by Codly');
    // You stopped its turn: its CLI takes the request back.
    expect(settledLine('cancelled', 'provider', command, null)).toBe('Stopped by its CLI');
  });

  it('a Claude agent’s plan is accepted or sent back, not approved or denied', () => {
    expect(settledLine('approved', 'user', plan, null, 'claude')).toBe('Plan accepted by you');
    expect(settledLine('denied', 'user', plan, null, 'claude')).toBe('Plan sent back by you');
    expect(settledLine('cancelled', 'user', plan, null, 'claude')).toBe('Stopped by you');
    expect(settledLine('denied', 'user', plan, null, 'codex')).toBe('Denied by you');
    expect(settledLine('approved', 'user', plan, null)).toBe('Approved by you');
  });
});

describe('what you said with an answer', () => {
  afterEach(() => yourWords.clear());
  const held = (id: string, a: Pick<Approval, 'subject'>) => ({ id, ...a });

  it('the note of a "no" ends the line of the request it went with, cut to fit', () => {
    yourWords.set('ap1', { note: 'use pnpm, not npm', planChanged: false });
    expect(settledLine('denied', 'user', held('ap1', command), null)).toBe('Denied by you: use pnpm, not npm');
    expect(settledLine('denied', 'user', held('ap2', command), null)).toBe('Denied by you');
    yourWords.set('ap3', { note: `Say which file changes.\n${'x'.repeat(200)}`, planChanged: false });
    const line = settledLine('denied', 'user', held('ap3', plan), null, 'claude');
    expect(line.startsWith('Plan sent back by you: Say which file changes. xxx')).toBe(true);
    expect(line).toHaveLength('Plan sent back by you: '.length + 80);
  });

  it('a plan you changed before you accepted it says so, on the card and in the history', () => {
    yourWords.set('ap1', { note: null, planChanged: true });
    expect(settledLine('approved', 'user', held('ap1', plan), null, 'claude')).toBe('Plan accepted with your changes by you');
    expect(settledLine('approved', 'user', held('ap2', plan), null, 'claude')).toBe('Plan accepted by you');
    // The closed card puts the same word before "by you".
    expect(settledWord('approved', plan, 'claude', true)).toBe('Plan accepted with your changes');
    expect(settledWord('approved', plan, 'claude', false)).toBe('Plan accepted');
    expect(settledWord('denied', plan, 'claude', false)).toBe('Plan sent back');
    // Every other request, and a plan of another CLI, reads as before.
    expect(settledWord('approved', plan, 'codex', true)).toBe('Approved');
    expect(settledWord('denied', command, 'claude', false)).toBe('Denied');
    expect(settledWord('approved', question, null, false)).toBe('Answered');
    expect(settledWord('expired', null, null, false)).toBe('Closed');
  });

  it('shows only on a row that reads "by you", and only for a request that is still known', () => {
    yourWords.set('ap1', { note: 'use pnpm', planChanged: false });
    expect(wordsOf(held('ap1', command), 'user')).toEqual({ note: 'use pnpm', planChanged: false });
    // Its CLI took the request back, or Codly closed it, while your answer was on its way.
    expect(wordsOf(held('ap1', command), 'provider')).toBeNull();
    expect(settledLine('cancelled', 'provider', held('ap1', command), null)).toBe('Stopped by its CLI');
    expect(settledLine('expired', 'codly', held('ap1', command), null)).toBe('Closed by Codly');
    expect(wordsOf(command, 'user')).toBeNull();
    expect(wordsOf(null, 'user')).toBeNull();
    expect(wordsOf(held('ap2', command), 'user')).toBeNull();
  });
});
