import { describe, expect, it } from 'vitest';
import type { CliInfo } from '../../lib/gen/CliInfo';
import { catalog } from '../catalog';
import { LEVELS, XP_PER_LEVEL, complete, crewReady, nextLevel, progress, rank, xp } from '../quest';

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
