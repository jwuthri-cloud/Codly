// The shortcuts sheet (?) lists every key the global handler knows: each key that
// `installKeys` compares a press with, and each G sequence. Read from the handler's source, so a
// key added there and not here fails this test.
import { describe, expect, it } from 'vitest';
import { SHORTCUTS } from '../state/keys';
import { ROOMS } from '../state/ui.svelte';
import { bracketed } from './support/source';

const keysTs = Object.values(import.meta.glob<string>('/src/lib/state/keys.ts', { query: '?raw', import: 'default', eager: true }))[0];
/** The body of the key handler (`onKey` in `installKeys`). */
const handler = bracketed(keysTs, keysTs.indexOf('{', keysTs.indexOf('const onKey = (e: KeyboardEvent) =>')), `"'`);
const sheet = SHORTCUTS.flatMap((g) => g.items.map(([k, label]) => `${k} ${label}`)).join('\n');

/** How the sheet writes a key that `e.key` names. */
const SYMBOL: Record<string, string> = { Enter: '↵', Backspace: '⌫', Escape: 'Esc', ArrowDown: '↓', ArrowUp: '↑' };

describe('the shortcuts sheet', () => {
  it('names every key the handler compares a press with', () => {
    const letters = [...handler.matchAll(/\bk === '(.)'/g)].map((m) => m[1].toUpperCase());
    const named = [...handler.matchAll(/e\.key === '([^']+)'/g)].map((m) => SYMBOL[m[1]] ?? m[1]);
    expect(letters.length).toBeGreaterThan(5);
    const missing = [...new Set([...letters, ...named])].filter((key) => !sheet.includes(key));
    expect(missing).toEqual([]);
  });

  it('lists every G sequence: each room and Settings', () => {
    const map = bracketed(keysTs, keysTs.indexOf('const G_MAP'), `"'`);
    const extra = [...map.matchAll(/\['(\w)', '(\w+)'\]/g)].map((m) => m[1].toUpperCase());
    expect(extra).toEqual(['S']);
    const go = SHORTCUTS.find((g) => g.group === 'Go to')!.items.map(([k]) => k);
    expect(go).toEqual([...ROOMS.map((r) => `G ${r.key}`), ...extra.map((k) => `G ${k}`)]);
  });

  it('says that ↓ ↑ move like J K and that ↵ opens the selected row', () => {
    expect(SHORTCUTS.flatMap((g) => g.items)).toEqual(expect.arrayContaining([['J K', 'Next · previous (also ↓ ↑)'], ['↵', 'Open the selected row (a list)']]));
  });
});
