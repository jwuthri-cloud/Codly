import { describe, expect, it } from 'vitest';
import { EMPTY, LEVELS, ONBOARDING_KEY, QUESTS, completed, levelOf, loadProgress, saveProgress, totalXp, type Progress, type QuestWorld } from '../onboarding';
import type { CliInfo } from '../gen/CliInfo';

const cli = (o: Partial<CliInfo>): CliInfo => ({ provider: 'claude', path: '/bin/claude', version: null, logged_in: true, problem: null, ...o }) as CliInfo;
const world = (o: Partial<QuestWorld> = {}): QuestWorld => ({ clis: [], workspace: null, chats: [], plans: [], taskMarks: {}, ...o });
const fresh = (o: Partial<Progress> = {}): Progress => ({ ...EMPTY, flags: { ...EMPTY.flags }, ...o });

describe('onboarding', () => {
  it('completes quests from real state', () => {
    expect(completed(world(), fresh())).toEqual([]);
    expect(completed(world({ clis: [cli({ path: null })] }), fresh())).toEqual([]);
    expect(completed(world({ clis: [cli({ logged_in: false })] }), fresh())).toEqual([]);
    expect(completed(world({ clis: [cli({ problem: 'broken' })] }), fresh())).toEqual([]);
    expect(completed(world({ clis: [cli({ logged_in: null })] }), fresh())).toEqual(['crew']);
    expect(completed(world(), fresh({ flags: { mode: true, vault: true } }))).toEqual(['rules', 'vault']);
    expect(completed(world({ workspace: {} }), fresh())).toEqual(['base']);
    expect(completed(world({ chats: [{ thread: 'agent:x', author: 'user' }, { thread: 'lead', author: 'a1' }] }), fresh())).toEqual([]);
    expect(completed(world({ chats: [{ thread: 'lead:abc', author: 'user' }] }), fresh())).toEqual(['order']);
    expect(completed(world({ plans: [{ status: 'running', merged_sha: null }] }), fresh())).toEqual([]);
    expect(completed(world({ plans: [{ status: 'done', merged_sha: null }] }), fresh())).toEqual(['ship']);
    expect(completed(world({ taskMarks: { t: [{ kind: 'merged', ok: false }] } }), fresh())).toEqual([]);
    expect(completed(world({ taskMarks: { t: [{ kind: 'merged', ok: true }] } }), fresh())).toEqual(['ship']);
    // done is sticky
    expect(completed(world(), fresh({ done: ['base'] }))).toEqual(['base']);
  });

  it('adds up XP with a recruit bonus and maps it to levels', () => {
    expect(totalXp([], 3)).toBe(0);
    expect(totalXp(['crew'], 1)).toBe(100);
    expect(totalXp(['crew'], 3)).toBe(150);
    expect(totalXp(['rules'], 3)).toBe(75);
    const all = QUESTS.map((q) => q.id);
    expect(levelOf(totalXp(all, 1)).name).toBe('Principal');
    expect(levelOf(0)).toMatchObject({ index: 0, name: 'Intern', next: 100, progress: 0 });
    expect(levelOf(175)).toMatchObject({ name: 'Junior', floor: 100, next: 250, progress: 0.5 });
    expect(levelOf(LEVELS[LEVELS.length - 1].xp + 999)).toMatchObject({ name: 'Principal', next: null, progress: 1 });
  });

  it('round-trips progress through storage and ignores junk', () => {
    const mem = new Map<string, string>();
    const storage = { getItem: (k: string) => mem.get(k) ?? null, setItem: (k: string, v: string) => void mem.set(k, v) };
    const p = fresh({ done: ['crew', 'vault'], flags: { mode: true, vault: false }, dismissed: true });
    saveProgress(p, storage);
    expect(loadProgress(storage)).toEqual(p);
    mem.set(ONBOARDING_KEY, JSON.stringify({ done: ['crew', 'nope'], flags: 3 }));
    expect(loadProgress(storage)).toEqual(fresh({ done: ['crew'] }));
    mem.set(ONBOARDING_KEY, '{oops');
    expect(loadProgress(storage)).toEqual(fresh());
  });
});
