// The rules of a request card that waits: when it takes the approval keys (what it asks and its
// buttons in view), when its answer may be sent, and when its answer field takes the focus.
import { afterEach, describe, expect, it } from 'vitest';
import { cardInView, registerCardView } from '../../state/keys';
import { answerMissing, answerTakesFocus, takesKeys, type Box } from '../approvalCard';

const box = (top: number, bottom: number, left = 217, right = 873): Box => ({ top, bottom, left, right });
/** The thread's visible box at 1024x640, as the review measured it: [185, 162, 905, 494]. */
const thread = box(162, 494, 185, 905);

describe('the card that takes the approval keys', () => {
  it('takes them while what it asks and its buttons are both in view', () => {
    expect(takesKeys([box(260, 303), box(420, 452)], thread)).toBe(true);
    // At the edges, give or take a pixel.
    expect(takesKeys([box(161.5, 200), box(460, 494.6)], thread)).toBe(true);
  });

  it('a card scrolled away takes nothing: the review’s card with 16 px in view', () => {
    // Bo's card [217, -90, 873, 178]: its command above the thread, its buttons at 140–172.
    expect(takesKeys([box(-40, 3), box(140, 172)], thread)).toBe(false);
  });

  it('half of it is not enough: the command in view and the buttons below, or the reverse', () => {
    expect(takesKeys([box(400, 443), box(500, 532)], thread)).toBe(false);
    expect(takesKeys([box(150, 193), box(300, 332)], thread)).toBe(false);
    // Cut at the side (a narrow window).
    expect(takesKeys([box(200, 243, 100, 600), box(300, 332)], thread)).toBe(false);
  });

  it('a card with nothing to measure takes nothing', () => {
    expect(takesKeys([], thread)).toBe(false);
  });
});

describe('the cards on screen, as the keys read them', () => {
  const off: (() => void)[] = [];
  afterEach(() => off.splice(0).forEach((f) => f()));

  it('a request is in view when a card that shows it says so, for its twins too', () => {
    let seen = false;
    off.push(registerCardView(['ap1', 'ap2'], () => seen));
    expect([cardInView('ap1'), cardInView('ap2')]).toEqual([false, false]);
    seen = true;
    expect([cardInView('ap1'), cardInView('ap2')]).toEqual([true, true]);
    // A request with no card on screen is never in view: its key only brings it there.
    expect(cardInView('ap3')).toBe(false);
  });

  it('the sheet and the conversation can both show it: one card in view is enough, and a card that goes is forgotten', () => {
    const sheet = registerCardView(['ap1'], () => false);
    const conversation = registerCardView(['ap1'], () => true);
    off.push(sheet);
    expect(cardInView('ap1')).toBe(true);
    conversation();
    expect(cardInView('ap1')).toBe(false);
  });
});

describe('the answer of a question card', () => {
  const q = (options: number, own: boolean, answered: boolean) => ({ options, own, answered });

  it('a question that takes only typed text waits for some: an empty answer is never sent', () => {
    expect(answerMissing([q(0, true, false)], null)).toBe(true);
    expect(answerMissing([q(0, true, true)], null)).toBe(false);
    // The one field of a question Codly cannot list (an MCP server asks through Codex).
    expect(answerMissing([], '')).toBe(true);
    expect(answerMissing([], '   \n')).toBe(true);
    expect(answerMissing([], 'Platform')).toBe(false);
  });

  it('a question with options and a field of its own needs one of the two; every question needs its answer', () => {
    expect(answerMissing([q(2, true, false)], null)).toBe(true);
    expect(answerMissing([q(2, true, true)], null)).toBe(false);
    expect(answerMissing([q(2, false, true), q(3, false, false)], null)).toBe(true);
    expect(answerMissing([q(2, false, true), q(3, false, true)], null)).toBe(false);
  });

  it('a card with nothing to answer has nothing missing', () => {
    expect(answerMissing([], null)).toBe(false);
    expect(answerMissing([q(0, false, false)], null)).toBe(false);
  });
});

describe('the focus of a question card that becomes the one that waits', () => {
  it('goes to its field when the answer can only be typed', () => {
    expect(answerTakesFocus([{ options: 0, own: true }], false, false)).toBe(true);
    expect(answerTakesFocus([], true, false)).toBe(true);
  });

  it('stays where it is while you type somewhere else', () => {
    expect(answerTakesFocus([{ options: 0, own: true }], false, true)).toBe(false);
    expect(answerTakesFocus([], true, true)).toBe(false);
  });

  it('stays where it is when there are options to pick with the digits, or several questions', () => {
    expect(answerTakesFocus([{ options: 2, own: true }], false, false)).toBe(false);
    expect(answerTakesFocus([{ options: 0, own: true }, { options: 0, own: true }], false, false)).toBe(false);
    expect(answerTakesFocus([], false, false)).toBe(false);
  });
});
