// Composer drafts: what you typed and attached but have not sent, per office and thread. They
// live here, outside the composer, so they survive whenever it leaves the screen (another room,
// the full office) for as long as the app is open.

/** A pasted, dropped or picked file: saved in the repo (`path`) once its upload ends. */
export interface Att {
  id: number;
  name: string;
  path: string | null;
  url: string | null;
  failed: boolean;
}
export type Draft = { text: string; files: Att[] };

const drafts = new Map<string, Draft>();
let seq = 0;

/** An id for a new attachment: unique across composers, so a kept draft's files never clash with new ones. */
export const nextAttId = (): number => ++seq;

/** Where a draft lives: thread names repeat across offices, so a draft is its office's too. */
export const draftSlot = (workspaceId: string | null | undefined, thread: string): string => `${workspaceId ?? ''}|${thread}`;

export function loadDraft(slot: string): Draft {
  return drafts.get(slot) ?? { text: '', files: [] };
}

/**
 * What the composer shows when another surface hands it a text (a seed). A seed marked `keep`
 * gives way to what you are typing: the draft stays (`kept`) and the seed is dropped.
 */
export function seedDraft(draft: string, seed: { text: string; keep: boolean }): { text: string; kept: boolean } {
  const kept = seed.keep && !!draft.trim() && draft !== seed.text;
  return { text: kept ? draft : seed.text, kept };
}

/** Keep a draft; an empty one is forgotten. `files` is kept as given, so an upload still on its way lands in it. */
export function saveDraft(slot: string, text: string, files: Att[]): void {
  if (text || files.length) drafts.set(slot, { text, files });
  else drafts.delete(slot);
}
