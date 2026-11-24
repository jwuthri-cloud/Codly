import { describe, expect, it } from 'vitest';
import { digitPick, digitQuestion, isTextField, sendsFromField } from '../questionKeys';

const press = (key: string, o: Partial<{ mod: boolean; alt: boolean; shift: boolean }> = {}) => ({ key, mod: false, alt: false, shift: false, ...o });

describe('digitPick', () => {
  it('picks options 1 to 9 by their place', () => {
    expect(digitPick(press('1'), false, 3)).toBe(0);
    expect(digitPick(press('3'), false, 3)).toBe(2);
    expect(digitPick(press('9'), false, 9)).toBe(8);
  });
  it('does nothing for a digit past the last option, 0, or another key', () => {
    expect(digitPick(press('4'), false, 3)).toBeNull();
    expect(digitPick(press('0'), false, 3)).toBeNull();
    expect(digitPick(press('a'), false, 3)).toBeNull();
    expect(digitPick(press('Enter'), false, 3)).toBeNull();
  });
  it('leaves a digit with a modifier to the other shortcuts (⌘1 is the office)', () => {
    expect(digitPick(press('1', { mod: true }), false, 3)).toBeNull();
    expect(digitPick(press('1', { alt: true }), false, 3)).toBeNull();
  });
  it('leaves a digit typed in a text field to that field', () => {
    expect(digitPick(press('1'), true, 3)).toBeNull();
  });
});

describe('sendsFromField', () => {
  it('sends on ↵ and ⌘↵, not on ⇧↵ (a new line) or while an IME composes', () => {
    expect(sendsFromField(press('Enter'))).toBe(true);
    expect(sendsFromField(press('Enter', { mod: true }))).toBe(true);
    expect(sendsFromField(press('Enter', { shift: true }))).toBe(false);
    expect(sendsFromField({ ...press('Enter'), composing: true })).toBe(false);
    expect(sendsFromField(press('a'))).toBe(false);
  });
});

describe('digitQuestion', () => {
  it('is the first question with options and no answer yet', () => {
    expect(digitQuestion([{ options: 2, answered: true }, { options: 0, answered: false }, { options: 3, answered: false }])).toBe(2);
  });
  it('is the last question with options once all have an answer', () => {
    expect(digitQuestion([{ options: 2, answered: true }, { options: 3, answered: true }, { options: 0, answered: false }])).toBe(1);
  });
  it('is -1 when no question has options', () => {
    expect(digitQuestion([{ options: 0, answered: false }])).toBe(-1);
    expect(digitQuestion([])).toBe(-1);
  });
});

describe('isTextField', () => {
  // The fields the rule reads, as an element has them (there is no DOM in these tests).
  const el = (tagName: string, o: { type?: string; editable?: boolean; inTerminal?: boolean } = {}) =>
    ({ tagName, type: o.type ?? 'text', isContentEditable: !!o.editable, closest: (s: string) => (s === '.xterm' && o.inTerminal ? {} : null) }) as unknown as Element;
  it('is true where keys write text', () => {
    expect(isTextField(el('TEXTAREA'))).toBe(true);
    expect(isTextField(el('INPUT'))).toBe(true);
    expect(isTextField(el('INPUT', { type: 'search' }))).toBe(true);
    expect(isTextField(el('SELECT'))).toBe(true);
    expect(isTextField(el('DIV', { editable: true }))).toBe(true);
    expect(isTextField(el('SPAN', { inTerminal: true }))).toBe(true);
  });
  it('is false on an option, a button, the page', () => {
    expect(isTextField(el('INPUT', { type: 'radio' }))).toBe(false);
    expect(isTextField(el('INPUT', { type: 'checkbox' }))).toBe(false);
    expect(isTextField(el('BUTTON'))).toBe(false);
    expect(isTextField(el('BODY'))).toBe(false);
    expect(isTextField(null)).toBe(false);
  });
});
