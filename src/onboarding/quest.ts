// The quest: five levels, 100 XP each. Pure so the map, the toasts and the tests agree.
import type { CliInfo } from '../lib/gen/CliInfo';
import { cliOk } from '../lib/util/cli';

export type LevelId = 'crew' | 'lead' | 'brains' | 'creative' | 'mission';
export type Level = { id: LevelId; title: string; blurb: string; icon: string; reward: string };

export const XP_PER_LEVEL = 100;

export const LEVELS: Level[] = [
  { id: 'crew', title: 'Hire your crew', blurb: 'Install and sign in to the agent CLIs you want on your team.', icon: 'users', reward: 'Crew hired' },
  { id: 'lead', title: 'Pick your Lead', blurb: 'The supervisor you talk to. It plans; the crew builds.', icon: 'user', reward: 'Lead appointed' },
  { id: 'brains', title: 'More brains', blurb: 'Any other model, through OpenCode and one OpenRouter key.', icon: 'sparkle', reward: 'Brains unlocked' },
  { id: 'creative', title: 'Level up', blurb: 'Which model to use for images, voice and video.', icon: 'image', reward: 'Creative stack ready' },
  { id: 'mission', title: 'First mission', blurb: 'Open a repository and send your first request.', icon: 'flag', reward: 'Crew assembled' },
];

export const levelIndex = (id: LevelId) => LEVELS.findIndex((l) => l.id === id);
export const xp = (done: readonly string[]) => LEVELS.filter((l) => done.includes(l.id)).length * XP_PER_LEVEL;
export const progress = (done: readonly string[]) => LEVELS.filter((l) => done.includes(l.id)).length / LEVELS.length;

/** The first level you have not finished; the last one once everything is done. */
export function nextLevel(done: readonly string[]): LevelId {
  return (LEVELS.find((l) => !done.includes(l.id)) ?? LEVELS[LEVELS.length - 1]).id;
}

const RANKS: [number, string][] = [
  [500, 'Studio head'],
  [300, 'Crew chief'],
  [100, 'Hire'],
  [0, 'Rookie'],
];
export const rank = (points: number) => RANKS.find(([min]) => points >= min)![1];

/** Level 1 completes by itself as soon as one CLI is ready to work. */
export const crewReady = (clis: readonly CliInfo[]) => clis.some(cliOk);

/** Mark a level done once, in map order. */
export function complete(done: readonly string[], id: LevelId): string[] {
  return done.includes(id) ? [...done] : LEVELS.map((l) => l.id).filter((x) => x === id || done.includes(x));
}
