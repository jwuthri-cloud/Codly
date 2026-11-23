import { describe, expect, it } from 'vitest';
import { questionKeysOf, registerQuestionKeys, type QuestionKeys } from '../state/keys';

const keys = (): QuestionKeys => ({ options: () => 2, pick: () => {}, send: () => {} });

describe('the keys of a question card', () => {
  it('go to the card shown last, and back to the first when it goes', () => {
    const inThread = keys();
    const inSheet = keys();
    const offThread = registerQuestionKeys('ap-1', inThread);
    const offSheet = registerQuestionKeys('ap-1', inSheet);
    expect(questionKeysOf('ap-1')).toBe(inSheet);
    offSheet();
    expect(questionKeysOf('ap-1')).toBe(inThread);
    offThread();
    expect(questionKeysOf('ap-1')).toBeNull();
  });

  it('belong to their own request only', () => {
    const off = registerQuestionKeys('ap-2', keys());
    expect(questionKeysOf('ap-3')).toBeNull();
    off();
  });
});
