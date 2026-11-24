// Composer drafts: what you typed and attached but have not sent, per office and thread. They
// live here, outside the composer, so they survive whenever it leaves the screen (another room,
// the full office). A copy is kept in the browser's storage too (`codly.draft.<office>.<thread>`),
// so a draft also survives a reload and a restart: its text, and the files whose upload ended.

/** A pasted, dropped or picked file: saved in the repo (`path`) once its upload ends. */
export interface Att {
  id: number;
  name: string;
  path: string | null;
  url: string | null;
  failed: boolean;
  /** A long text you pasted, attached as a file: it can still go into the field instead. Not
   *  kept across a restart (the file is). */
  pasted?: { text: string; lines: number };
}
export type Draft = { text: string; files: Att[] };

const drafts = new Map<string, Draft>();
/** The slots of closed requests: their thread name is never given again, so nothing is kept for them. */
const forgotten = new Set<string>();
let seq = 0;

/** Where the kept copies live in the browser's storage. */
const DRAFT_KEY = 'codly.draft.';
/** A longer text is not kept across a restart (it stays while the app is open). */
export const DRAFT_MOST_CHARS = 100_000;
/** How many drafts are kept across a restart: the oldest goes first. */
export const DRAFT_MOST = 50;

/** A draft as the storage keeps it. */
type Kept = { text: string; files: { name: string; path: string }[]; at: number };

/** An id for a new attachment: unique across composers, so a kept draft's files never clash with new ones. */
export const nextAttId = (): number => ++seq;

/** Where a draft lives: thread names repeat across offices, so a draft is its office's too. */
export const draftSlot = (workspaceId: string | null | undefined, thread: string): string => `${workspaceId ?? ''}|${thread}`;

/** The storage key of a draft; null: no office is open, the draft is not kept across a restart. */
function keyOf(slot: string): string | null {
  const bar = slot.indexOf('|');
  return bar > 0 ? `${DRAFT_KEY}${slot.slice(0, bar)}.${slot.slice(bar + 1)}` : null;
}

/** The kept copy of a draft, if the storage holds a readable one. One it cannot read is dropped. */
function readKept(key: string): Draft | null {
  try {
    const raw = localStorage.getItem(key);
    if (raw === null) return null;
    const kept = JSON.parse(raw) as Kept;
    if (typeof kept?.text !== 'string' || !Array.isArray(kept.files)) {
      localStorage.removeItem(key);
      return null;
    }
    const files = kept.files.filter((f) => typeof f?.name === 'string' && typeof f.path === 'string');
    return { text: kept.text, files: files.map((f) => ({ id: nextAttId(), name: f.name, path: f.path, url: null, failed: false })) };
  } catch {
    return null;
  }
}

/** Keep a copy of the draft in the storage, or forget it: the text and the files that are saved. */
function writeKept(key: string, text: string, files: Att[]): void {
  const saved = files.flatMap((f) => (f.path ? [{ name: f.name, path: f.path }] : []));
  try {
    // Too long to keep: no copy, so an older one never comes back in its place.
    if ((!text && !saved.length) || text.length > DRAFT_MOST_CHARS) {
      localStorage.removeItem(key);
      return;
    }
    localStorage.setItem(key, JSON.stringify({ text, files: saved, at: Date.now() } satisfies Kept));
    dropOldest();
  } catch {
    // No storage (a private window, full, or refused): the draft still lives while the app is open.
  }
}

/** More than `DRAFT_MOST` drafts kept: the oldest go. */
function dropOldest(): void {
  const kept = keptKeys(DRAFT_KEY).map((key) => ({ key, at: keptAt(key) }));
  kept.sort((a, b) => a.at - b.at);
  for (const k of kept.slice(0, Math.max(0, kept.length - DRAFT_MOST))) localStorage.removeItem(k.key);
}

/** The storage's keys that start with `prefix`. */
function keptKeys(prefix: string): string[] {
  const keys: string[] = [];
  for (let i = 0; i < localStorage.length; i++) {
    const key = localStorage.key(i);
    if (key?.startsWith(prefix)) keys.push(key);
  }
  return keys;
}

/** When the draft under `key` was last kept; 0 when it cannot be read (it goes first). */
function keptAt(key: string): number {
  try {
    return Number((JSON.parse(localStorage.getItem(key) ?? '{}') as Partial<Kept>).at) || 0;
  } catch {
    return 0;
  }
}

/** The draft of `slot`: the one of this session, else the copy kept from an earlier one. */
export function loadDraft(slot: string): Draft {
  if (forgotten.has(slot)) return { text: '', files: [] };
  const here = drafts.get(slot);
  if (here) return here;
  const key = keyOf(slot);
  const kept = key ? readKept(key) : null;
  if (!kept || (!kept.text && !kept.files.length)) return { text: '', files: [] };
  drafts.set(slot, kept);
  return kept;
}

/**
 * What the composer shows when another surface hands it a text (a seed). A seed marked `keep`
 * gives way to what you are typing: the draft stays (`kept`) and the seed is dropped. A seed
 * that `replaces` the text of an earlier seed (your review comments, sent back again after you
 * changed them) never drops what you wrote around it: where the draft still holds that text
 * the new one takes its place, and a draft that changed it gets the new text under it.
 */
export function seedDraft(draft: string, seed: { text: string; keep: boolean; replaces?: string | null }): { text: string; kept: boolean } {
  if (seed.replaces && draft.trim()) {
    const at = draft.indexOf(seed.replaces);
    return { text: at < 0 ? `${draft.trimEnd()}\n\n${seed.text}` : draft.slice(0, at) + seed.text + draft.slice(at + seed.replaces.length), kept: false };
  }
  const kept = seed.keep && !!draft.trim() && draft !== seed.text;
  return { text: kept ? draft : seed.text, kept };
}

/**
 * A draft without the text a seed put in it (your review comments, discarded since): what you
 * wrote around that text stays. Null: the draft does not hold the text as it was put there
 * (you changed it), so nothing can be taken out.
 */
export function withoutSeed(draft: string, seed: string): string | null {
  const at = draft.indexOf(seed);
  if (at < 0) return null;
  return [draft.slice(0, at).trimEnd(), draft.slice(at + seed.length).trimStart()].filter(Boolean).join('\n\n');
}

/**
 * Keep a draft; an empty one is forgotten, here and in the storage. `files` is kept as given, so
 * an upload still on its way lands in it; the storage keeps only the files already saved.
 */
export function saveDraft(slot: string, text: string, files: Att[]): void {
  if (forgotten.has(slot)) return;
  if (text || files.length) drafts.set(slot, { text, files });
  else drafts.delete(slot);
  const key = keyOf(slot);
  if (key) writeKept(key, text, files);
}

/**
 * A request was closed: its draft goes, also the copy kept across a restart, and nothing saves
 * one for it again. Its composer may still be on screen and save what its field holds as it
 * leaves that thread: that save is ignored.
 */
export function forgetDraft(slot: string): void {
  saveDraft(slot, '', []);
  forgotten.add(slot);
}

/** The threads of an office (`office`) that have a draft kept from an earlier session, with when
 *  it was last kept. Empty when the storage cannot be read. */
export function keptThreads(office: string): { thread: string; at: number }[] {
  const prefix = `${DRAFT_KEY}${office}.`;
  let keys: string[];
  try {
    keys = keptKeys(prefix);
  } catch {
    return [];
  }
  return keys.flatMap((key) => {
    const kept = readKept(key);
    return kept && (kept.text || kept.files.length) ? [{ thread: key.slice(prefix.length), at: keptAt(key) }] : [];
  });
}
