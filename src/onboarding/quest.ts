// The quest: five levels, 100 XP each. Pure so the map, the toasts and the tests agree.
import type { ByokStatus } from '../lib/gen/ByokStatus';
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

/** Whether OpenRouter is on: its key is saved, and agents get it. */
export const openRouterOn = (s: ByokStatus | null | undefined) => s?.vendors.find((v) => v.vendor === 'openrouter')?.source.kind === 'key';

/** What is true on this machine: what the levels' rewards speak about. */
export type QuestFacts = { clis: readonly CliInfo[]; openRouter: boolean };

/** What is said in place of a reward: what is missing, and where to get it. */
export type Missing = { title: string; body: string };

const HIRE = 'Install or sign in to one on level 1';

/**
 * The one rule for every level: a reward ("+100 XP · Crew hired") is for something that
 * happened. Null: the level's reward is true, so it is earned. Otherwise what is missing: the
 * quest says that, moves on (every level is optional), and gives nothing.
 * A crew, a Lead, a creative stack and the first mission all need one CLI that is ready to
 * work (only a ready CLI can lead: the first one does until you pick). More brains needs
 * OpenCode installed and an OpenRouter key that is on.
 */
export function missing(id: LevelId, facts: QuestFacts): Missing | null {
  const ready = crewReady(facts.clis);
  switch (id) {
    case 'crew':
      return ready ? null : { title: 'No crew yet', body: 'No agent CLI is installed and signed in. Install one here and sign in: the level completes by itself then.' };
    case 'lead':
      return ready ? null : { title: 'No Lead yet', body: `No agent is ready, so there is no Lead to appoint. ${HIRE}, then pick your Lead.` };
    case 'brains':
      if (!facts.clis.some((c) => c.provider === 'opencode' && !!c.path)) return { title: 'No extra models yet', body: 'OpenCode is not installed, and the other models run through it. Install it on level 1, then save an OpenRouter key here.' };
      return facts.openRouter ? null : { title: 'No extra models yet', body: 'No OpenRouter key is saved and on, so no other model is unlocked. Save one here, or later in Settings.' };
    case 'creative':
      return ready ? null : { title: 'No creative stack yet', body: `No agent is ready, so none of these models can run for you. ${HIRE}.` };
    case 'mission':
      return ready ? null : { title: 'No crew yet', body: `Your repository is open, but no agent is ready to work in it. Open this quest again from Settings: ${HIRE.toLowerCase()}.` };
  }
}

/** Mark a level done once, in map order. */
export function complete(done: readonly string[], id: LevelId): string[] {
  return done.includes(id) ? [...done] : LEVELS.map((l) => l.id).filter((x) => x === id || done.includes(x));
}
