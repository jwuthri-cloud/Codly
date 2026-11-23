import { describe, expect, it } from 'vitest';
import type { ByokStatus } from '../../lib/gen/ByokStatus';
import type { CliInfo } from '../../lib/gen/CliInfo';
import { catalog } from '../catalog';
import { LEVELS, XP_PER_LEVEL, complete, crewReady, missing, nextLevel, openRouterOn, progress, rank, xp } from '../quest';

const cli = (over: Partial<CliInfo>): CliInfo => ({ provider: 'claude', path: null, version: null, logged_in: null, billing: 'unknown', plan: null, problem: null, transport: 'stream-json', ...over }) as CliInfo;

describe('quest', () => {
  it('starts at the crew and walks the map in order', () => {
    expect(nextLevel([])).toBe('crew');
    expect(nextLevel(['crew'])).toBe('lead');
    expect(nextLevel(['crew', 'brains'])).toBe('lead');
    expect(nextLevel(LEVELS.map((l) => l.id))).toBe('mission');
  });
  it('counts XP and progress only for known levels', () => {
    expect(xp(['crew', 'nonsense'])).toBe(XP_PER_LEVEL);
    expect(progress(LEVELS.map((l) => l.id))).toBe(1);
    expect(xp(LEVELS.map((l) => l.id))).toBe(LEVELS.length * XP_PER_LEVEL);
  });
  it('completes a level once, in map order', () => {
    expect(complete(['lead'], 'crew')).toEqual(['crew', 'lead']);
    expect(complete(['crew'], 'crew')).toEqual(['crew']);
  });
  it('ranks by XP', () => {
    expect(rank(0)).toBe('Rookie');
    expect(rank(100)).toBe('Hire');
    expect(rank(500)).toBe('Studio head');
  });
  it('a crew is ready as soon as one CLI is installed and signed in', () => {
    expect(crewReady([cli({}), cli({ provider: 'codex', path: '/bin/codex', logged_in: false })])).toBe(false);
    expect(crewReady([cli({}), cli({ provider: 'codex', path: '/bin/codex', logged_in: true })])).toBe(true);
  });
});

describe('a reward is for something that happened', () => {
  const none: CliInfo[] = [];
  const notReady = [cli({}), cli({ provider: 'codex', path: '/bin/codex', logged_in: false })];
  const claude = [cli({ path: '/bin/claude', logged_in: true })];
  const withOpenCode = [...claude, cli({ provider: 'opencode', path: '/bin/opencode' })];
  const ids = LEVELS.map((l) => l.id);

  it('with no agent ready, no level gives its reward: each says what is missing', () => {
    for (const clis of [none, notReady])
      for (const id of ids) {
        const gap = missing(id, { clis, openRouter: true });
        expect(gap, id).not.toBeNull();
        // It names what is missing, never the reward.
        expect(`${gap!.title} ${gap!.body}`).not.toContain(LEVELS.find((l) => l.id === id)!.reward);
      }
    expect(missing('lead', { clis: none, openRouter: false })).toEqual({ title: 'No Lead yet', body: 'No agent is ready, so there is no Lead to appoint. Install or sign in to one on level 1, then pick your Lead.' });
    expect(missing('mission', { clis: none, openRouter: false })?.title).toBe('No crew yet');
  });

  it('one ready CLI earns the crew, the Lead, the creative stack and the first mission', () => {
    for (const id of ['crew', 'lead', 'creative', 'mission'] as const) expect(missing(id, { clis: claude, openRouter: false }), id).toBeNull();
  });

  it('more brains needs OpenCode installed and an OpenRouter key that is on', () => {
    expect(missing('brains', { clis: claude, openRouter: true })?.body).toContain('OpenCode is not installed');
    expect(missing('brains', { clis: withOpenCode, openRouter: false })?.body).toContain('No OpenRouter key is saved and on');
    expect(missing('brains', { clis: withOpenCode, openRouter: true })).toBeNull();
  });

  it('OpenRouter is on when its key is the source agents get', () => {
    const status = (kind: 'key' | 'off', key_saved: boolean) => ({ vendors: [{ vendor: 'openrouter', source: { kind }, key_saved }] }) as ByokStatus;
    expect(openRouterOn(status('key', true))).toBe(true);
    expect(openRouterOn(status('off', true))).toBe(false); // saved, but switched off
    expect(openRouterOn({ vendors: [] })).toBe(false);
    expect(openRouterOn(null)).toBe(false);
  });
});

describe('catalog', () => {
  it('every pick says what it is, why, and which tier', () => {
    for (const c of catalog.capabilities) {
      expect(c.picks.length).toBeGreaterThan(0);
      for (const p of c.picks) {
        expect(p.name.length).toBeGreaterThan(2);
        expect(p.why.length).toBeGreaterThan(10);
        expect(['best', 'fast', 'good', 'none']).toContain(p.tier);
      }
    }
  });
  it('Claude is never offered as an image generator', () => {
    const images = catalog.capabilities.find((c) => c.id === 'images')!;
    expect(images.picks.filter((p) => p.provider === 'claude').every((p) => p.tier === 'none')).toBe(true);
  });
  it('carries no secrets or non-https links', () => {
    const text = JSON.stringify(catalog);
    expect(text).not.toMatch(/sk-[A-Za-z0-9]{8,}|gsk_[A-Za-z0-9]{8,}/);
    for (const m of text.matchAll(/https?:\/\/[^\s"]+/g)) expect(m[0].startsWith('https://')).toBe(true);
  });
  it('has a first mission to offer', () => {
    expect(catalog.missions.length).toBeGreaterThan(0);
    expect(catalog.brains.steps).toHaveLength(3);
  });
});
