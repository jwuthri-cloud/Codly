import { describe, expect, it } from 'vitest';
import type { Approval } from '../../gen/Approval';
import { answerText, askedLine, firstQuestion, outcomeBy, outcomeWord, settledLine } from '../outcome';

const command = { subject: { kind: 'command', command: 'pnpm install', cwd: null } } satisfies Pick<Approval, 'subject'>;
const question = {
  subject: { kind: 'question', questions: { questions: [{ question: 'Keep the localStorage fallback?', header: 'Fallback', options: [{ label: 'Keep it' }, { label: 'Drop it' }] }] } },
} satisfies Pick<Approval, 'subject'>;

describe('how a settled request reads', () => {
  it('a permission is approved or denied; a question is answered or skipped', () => {
    expect((['approved', 'denied', 'cancelled', 'expired', 'pending'] as const).map((s) => outcomeWord(s, command))).toEqual(['Approved', 'Denied', 'Stopped', 'Expired', '']);
    expect((['approved', 'denied', 'cancelled', 'expired', 'pending'] as const).map((s) => outcomeWord(s, question))).toEqual(['Answered', 'Skipped', 'Stopped', 'Expired', '']);
  });

  it('says who settled it', () => {
    expect(['user', 'policy', 'provider', null].map(outcomeBy)).toEqual(['by you', 'by Codly’s rules', 'by provider', '']);
  });

  it('puts the answers a question card sends on one line', () => {
    expect(answerText({ 'Keep the localStorage fallback?': 'Keep it' })).toBe('Keep it');
    expect(answerText({ Colour: 'Red', Sizes: ['Small', 'Fast'] })).toBe('Red · Small, Fast');
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
  });

  it('an answered question reads "Answered by you" with the answer, never "Approval approved"', () => {
    expect(settledLine('approved', 'user', question, 'Keep it')).toBe('Answered by you: Keep it');
    // Answered before this window opened: the core keeps no copy of the answer.
    expect(settledLine('approved', 'user', question, null)).toBe('Answered by you');
    expect(settledLine('denied', 'user', question, null)).toBe('Skipped by you');
    expect(settledLine('cancelled', 'provider', question, 'Keep it')).toBe('Stopped by provider');
  });

  it('an approval keeps its words, also when the request itself is no longer held', () => {
    expect(settledLine('approved', 'user', command, null)).toBe('Approval approved by you');
    expect(settledLine('denied', 'policy', command, null)).toBe('Approval denied by policy');
    expect(settledLine('approved', 'user', null, null)).toBe('Approval approved by you');
  });
});
