// The rules of a request card that waits for you, away from the component so they can be
// tested: when the card takes the approval keys (what you must see, and for how long), when its
// answer may be sent, and when its answer field takes the focus.

/** A box on the screen, in px (a DOMRect has these). */
export type Box = { top: number; bottom: number; left: number; right: number };

/**
 * One part of a card that you must see before a key answers it (its title, what it asks, its
 * button row): its box, and for each point of `probePoints(box)` whether a press there reaches
 * the part itself (nothing else is drawn over it: a pinned bar, a toast, a pill, a menu, a dialog).
 */
export type SeenPart = { box: Box; free: boolean[] };

/**
 * The points of a part that are tested for what lies over it: its four corners, a few px inside,
 * the middle of its top and bottom edges, and its centre.
 */
export function probePoints(b: Box): [number, number][] {
  const inX = Math.min(4, (b.right - b.left) / 2);
  const inY = Math.min(3, (b.bottom - b.top) / 2);
  const xs = [b.left + inX, (b.left + b.right) / 2, b.right - inX];
  const ys = [b.top + inY, b.bottom - inY];
  return [...ys.flatMap((y) => xs.map((x): [number, number] => [x, y])), [(b.left + b.right) / 2, (b.top + b.bottom) / 2]];
}

/**
 * Where a card stands with the approval keys (⌘↵, ⌥⌘↵, ⌘⌫, ⇧⌘⌫, the digits):
 * - 'ok': its title, what it asks and its buttons (`parts`) are inside `view` (the box of its
 *   scroller less its padding), give or take a pixel, and nothing is drawn over any of them;
 * - 'out': a part is (partly) outside the view, or there is no part to measure;
 * - 'covered': all are in view, and something lies over one of them;
 * - 'too-big': the three need more height (`need`, with the card's body at its smallest) than the
 *   view has: the card never takes the keys, only its buttons answer it.
 */
export type KeysView = 'ok' | 'out' | 'covered' | 'too-big';
export function keysView(parts: SeenPart[], view: Box, need: number): KeysView {
  const SLACK = 1;
  if (need > view.bottom - view.top + SLACK) return 'too-big';
  if (!parts.length) return 'out';
  const inside = (p: Box) => p.top >= view.top - SLACK && p.bottom <= view.bottom + SLACK && p.left >= view.left - SLACK && p.right <= view.right + SLACK;
  if (!parts.every((p) => inside(p.box))) return 'out';
  return parts.every((p) => p.free.length > 0 && p.free.every(Boolean)) ? 'ok' : 'covered';
}

/** How long a card must be in view, uncovered and still before a key answers it, in ms. */
export const SETTLE_MS = 600;

/**
 * Since when the card has been 'ok' (`keysView`) without moving: `since`, null while it is not
 * (or has just moved); `boxes`, its parts' boxes at the last look.
 */
export type Hold = { since: number | null; boxes: Box[] | null };
export const NO_HOLD: Hold = { since: null, boxes: null };

const sameBoxes = (a: Box[], b: Box[]) =>
  a.length === b.length && a.every((x, i) => Math.abs(x.top - b[i].top) < 0.5 && Math.abs(x.bottom - b[i].bottom) < 0.5 && Math.abs(x.left - b[i].left) < 0.5 && Math.abs(x.right - b[i].right) < 0.5);

/**
 * The hold after one more look at `now` (a frame, or a key press). The time starts at the first
 * look whose boxes are those of the look before (the card does not move) while it is 'ok'; a
 * look that is not 'ok', or whose boxes moved, starts it over.
 */
export function holdStep(h: Hold, now: number, ok: boolean, boxes: Box[]): Hold {
  if (!ok) return { since: null, boxes };
  const still = h.boxes !== null && sameBoxes(h.boxes, boxes);
  return { since: still ? (h.since ?? now) : null, boxes };
}

/**
 * Whether the card is settled at `now`: held 'ok' and still for `SETTLE_MS`, counted from when
 * it began (`h.since`) or from `from` (it became the card the keys act on: the one before was
 * answered), whichever is later.
 */
export const settled = (h: Hold, now: number, from: number): boolean => h.since !== null && now - Math.max(h.since, from) >= SETTLE_MS;

/**
 * What an approval key does to a card: 'answer' it (it is 'ok' and settled), 'wait' (it is 'ok'
 * and not settled yet: the key does nothing at all, and the wait goes on), or 'reveal' (bring it
 * into view: it is out, covered or too big; the key never answers).
 */
export type KeyAct = 'answer' | 'wait' | 'reveal';

/**
 * Whether a key press at `now` does nothing because a key sent an answer at `answeredAt` (null:
 * none yet) less than `SETTLE_MS` ago: the answered card may still show until the core confirms,
 * and the card that comes next has not been seen yet.
 */
export const restsAfterAnswer = (now: number, answeredAt: number | null): boolean => answeredAt !== null && now - answeredAt < SETTLE_MS;
export function keyAct(view: KeysView, isSettled: boolean): KeyAct {
  if (view !== 'ok') return 'reveal';
  return isSettled ? 'answer' : 'wait';
}

/**
 * What a key does when several cards show one request (the sheet's and the conversation's): it
 * answers when one of them may be answered, waits when one of them settles, and else brings the
 * request into view. No card: it brings it into view.
 */
export function keyActOf(acts: KeyAct[]): KeyAct {
  if (acts.includes('answer')) return 'answer';
  return acts.includes('wait') ? 'wait' : 'reveal';
}

/** One question of a card, as far as its answer goes. */
export type AskedQuestion = {
  /** How many options it lists to pick from. */
  options: number;
  /** It also takes a typed answer of its own. */
  own: boolean;
  /** You picked an option or typed an answer. */
  answered: boolean;
};

/**
 * Whether the card's answer is still missing, so "Send answer" and ⌘↵ do nothing. Every
 * question that can be answered needs an answer: an option or your own words (either one for a
 * question with both). `free`: the text of the one answer field of a question Codly cannot list
 * (an MCP server's elicitation), null when the card lists its questions or the elicitation asks
 * for no text (`envelopeAsksText`). An empty answer is never sent: an agent takes it as your answer.
 */
export function answerMissing(questions: AskedQuestion[], free: string | null): boolean {
  if (!questions.length) return free !== null && !free.trim();
  return questions.some((q) => (q.options > 0 || q.own) && !q.answered);
}

/**
 * What an agent's sheet says of its requests that wait in the conversation: a question is a
 * question, any other request an approval. `questions`: for each request, whether it is a question.
 */
export function openInConversation(questions: boolean[]): string {
  const n = questions.length;
  const word = questions.every(Boolean) ? 'question' : questions.some(Boolean) ? 'request' : 'approval';
  return n === 1 ? `Its ${word} is open in the conversation.` : `${n} ${word}s are open in the conversation.`;
}

/**
 * Whether a question Codly cannot list (an MCP server's elicitation, in its request's envelope)
 * asks for typed text, so the card shows its answer field and waits for some. Not a link to open
 * ("url" mode), and not a form whose schema asks for nothing (`requestedSchema` with no property):
 * those are only accepted or declined. A schema Codly cannot read asks for text.
 * Shapes: codex-cli 0.166.0 `McpServerElicitationRequestParams`.
 */
export function envelopeAsksText(envelope: unknown): boolean {
  if (!envelope || typeof envelope !== 'object') return true;
  const { mode, requestedSchema } = envelope as { mode?: unknown; requestedSchema?: unknown };
  if (mode === 'url') return false;
  if (!requestedSchema || typeof requestedSchema !== 'object') return true;
  const props = (requestedSchema as { properties?: unknown }).properties;
  return !!props && typeof props === 'object' && Object.keys(props).length > 0;
}

/**
 * Whether the answer field of the card takes the focus when the card becomes the one that
 * waits: only when the answer can only be typed (one question with no option to pick, or the
 * one field of a question Codly cannot list), and never while you type somewhere else.
 */
export function answerTakesFocus(questions: Pick<AskedQuestion, 'options' | 'own'>[], freeField: boolean, typingElsewhere: boolean): boolean {
  if (typingElsewhere) return false;
  return questions.length ? questions.length === 1 && questions[0].options === 0 && questions[0].own : freeField;
}

/**
 * Where the focus is while a card shows its keycaps: on no text field ('page'), in the composer
 * ('composer-empty'; 'composer-blank': only spaces; 'composer-text'), in a text field of the card
 * itself ('own-field': its answer), or in another text field ('field').
 */
export type KeyFocus = 'page' | 'composer-empty' | 'composer-blank' | 'composer-text' | 'own-field' | 'field';

/**
 * Whether the key of a keycap (`kbd`: '⌘↵', '⌥⌘↵', '⌘⌫', '⇧⌘⌫', or a digit '1'-'9') acts with the
 * focus at `focus`, so the card shows it: a keycap never shows a key that would do something else.
 * As state/keys.ts, the composer and the card's answer field take them: ⌘↵ acts but in another
 * text field and in a composer with a draft (Composer.svelte sends the draft; with only spaces it
 * leaves the key to the card; in the card's answer field the field sends); ⌥⌘↵ only on the page
 * and in the composer; the deny keys are text keys in a field with any text, spaces too, and in the
 * card's field (`denyKeyInField`), and act only on the page and in the empty composer; a digit is a
 * character in every text field.
 */
export function keycapActs(kbd: string, focus: KeyFocus): boolean {
  if (/^[1-9]$/.test(kbd)) return focus === 'page';
  if (kbd === '⌘↵') return focus !== 'field' && focus !== 'composer-text';
  if (kbd === '⌥⌘↵') return focus === 'page' || focus.startsWith('composer-');
  if (kbd === '⌘⌫' || kbd === '⇧⌘⌫') return focus === 'page' || focus === 'composer-empty';
  throw new Error(`no approval key ${kbd}`);
}

/** The fewest lines of its plan a plan card shows (1024x640, a 332 px thread, has room for them). */
export const PLAN_LINES_MIN = 3;

/** What a plan card's fit needs to know, in px. */
export type PlanFit = {
  /** The height the card may take: the part of the thread that shows it, less a small margin. */
  room: number;
  /** All of the card but its body (padding, title, the agent's reason whole, risks, buttons,
   *  gaps). */
  fixed: number;
  /** The reason's whole height, and the height of one line of it. */
  reason: number;
  reasonLine: number;
  /** One line of the plan, and the padding of its box. */
  planLine: number;
  planPad: number;
};

/**
 * Whether a plan card folds the agent's reason to one line (its "more" opens it again), so that
 * its plan keeps `PLAN_LINES_MIN` lines in view: only when the room the other parts leave is
 * less than that, and the reason has more than one line to give.
 */
export function foldsReason(f: PlanFit): boolean {
  if (f.reason <= f.reasonLine + 1) return false;
  return f.room - f.fixed - f.planPad < PLAN_LINES_MIN * f.planLine;
}
