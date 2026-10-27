// First-run quest line: quests complete from real state (ready CLIs, a workspace, a Lead
// message, shipped work) plus two choices only onboarding records (an approval mode, macOS
// folder access). Pure functions; the reactive wrapper lives in src/onboarding/quest.svelte.ts.

import type { ApprovalMode } from './gen/ApprovalMode';
import type { CliInfo } from './gen/CliInfo';

export type QuestId = 'crew' | 'rules' | 'vault' | 'base' | 'order' | 'ship';

/** What onboarding remembers (localStorage, one key). `done` is sticky: XP is never taken back. */
export type Progress = {
  done: QuestId[];
  flags: { mode: boolean; vault: boolean };
  dismissed: boolean;
};

/** The slice of the store the quests read. */
export type QuestWorld = {
  clis: CliInfo[];
  workspace: unknown;
  chats: { thread: string; author: string }[];
  plans: { status: string; merged_sha: string | null }[];
  taskMarks: Record<string, { kind: string; ok?: boolean }[]>;
};

export type Quest = { id: QuestId; title: string; flavor: string; xp: number; done: (w: QuestWorld, p: Progress) => boolean };

/** A CLI that can take work: the same rule as Welcome. */
export const cliReady = (c: CliInfo) => !!c.path && c.logged_in !== false && !c.problem;
const isLeadThread = (t: string) => t === 'lead' || t.startsWith('lead:');

export const QUESTS: Quest[] = [
  { id: 'crew', title: 'Recruit your crew', flavor: 'Every agent CLI signed in on this Mac joins the team. More recruits, more XP.', xp: 100, done: (w) => w.clis.some(cliReady) },
  { id: 'rules', title: 'Set the rules', flavor: 'Pick a difficulty: how often agents stop to ask you.', xp: 75, done: (_w, p) => p.flags.mode },
  { id: 'vault', title: 'Unlock the vault', flavor: 'Let Codly into your project folders so macOS stops asking mid-task.', xp: 50, done: (_w, p) => p.flags.vault },
  { id: 'base', title: 'Open your base', flavor: 'Choose a repository. Agents work in their own worktrees, never on your branch.', xp: 100, done: (w) => !!w.workspace },
  { id: 'order', title: 'Give the Lead an order', flavor: 'Ask the Lead for something. It reads the repo and drafts a plan.', xp: 125, done: (w) => w.chats.some((m) => m.author === 'user' && isLeadThread(m.thread)) },
  {
    id: 'ship',
    title: 'Ship it',
    flavor: 'Finish a plan or merge a task. Your crew’s first win.',
    xp: 200,
    done: (w) => w.plans.some((p) => p.status === 'done' || !!p.merged_sha) || Object.values(w.taskMarks).some((ms) => ms.some((m) => m.kind === 'merged' && m.ok !== false)),
  },
];

/** Bonus XP for each ready CLI past the first. */
export const RECRUIT_BONUS = 25;

export const LEVELS = [
  { name: 'Intern', xp: 0 },
  { name: 'Junior', xp: 100 },
  { name: 'Senior', xp: 250 },
  { name: 'Staff', xp: 400 },
  { name: 'Principal', xp: 650 },
];

/** Quest ids done right now (live state) or earlier (sticky). */
export function completed(w: QuestWorld, p: Progress): QuestId[] {
  return QUESTS.filter((q) => p.done.includes(q.id) || q.done(w, p)).map((q) => q.id);
}

export function totalXp(done: QuestId[], readyClis: number): number {
  const base = QUESTS.filter((q) => done.includes(q.id)).reduce((s, q) => s + q.xp, 0);
  return base + (done.includes('crew') ? Math.max(0, readyClis - 1) * RECRUIT_BONUS : 0);
}

/** The level for `xp`, and how far it is to the next one (`next` null at the top). */
export function levelOf(xp: number): { index: number; name: string; floor: number; next: number | null; progress: number } {
  let i = 0;
  while (i + 1 < LEVELS.length && xp >= LEVELS[i + 1].xp) i++;
  const floor = LEVELS[i].xp;
  const next = LEVELS[i + 1]?.xp ?? null;
  return { index: i, name: LEVELS[i].name, floor, next, progress: next == null ? 1 : (xp - floor) / (next - floor) };
}

export const ONBOARDING_KEY = 'codly.onboarding.v1';
export const EMPTY: Progress = { done: [], flags: { mode: false, vault: false }, dismissed: false };

export function loadProgress(storage: Pick<Storage, 'getItem'> | undefined = globalThis.localStorage): Progress {
  try {
    const p = JSON.parse(storage?.getItem(ONBOARDING_KEY) ?? '{}') as Partial<Progress>;
    const ids = QUESTS.map((q) => q.id);
    return {
      done: Array.isArray(p.done) ? p.done.filter((d): d is QuestId => ids.includes(d)) : [],
      flags: { mode: !!p.flags?.mode, vault: !!p.flags?.vault },
      dismissed: !!p.dismissed,
    };
  } catch {
    return { ...EMPTY, flags: { ...EMPTY.flags } };
  }
}

export function saveProgress(p: Progress, storage: Pick<Storage, 'setItem'> | undefined = globalThis.localStorage) {
  try {
    storage?.setItem(ONBOARDING_KEY, JSON.stringify(p));
  } catch {
    /* private mode */
  }
}

/** The approval modes as onboarding "difficulties", easiest-to-trust first. */
export const DIFFICULTIES: { id: ApprovalMode; title: string; tag: string; body: string }[] = [
  { id: 'strict', title: 'Strict', tag: 'Hard mode', body: 'Every command and edit the CLI would ask about comes to you.' },
  { id: 'balanced', title: 'Balanced', tag: 'Recommended', body: 'Codly settles the obvious cases; anything risky (rm -rf, git push, outside the worktree, network installs) asks you.' },
  { id: 'autonomous', title: 'Autonomous', tag: 'Fewer stops', body: 'Anything inside the agent’s worktree is allowed. Risky commands still ask.' },
  { id: 'full', title: 'Full access', tag: 'No stops', body: 'Workers never wait on you for commands or edits, risky ones included. A few hard floors still ask.' },
];

/** What Full access lets through, and what still asks: shown before it can be turned on. */
export const FULL_ACCESS = {
  allows: [
    'Any command or edit a worker asks about, including risky ones: rm -rf, git push, network installs, files outside its worktree.',
    'Approved once per request, automatically: nothing waits for you.',
  ],
  stillAsks: [
    'Questions an agent asks you',
    'Changes to an agent’s permissions',
    'Anything touching Codly’s private data',
    'Read-only agents: the Lead and reviewers',
  ],
};

export const FOLDER_ACCESS_URL = 'x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders';
