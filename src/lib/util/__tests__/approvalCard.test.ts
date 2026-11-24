// The rules of a request card that waits: when it takes the approval keys (its title, what it
// asks and its buttons in view and uncovered, then a moment of rest), when its answer may be
// sent, and when its answer field takes the focus.
import { afterEach, describe, expect, it } from 'vitest';
import { cardKeyAct, registerCardView } from '../../state/keys';
import {
  answerMissing,
  answerTakesFocus,
  envelopeAsksText,
  foldsReason,
  holdStep,
  keyAct,
  keyActOf,
  keycapActs,
  keysView,
  NO_HOLD,
  openInConversation,
  PLAN_LINES_MIN,
  probePoints,
  restsAfterAnswer,
  settled,
  SETTLE_MS,
  type Box,
  type Hold,
  type SeenPart,
} from '../approvalCard';

const box = (top: number, bottom: number, left = 217, right = 873): Box => ({ top, bottom, left, right });
/** The thread's visible box at 1024x640, less its padding: [162, 494] less 28 px at each end. */
const thread = box(190, 466, 185, 905);
const free = (b: Box): SeenPart => ({ box: b, free: probePoints(b).map(() => true) });
/** A part with something over the given points (by their index in `probePoints`). */
const under = (b: Box, ...at: number[]): SeenPart => ({ box: b, free: probePoints(b).map((_, i) => !at.includes(i)) });
/** Title, what it asks, buttons: Bo's card at 1024x640 (233 px from its title to its buttons). */
const bo = (shift = 0) => [box(225 + shift, 249 + shift), box(257 + shift, 300 + shift), box(420 + shift, 458 + shift)];

describe('the card that takes the approval keys (`keysView`)', () => {
  it('takes them while its title, what it asks and its buttons are all in view and uncovered', () => {
    expect(keysView(bo().map(free), thread, 233)).toBe('ok');
    // At the edges, give or take a pixel.
    expect(keysView([free(box(189.5, 214)), free(box(222, 265)), free(box(430, 466.6))], thread, 277)).toBe('ok');
  });

  it('the review’s row 4: who asks is above the thread, the command and the buttons in view', () => {
    // At 1440x900: the thread [162, 754] less its padding; Ada's card [118, 407], its title at [132, 156].
    const tall = box(190, 726, 185, 1255);
    expect(keysView([free(box(132, 156)), free(box(200, 243)), free(box(369, 407))], tall, 275)).toBe('out');
    expect(keysView([free(box(232, 256)), free(box(300, 343)), free(box(469, 507))], tall, 275)).toBe('ok');
  });

  it('a card scrolled away, or half in view, takes nothing', () => {
    expect(keysView(bo(-300).map(free), thread, 233)).toBe('out');
    expect(keysView(bo(100).map(free), thread, 233)).toBe('out');
    // Cut at the side (a narrow window).
    expect(keysView([free(box(225, 249, 100, 600)), ...bo().slice(1).map(free)], thread, 233)).toBe('out');
  });

  it('the review’s row 19: in the sheet, something over what it asks or over its buttons', () => {
    // The sheet's view less its scroll padding at the top (48 px): [62, 626].
    const sheet = box(62, 626, 640, 1010);
    const parts = [box(70, 90, 660, 990), box(98, 160, 660, 990), box(320, 388, 660, 990)];
    expect(keysView(parts.map(free), sheet, 318)).toBe('ok');
    // Something drawn over one corner of what it asks: covered, whatever it is.
    expect(keysView([free(parts[0]), under(parts[1], 0), free(parts[2])], sheet, 318)).toBe('covered');
    // A pill over the centre of the buttons: covered.
    expect(keysView([free(parts[0]), free(parts[1]), under(parts[2], 6)], sheet, 318)).toBe('covered');
  });

  it('"in view" means readable: not under the thread’s top fade, and not scrolled out of the card’s body', () => {
    // The thread [162, 494] fades in over its top 28 px: its view starts at 190 (`viewOf` reads its padding).
    // A command at [176, 219], under the fade: no keys.
    expect(keysView([free(box(150, 174)), free(box(176, 219)), free(box(380, 418))], thread, 268)).toBe('out');
    // A question whose legend is scrolled up out of the card's body: the body clips it, and a
    // press at its points reaches the reason above the body, not the legend: no keys.
    expect(keysView([free(box(200, 224)), under(box(236, 260), 0, 1, 2, 3, 4, 5, 6), free(box(420, 458))], thread, 258)).toBe('covered');
    // Both fully in view: keys.
    expect(keysView([free(box(200, 224)), free(box(290, 314)), free(box(420, 458))], thread, 258)).toBe('ok');
  });

  it('a part that could not be tested at any point is not seen', () => {
    expect(keysView([free(box(225, 249)), { box: box(257, 300), free: [] }, free(box(420, 458))], thread, 233)).toBe('covered');
    expect(keysView([], thread, 0)).toBe('out');
  });

  it('a card that needs more room than the view never takes the keys, wherever it is', () => {
    // The review’s defect 3: the card needs 320 px, the thread (Ask-me open) has less.
    const small = box(190, 436);
    expect(keysView(bo().map(free), small, 320)).toBe('too-big');
    expect(keysView(bo().map(free), small, 248)).toBe('too-big');
    expect(keysView(bo(-30).map(free), small, 246)).toBe('ok');
  });

  it('tests the corners, the middle of the top and bottom edges and the centre, inside the part', () => {
    const pts = probePoints(box(100, 124, 200, 400));
    expect(pts).toHaveLength(7);
    for (const [x, y] of pts) {
      expect(x).toBeGreaterThan(200);
      expect(x).toBeLessThan(400);
      expect(y).toBeGreaterThan(100);
      expect(y).toBeLessThan(124);
    }
    // A part 2 px high is tested on its middle line, not outside it.
    for (const [, y] of probePoints(box(100, 102, 0, 10))) expect(y).toBe(101);
  });
});

/** Bo's card in the thread, with nothing over it. */
const look = (boxes: Box[]) => keysView(boxes.map(free), thread, 233);
/**
 * The card's hold over a run of frames (one every 16 ms, from `t0` to `t1`), with the boxes the
 * card has at each time (`at`), as the card's loop looks at them.
 */
function run(h: Hold, t0: number, t1: number, at: (t: number) => Box[]): Hold {
  for (let t = t0; t <= t1; t += 16) h = holdStep(h, t, look(at(t)) === 'ok', at(t));
  return h;
}
/** The press at `t`: one last look at the boxes, then what the key does. */
function press(h: Hold, t: number, boxes: Box[], from: number) {
  const view = look(boxes);
  const now = holdStep(h, t, view === 'ok', boxes);
  return keyAct(view, settled(now, t, from));
}

describe('the settle time before a key answers (`holdStep`, `settled`, `keyAct`)', () => {
  it('the review’s row 12: a card out of view at the first press is not answered by a second one 157, 257 or 317 ms later', () => {
    for (const gap of [157, 257, 317]) {
      // Press 1 at 0: the card is out of view, so the key brings it in (a smooth scroll of 300 ms).
      expect(press(NO_HOLD, 0, bo(-300), 0)).toBe('reveal');
      const scroll = (t: number) => bo(-300 + Math.min(300, t));
      const h = run(NO_HOLD, 16, gap - 1, scroll);
      expect(press(h, gap, scroll(gap), 0)).not.toBe('answer');
    }
  });

  it('the review’s row 14: the next card, never seen, is not answered 157, 257 or 317 ms after the first one was', () => {
    for (const gap of [157, 257, 317]) {
      // Bo was answered at 0 by the first press; Ada is the card of the keys from 20 ms, already in view and still.
      const from = 20;
      const h = run(NO_HOLD, from, gap - 1, () => bo());
      expect(press(h, gap, bo(), from)).toBe('wait');
    }
  });

  it('answers once the card has been in view, uncovered and still for 600 ms', () => {
    const h = run(NO_HOLD, 0, 640, () => bo());
    // The first still look is at 16 ms: from there, 600 ms.
    expect(press(h, 600, bo(), 0)).toBe('wait');
    expect(press(h, 616, bo(), 0)).toBe('answer');
    // …counted from when it became the card of the keys, when that is later.
    expect(press(h, 616, bo(), 100)).toBe('wait');
    expect(press(h, 700, bo(), 100)).toBe('answer');
  });

  it('a press while it settles does nothing, and does not start the wait over', () => {
    let h = run(NO_HOLD, 0, 300, () => bo());
    expect(press(h, 300, bo(), 0)).toBe('wait');
    h = run(h, 316, 640, () => bo());
    expect(press(h, 640, bo(), 0)).toBe('answer');
  });

  it('a card that moves, or leaves the view or is covered for one frame, starts the wait over', () => {
    let h = run(NO_HOLD, 0, 800, () => bo());
    expect(settled(h, 800, 0)).toBe(true);
    h = holdStep(h, 816, true, bo(3));
    expect(h.since).toBeNull();
    h = run(h, 832, 1200, () => bo(3));
    expect(settled(h, 1200, 0)).toBe(false);
    expect(settled(h, 1432, 0)).toBe(true);
    h = holdStep(h, 1448, false, bo(3));
    expect(settled(h, 1448, 0)).toBe(false);
    // A move seen only at the press: no answer.
    h = run(NO_HOLD, 0, 800, () => bo());
    expect(press(h, 816, bo(5), 0)).toBe('wait');
  });

  it('two fast presses 100, 300, 500 and 700 ms apart, the card brought into view by the first', () => {
    // The card scrolls in for 300 ms, then rests.
    const scroll = (t: number) => bo(-300 + Math.min(300, t));
    const got = [100, 300, 500, 700].map((gap) => press(run(NO_HOLD, 16, gap - 1, scroll), gap, scroll(gap), 0));
    expect(got).toEqual(['reveal', 'wait', 'wait', 'wait']);
    // Still from the frame at 320 ms: the key answers from 920 ms on.
    expect(press(run(NO_HOLD, 16, 915, scroll), 919, scroll(919), 0)).toBe('wait');
    expect(press(run(NO_HOLD, 16, 915, scroll), 920, scroll(920), 0)).toBe('answer');
  });

  it('a press less than 600 ms after a key answered does nothing: the answered card may still show (seen at 108 ms in the browser)', () => {
    expect(restsAfterAnswer(1108, 1000)).toBe(true);
    expect(restsAfterAnswer(1599, 1000)).toBe(true);
    expect(restsAfterAnswer(1600, 1000)).toBe(false);
    expect(restsAfterAnswer(5, null)).toBe(false);
  });

  it('a card that is out, covered or too big is only brought into view', () => {
    for (const v of ['out', 'covered', 'too-big'] as const) {
      expect(keyAct(v, true)).toBe('reveal');
      expect(keyAct(v, false)).toBe('reveal');
    }
    expect(SETTLE_MS).toBe(600);
  });
});

describe('the cards on screen, as the keys read them', () => {
  const off: (() => void)[] = [];
  afterEach(() => off.splice(0).forEach((f) => f()));

  it('a key does what a card that shows the request says, for its twins too', () => {
    let act: 'answer' | 'wait' | 'reveal' = 'reveal';
    off.push(registerCardView(['ap1', 'ap2'], () => act));
    expect([cardKeyAct('ap1'), cardKeyAct('ap2')]).toEqual(['reveal', 'reveal']);
    act = 'answer';
    expect([cardKeyAct('ap1'), cardKeyAct('ap2')]).toEqual(['answer', 'answer']);
    // A request with no card on screen is never answered: its key only brings it into view.
    expect(cardKeyAct('ap3')).toBe('reveal');
  });

  it('the sheet and the conversation can both show it: one card that may be answered is enough, and a card that goes is forgotten', () => {
    const sheet = registerCardView(['ap1'], () => 'wait');
    const conversation = registerCardView(['ap1'], () => 'answer');
    off.push(sheet);
    expect(cardKeyAct('ap1')).toBe('answer');
    conversation();
    expect(cardKeyAct('ap1')).toBe('wait');
    expect(keyActOf(['reveal', 'reveal'])).toBe('reveal');
    expect(keyActOf([])).toBe('reveal');
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

describe('the answer field of a question Codly cannot list (an MCP server asks through Codex)', () => {
  // The params of 'mcpServer/elicitation/request' (codex-cli 0.166.0 McpServerElicitationRequestParams; written from the generated types, not recorded).
  const base = { threadId: 't', turnId: null, serverName: 'docs', _meta: null };

  it('a form that asks for a value shows the field and waits for text', () => {
    const form = { ...base, mode: 'form', message: 'Which space?', requestedSchema: { type: 'object', properties: { space: { type: 'string' } } } };
    expect(envelopeAsksText(form)).toBe(true);
    expect(answerMissing([], '')).toBe(true);
  });

  it('a link to open, or a form that asks for nothing, is only accepted or skipped: no field, nothing to type', () => {
    const url = { ...base, mode: 'url', message: 'Sign in to docs', url: 'https://docs.example/auth', elicitationId: 'e1' };
    const confirm = { ...base, mode: 'form', message: 'Confirm?', requestedSchema: {} };
    const empty = { ...base, mode: 'form', message: 'Confirm?', requestedSchema: { type: 'object', properties: {} } };
    expect([envelopeAsksText(url), envelopeAsksText(confirm), envelopeAsksText(empty)]).toEqual([false, false, false]);
    // The card then passes no field text: "Send answer" is not held back.
    expect(answerMissing([], null)).toBe(false);
  });

  it('a request Codly cannot read asks for text', () => {
    expect(envelopeAsksText(null)).toBe(true);
    expect(envelopeAsksText({ ...base, mode: 'openaiForm', message: 'Name?', requestedSchema: 'x' })).toBe(true);
  });
});

describe('what the sheet says of requests open in the conversation', () => {
  it('a question is a question (the review’s defect 15), any other request an approval', () => {
    expect(openInConversation([true])).toBe('Its question is open in the conversation.');
    expect(openInConversation([false])).toBe('Its approval is open in the conversation.');
    expect(openInConversation([false, false])).toBe('2 approvals are open in the conversation.');
    expect(openInConversation([true, true])).toBe('2 questions are open in the conversation.');
    expect(openInConversation([true, false])).toBe('2 requests are open in the conversation.');
  });
});

describe('the keycaps a card shows, by where the focus is (`keycapActs`)', () => {
  const caps = ['⌘↵', '⌥⌘↵', '⌘⌫', '⇧⌘⌫', '1'];
  const shown = (focus: Parameters<typeof keycapActs>[1]) => caps.filter((k) => keycapActs(k, focus));

  it('on the page every key acts', () => {
    expect(shown('page')).toEqual(caps);
  });

  it('in the empty composer the approval keys act, a digit is a character', () => {
    expect(shown('composer-empty')).toEqual(['⌘↵', '⌥⌘↵', '⌘⌫', '⇧⌘⌫']);
  });

  it('in a field with text ⌘⌫ deletes text: Deny and Skip show no keycap', () => {
    // A draft in the composer: ⌘↵ sends the draft (Composer.svelte), only ⌥⌘↵ is left to the card.
    expect(shown('composer-text')).toEqual(['⌥⌘↵']);
    // Only spaces: the composer leaves ⌘↵ to the card, but ⌘⌫ still deletes a space.
    expect(shown('composer-blank')).toEqual(['⌘↵', '⌥⌘↵']);
    // Any other field (an Ask-me answer): the keys act there, the card shows none.
    expect(shown('field')).toEqual([]);
  });

  it('in the card’s own answer field only ⌘↵ acts: the field sends (the review’s nit 4: Skip showed ⌘⌫)', () => {
    expect(shown('own-field')).toEqual(['⌘↵']);
  });

  it('a key the card never shows is an error, not a hidden keycap', () => {
    expect(() => keycapActs('⌘K', 'page')).toThrow();
  });
});

describe('a plan card in a small window (`foldsReason`)', () => {
  // 1024x640: the thread is 332 px, the card may take 332 - 28 (top fade) - 20 (bottom) - 8 = 276 px.
  // Measured parts (the card's style notes): padding 28, title 24, buttons with keycaps 53, gaps 8.
  // The plan: lines of 17 px (--lh-sm) in a box with 20 px of padding.
  const plan = { room: 276, reasonLine: 20, planLine: 17, planPad: 20 };

  it('with no reason the plan keeps about eight lines: nothing to fold', () => {
    // Title, body and buttons: two gaps.
    const fixed = 28 + 24 + 53 + 16;
    expect((plan.room - fixed - plan.planPad) / plan.planLine).toBeGreaterThan(7);
    expect(foldsReason({ ...plan, fixed, reason: 0 })).toBe(false);
  });

  it('a three-line reason and the risks (92 px) would leave the plan two lines: the reason folds, and the plan gets four', () => {
    const fixed = 28 + 24 + 92 + 53 + 24;
    expect(Math.floor((plan.room - fixed - plan.planPad) / plan.planLine)).toBe(2);
    expect(foldsReason({ ...plan, fixed, reason: 60 })).toBe(true);
    // Folded to one line, the reason gives back 40 px.
    expect(Math.floor((plan.room - (fixed - 40) - plan.planPad) / plan.planLine)).toBeGreaterThanOrEqual(PLAN_LINES_MIN);
  });

  it('a reason of one line has nothing to give, and a big window has room for all', () => {
    expect(foldsReason({ ...plan, fixed: 28 + 24 + 20 + 53 + 24, reason: 20 })).toBe(false);
    expect(foldsReason({ ...plan, room: 584, fixed: 28 + 24 + 92 + 53 + 24, reason: 60 })).toBe(false);
  });
});
