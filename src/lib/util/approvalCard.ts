// The rules of a request card that waits for you, away from the component so they can be
// tested: when the card takes the approval keys, when its answer may be sent, and when its
// answer field takes the focus.

/** A box on the screen, in px (a DOMRect has these). */
export type Box = { top: number; bottom: number; left: number; right: number };

/**
 * Whether a card takes the approval keys (⌘↵, ⌘⌫, the digits) and shows its keycaps: only while
 * you can see what you answer and how. `parts`: the boxes of what it asks (its command, its
 * question or its title) and of its button row; `view`: the visible box of the scroller that
 * shows it. Every part must lie inside the view, give or take a pixel. A card with no part to
 * measure takes nothing.
 */
export function takesKeys(parts: Box[], view: Box): boolean {
  const SLACK = 1;
  return (
    parts.length > 0 &&
    parts.every((p) => p.top >= view.top - SLACK && p.bottom <= view.bottom + SLACK && p.left >= view.left - SLACK && p.right <= view.right + SLACK)
  );
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
 * (an MCP server's elicitation), null when the card lists its questions. An empty answer is
 * never sent: an agent takes it as your answer.
 */
export function answerMissing(questions: AskedQuestion[], free: string | null): boolean {
  if (!questions.length) return free !== null && !free.trim();
  return questions.some((q) => (q.options > 0 || q.own) && !q.answered);
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
