// May the composer send what it holds, and if not, why: the send button's state and its hint.
// ↵ follows the same rule: while sending is off it keeps the text and does nothing else.

export type SendState = { ok: true } | { ok: false; why: string | null };

/**
 * `inTerminal`: the name of the agent of this chat while it is in your terminal (Codly sends
 * it nothing until you give it back), else null. `unknown`: the first word is a slash command
 * Codly does not have (`unknownSlash`). `why` null: there is nothing to send, which needs no word.
 */
export function sendState(s: { text: string; files: number; saving: boolean; unknown: boolean; inTerminal: string | null }): SendState {
  if (s.inTerminal) return { ok: false, why: `${s.inTerminal} is in your terminal: give it back to send` };
  if (s.unknown) return { ok: false, why: 'No such command: change the text to send it' };
  if (s.saving) return { ok: false, why: 'Wait until the files are attached' };
  return s.text.trim() || s.files > 0 ? { ok: true } : { ok: false, why: null };
}
