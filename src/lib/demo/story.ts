// The scripted demo session for `pnpm dev` (no backend). Two requests run side by side, each in
// its own thread with its own Lead and plan:
//   "lead"       — you ask for a dark-mode toggle; while the Lead is still planning you add a
//                  detail. A Claude Code or Codex Lead reads it after the step it is in, as the
//                  real one does; with another CLI it waits as Queued for the turn's end (Send
//                  now interrupts the Lead so it runs next). Claude Code, Codex and Gemini CLI
//                  then build it in parallel worktrees.
//   "lead:<id>"  — a little later you open a second request (sign-in lockout); "Lead 2" plans
//                  it and Codex + OpenCode build it while the first plan is still running.
// Workers ask before risky commands (approvals land in their plan's thread), Codly commits,
// verifies and merges, and each Lead reports back in its own thread. Everything is expressed as
// real EventKinds, so every panel and the office render it exactly like a live session.
// A 1:1 Claude agent you hire works in its mode (plan first, ask for edits, accept edits).
// The demo's second office, billing-api, has a Story of its own (`openBilling`): one request
// whose plan runs, and one request of a worker's that waits for you.
// `?resume=ask` starts the first office as a restart of Codly leaves it: nothing starts before
// your Resume (`seedHeld`, as orchestrator/resume.rs).
// docs/DEMO.md lists the URL parameters that start the demo in a state the script does not reach.
import type { Agent } from '../gen/Agent';
import type { Approval } from '../gen/Approval';
import type { Citation } from '../gen/Citation';
import type { HeldCheck } from '../gen/HeldCheck';
import type { ResumeOffer } from '../gen/ResumeOffer';
import type { JsonValue } from '../gen/serde_json/JsonValue';
import type { Plan } from '../gen/Plan';
import type { Provider } from '../gen/Provider';
import type { Target } from '../gen/Target';
import type { Task } from '../gen/Task';
import type { TaskStatus } from '../gen/TaskStatus';
import type { Workspace } from '../gen/Workspace';
import { describeRule, MockApprovalRules, PACKAGE_INSTALL } from './approvalRules';
import { bytesOf, contrastCsv, contrastSvg, ILLUSTRATION_PROMPT, illustrationPng, SETTINGS_API_MD, TOGGLE_PREVIEW_HTML } from './artifacts';
import { demoBackground } from './background';
import { ChecksDemo } from './checks';
import { DEMO_MCP, MODE_LABEL } from './controls';
import { BILLING } from './office';
import { BILLING_FILES, diffOf, INDEX, INDEX_AFTER, NEXT_BUMP, TASK_FILES, type RepoFile } from './repo';
import { saidWith } from './said';
import { Actor, Cancelled, HOME, REPO, Sim, TurnFailed, WORKTREES, sha, uid, type ToolSpec } from './sim';
import type { AskKind } from './states';
import { demoWorkspaceId } from './workspaceId';
import { waitingOn } from '../util/plan';
import { memberTitle } from '../util/team';

export const REQUEST =
  'Add a dark mode toggle to Settings. Persist the choice per user, follow the system by default, and cover it with tests.';
/** Sent while the Lead is still planning: a Claude Code or Codex Lead reads it after its current step. */
export const AMEND = 'Also make the toggle fully keyboard-accessible: arrow keys should move between the options.';
/** The second request, opened in parallel (⌘N) a little later. */
export const REQUEST_B = 'Lock sign-in for 15 minutes after 5 failed attempts, and tell people clearly when they’re locked out.';
/** The second request's thread: the same on every page load, so a reload can open on it (`?requests=two`). */
export const REQUEST_B_THREAD = 'lead:lockout';
/** `?cards=3`: the request whose plan has four tasks that wait on nothing; three of its workers ask you within seconds of each other. */
export const REQUEST_CARDS = 'Get the repo ready for the release: lint staged files before each commit, document the environment variables, update the licence year, and add the release to the changelog.';
/** The one request of the demo's second office, billing-api: asked before you opened it. */
export const REQUEST_BILLING = 'Try a failed invoice payment again, once a day and three times at most. After the last try, email the customer.';

const HOUR = 3_600_000;
const cite = (id: string, path: string, s: number, e: number, symbol: string | null, why: string[], score: number): Citation => ({
  id, path, start_line: s, end_line: e, symbol, why, score,
});

const WORKER_NAMES: Record<Provider, string> = { claude: 'Eli', codex: 'Fay', gemini: 'Gus', opencode: 'Dee', cursor: 'Hal', copilot: 'Ivy' };
const WORKER_MODELS: Record<Provider, string | null> = {
  claude: 'claude-sonnet-5-5', codex: 'gpt-6.1-sol', gemini: 'gemini-3.8-flash', opencode: 'claude-haiku-5-5', cursor: null, copilot: null,
};
const PROVIDER_NAME: Record<Provider, string> = { claude: 'Claude Code', codex: 'Codex', gemini: 'Gemini CLI', opencode: 'OpenCode', cursor: 'Cursor', copilot: 'Copilot' };

/** The plans the demo scripts: two in acme/web (and `cards` there with `?cards=3`), and `retries` in billing-api. */
type Spec = 'theme' | 'lockout' | 'retries' | 'cards';
/** Who takes each planned task, and why (the route decision says it). */
const ROUTES: Record<Spec, Record<string, { provider: Provider; name: string; color: number; why: string; alternatives: string[] }>> = {
  theme: {
    t1: { provider: 'claude', name: 'Ada', color: 7, why: 'TypeScript UI work; planner hint claude; 2 of 2 Claude slots free', alternatives: ['Codex: assigned t2', 'Gemini CLI: kept for t3 (cross-vendor)'] },
    t2: { provider: 'codex', name: 'Bo', color: 4, why: 'API + migration; planner hint codex; Codex idle', alternatives: ['Claude Code: busy with t1', 'Gemini CLI: kept for t3'] },
    t3: { provider: 'gemini', name: 'Cy', color: 2, why: 'dependencies merged; planner hint gemini; a different vendor than t1’s author', alternatives: ['Claude Code: authored t1', 'Codex: authored t2'] },
  },
  lockout: {
    t1: { provider: 'codex', name: 'Fay', color: 0, why: 'server + Redis work; planner hint codex; 1 of 2 Codex slots free', alternatives: ['Claude Code: 2 of 2 slots busy', 'OpenCode: kept for t2'] },
    t2: { provider: 'opencode', name: 'Dee', color: 3, why: 'small UI change + e2e; planner hint opencode; OpenCode idle', alternatives: ['Gemini CLI: busy with the other request', 'Codex: authored t1'] },
  },
  retries: {
    t1: { provider: 'codex', name: 'Otto', color: 4, why: 'payment code + tests; planner hint codex; Codex idle', alternatives: ['Claude Code: kept for t2', 'Gemini CLI: Pia is at home'] },
    t2: { provider: 'claude', name: 'Rex', color: 7, why: 'a small new module; planner hint claude; Claude Code idle', alternatives: ['Codex: assigned t1', 'Gemini CLI: Pia is at home'] },
  },
  cards: {
    t1: { provider: 'claude', name: 'Max', color: 1, why: 'tooling and git hooks; planner hint claude; 2 of 2 Claude slots free', alternatives: ['Codex: kept for t2', 'Gemini CLI: kept for t4'] },
    t2: { provider: 'codex', name: 'Noa', color: 3, why: 'docs and README; planner hint codex; Codex idle', alternatives: ['Claude Code: assigned t1', 'Gemini CLI: kept for t4'] },
    t3: { provider: 'claude', name: 'Rio', color: 5, why: 'one licence line, a choice to ask about; planner hint claude; 1 of 2 Claude slots free', alternatives: ['Codex: assigned t2', 'Gemini CLI: kept for t4'] },
    t4: { provider: 'gemini', name: 'Sam', color: 2, why: 'the changelog; planner hint gemini; Gemini CLI idle', alternatives: ['Claude Code: 2 of 2 slots busy', 'Codex: assigned t2'] },
  },
};
const BRANCH: Record<Spec, string> = { theme: 'codly/dark-mode', lockout: 'codly/sign-in-lockout', retries: 'codly/payment-retries', cards: 'codly/release-checklist' };
/** Task key → the demo files it writes (the key of `filesOf`). The `cards` tasks write files whose text they decide as they work (`Story.written`). */
const FILES: Record<Spec, Record<string, string>> = { theme: { t1: 't1', t2: 't2', t3: 't3' }, lockout: { t1: 'l1', t2: 'l2' }, retries: { t1: 'b1', t2: 'b2' }, cards: {} };
/** The demo files of a task, by its file key (acme/web's, or billing-api's). */
const filesOf = (key: string): RepoFile[] | undefined => TASK_FILES[key] ?? BILLING_FILES[key];
/** A plan that is done: what its Lead offers when you write again. */
const FOLLOW_UP: Record<Spec, string> = {
  theme: 'A good next step is rendering the theme on the server so the first paint never flashes',
  lockout: 'A good next step is an admin unlock and an email to the account owner after a lockout',
  retries: 'A good next step is a page where the customer can pay an unpaid invoice with another card',
  cards: 'A good next step is a release script that runs these checks and tags the version',
};
/** The tool a CLI runs a shell command with, as its other calls in the story name it. */
const shellTool = (provider: Provider) => (provider === 'claude' ? 'Bash' : provider === 'codex' ? 'commandExecution' : 'run_shell_command');
const KEYBOARD = /keyboard|arrow|a11y|accessib/i;
/** Why a task does not start while the work of the last run waits for your Resume: the core's words (orchestrator/resume.rs `HELD`). */
export const HELD = "paused: Codly restarted. Nothing starts until you press 'Resume work' at the top of the window";
/** Who answered a request that `?autoplay=1` answered: its rows read "Approved by autoplay", not "by you". */
export const AUTOPLAY = 'autoplay';
const NO_TARGET: Target = { path: null, paths: [], command: null, query: null, url: null, server: null, tool: null };
/** Claude Code and Codex take a message into the turn that runs; the other CLIs run it as the next turn (state/queue.ts). */
const readsMidTurn = (provider: Provider) => provider === 'claude' || provider === 'codex';
const quoted = (text: string, max: number) => `“${text.slice(0, max)}${text.length > max ? '…' : ''}”`;
/** An agent's list (`plan_updated`) when it is on step `n` (from 0; `steps.length`: all done). */
const listAt = (steps: string[], n: number): [string, 'pending' | 'in_progress' | 'completed'][] =>
  steps.map((text, i) => [text, i < n ? 'completed' : i === n ? 'in_progress' : 'pending']);
/** `?turn=failed`: why the turn failed, as the CLI's adapter says it (Claude Code's words: adapters/claude.rs `explain`). */
const overloaded = (provider: Provider) => (provider === 'claude' ? "Anthropic's servers are overloaded: wait a moment, then try again" : 'The API is overloaded: try again');
/** "resolve", then "resolve-2"…: a plan can need the same kind of task more than once (merge_conflict.rs `next_key`). */
const nextKey = (tasks: Task[], stem: string) => {
  let key = stem;
  for (let n = 2; tasks.some((t) => t.key === key); n++) key = `${stem}-${n}`;
  return key;
};
/** An agent's name as it reads in a path ("Eli 2" → "eli-2"). */
const slug = (name: string) => name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'agent';
/** What the demo's build prints. */
const BUILD_OUTPUT = [
  '> acme-web@2.4.0 build', '> next build', '', '   ▲ Next.js 16.1.2', '   Creating an optimized production build …', ' ✓ Compiled successfully in 9.4s',
  '   Linting and checking validity of types …', ' ✓ Generating static pages (14/14)', '', 'Route (app)                Size  First Load JS',
  '┌ ○ /                     4.1 kB        112 kB', '├ ƒ /api/settings           0 B           0 B', '└ ○ /settings             3.2 kB        109 kB',
];
/** The plan a 1:1 Claude agent shows in "plan first" (`note`: what you asked to change when you sent it back). */
const soloPlan = (ask: string, note: string | null) =>
  [
    '# Plan', '', `You asked: ${quoted(ask, 200)}`, '', '## Steps', '1. Add a short section about it to `README.md`, the only file that changes.',
    '2. Run `npm test`.', '3. Tell you what changed.', ...(note ? ['', `Changed as you asked: ${note}`] : []), '', 'Nothing changes before you accept this plan.',
  ].join('\n');

/** The demo repo's README.md, as the agents read it. */
const README = '# Acme web\n\nNext.js 16 (app router) · Prisma · Postgres · vitest · Playwright\n\n## Scripts\n- npm run dev\n- npm test\n- npx playwright test\n';
/** Its last line: where an agent adds to it. */
const README_TAIL = '- npx playwright test';
/** The preview of a Claude `Edit` on its card: the old lines taken out, the new ones put in (adapters/claude.rs `edit_preview`). */
const editPreview = (before: string, after: string) => `${before.split('\n').map((l) => `-${l}`).join('\n')}\n${after.split('\n').map((l) => `+${l}`).join('\n')}`;

/**
 * `?merge=checks`: why a check of a demo plan fails on its merge with your branch, said as the
 * check prints it, and the line of its test file that the agent's fix changes. A plan fails with
 * the last of these checks that its finished tasks ran (`failingChecks`); a check of no entry
 * here passes on the merge (billing-api's plan has none: that office never starts with it).
 */
export const MERGE_FAILURES: Record<string, { files: string; path: string; test: string; why: string; from: string; to: string }> = {
  'npm test -- theme': {
    files: 't1', path: 'src/theme/ThemeProvider.test.tsx', test: 'ThemeProvider › follows the system by default', why: 'expected “system:dark”, got “system:light”',
    from: '  beforeEach(() => localStorage.clear());',
    to: '  beforeEach(() => {\n    localStorage.clear();\n    delete document.documentElement.dataset.theme;\n  });',
  },
  'npm test -- settings': {
    files: 't2', path: 'src/server/settings/route.test.ts', test: '/api/settings › returns defaults for a new user', why: 'expected 3 fields, got 4',
    from: "    expect(await res.json()).toEqual({ locale: 'en', emailDigest: true, theme: 'system' });",
    to: "    expect(await res.json()).toMatchObject({ locale: 'en', emailDigest: true, theme: 'system' });",
  },
  'npx playwright test settings-theme': {
    files: 't3', path: 'tests/e2e/settings-theme.spec.ts', test: 'settings · theme › dark mode persists after reload', why: 'expected “dark”, got “light”',
    from: '    await page.reload();',
    to: "    await page.waitForResponse((res) => res.url().endsWith('/api/settings'));\n    await page.reload();",
  },
  'npm test -- rate-limit': {
    files: 'l1', path: 'src/server/rate-limit.test.ts', test: 'sign-in lockout › lifts after the window', why: 'expected 0, got 1',
    from: '    vi.advanceTimersByTime(LOCKOUT.windowSec * 1000 + 1);',
    to: '    await vi.advanceTimersByTimeAsync(LOCKOUT.windowSec * 1000 + 1);',
  },
  'npx playwright test auth-lockout': {
    files: 'l2', path: 'tests/e2e/auth-lockout.spec.ts', test: 'five wrong passwords lock the account for 15 minutes', why: 'expected “Too many attempts”, got “Too many sign-in attempts”',
    from: "toContainText('Too many attempts')",
    to: "toContainText('Too many sign-in attempts')",
  },
};

/** `?cards=3`: the files the release checklist's tasks write. */
const LINT_STAGED = '{\n  "*.{ts,tsx}": ["eslint --fix", "prettier --write"]\n}\n';
const PRE_COMMIT = 'npx lint-staged\n';
const ENV_DOC = '# Environment variables\n\n| Name | What it is |\n| --- | --- |\n| `DATABASE_URL` | The Postgres database |\n| `SESSION_SECRET` | Signs the session cookie |\n| `STRIPE_TEST_KEY` | Stripe in tests (never the live key) |\n';
const README_ENV = `${README}\n## Environment\n\nEvery variable is in [docs/env.md](docs/env.md).\n`;
const licence = (years: string) => `MIT License\n\nCopyright (c) ${years} Acme Inc.\n`;
const CHANGELOG_BEFORE = '# Changelog\n\n## 2.4.0\n\n- Bumped Next.js to 16.1.\n';
const CHANGELOG_AFTER = '# Changelog\n\n## 2.5.0\n\n- Lint runs on staged files before each commit.\n- The environment variables are in docs/env.md.\n\n## 2.4.0\n\n- Bumped Next.js to 16.1.\n';

function stats(files: RepoFile[]) {
  const d = diffOf(files).split('\n');
  return {
    files: files.length,
    added: d.filter((l) => l.startsWith('+') && !l.startsWith('+++')).length,
    removed: d.filter((l) => l.startsWith('-') && !l.startsWith('---')).length,
  };
}

/** One request thread: its words, its plan, and the FIFO of turns its Lead takes. */
type Req = { thread: string; spec: Spec | null; words: string[]; planId: string | null; chain: Promise<void> };
/** A message of yours that its agent has not read yet. */
type Sent = { text: string };
/** One call a helper (sub-agent) makes, and what it says it is doing meanwhile. */
type HelperStep = Pick<ToolSpec, 'name' | 'activity' | 'target'> & { step: string };

/** The states a URL parameter starts the story in (demo/states.ts, docs/DEMO.md). */
export type StoryOptions = {
  /** The repo's required checks (Settings → Safety): the gate runs them for every task while they are on. */
  checks?: ChecksDemo;
  /** `?helper=slow`: a helper stays at work long enough to look at. */
  slowHelper?: boolean;
  /** `?mode=plan`: a 1:1 Claude agent in "plan first" shows you a plan when the repo opens. */
  planMode?: boolean;
  /** `?intake=…`: the scripted requests do not start, so the request you start from GitHub is the first. */
  yourRequestFirst?: boolean;
  /** `?approval=…`: a 1:1 agent asks that kind of request when the repo opens. */
  ask?: AskKind | null;
  /** The demo's second office, billing-api: its own crew and its one request (`openBilling`), and no scripted plan for what you ask there. */
  billing?: boolean;
  /** `?resume=ask`: the first request's plan was running when Codly last closed, and its work waits for your Resume. */
  resumeAsk?: boolean;
  /** `?background=on`: a 1:1 Claude agent left a command and a helper at work in the background when the repo opens. */
  background?: boolean;
  /** `?turn=failed`: the first turn of the first 1:1 agent you talk to fails (the API is overloaded). */
  failFirstTurn?: boolean;
  /** `?requests=two`: both scripted requests are there when the repo opens, not 1.6 and 24 seconds later. */
  twoRequests?: boolean;
  /** `?cards=3`: the one request is the release checklist (`REQUEST_CARDS`), not the scripted two. */
  threeCards?: boolean;
};

/**
 * `?approval=…`: who asks (a 1:1 agent of that CLI), and what you asked it. `plan` and `file`
 * are a 1:1 Claude agent in its mode (`soloWork`). The names are none of the plans' workers
 * (`ROUTES`). A plan's task goes to the worker started for it, else to a free worker of the
 * route's name, and never to a 1:1 agent: one you hired with a worker's name makes the plan
 * start "<name> 2" (`startPlanned`).
 */
const ASKERS: Record<Exclude<AskKind, 'plan' | 'file'>, { provider: Provider; name: string; title: string; text: string }[]> = {
  // Claude Code's shell sandbox asks for a host on behalf of a command (adapters/claude.rs `SANDBOX_NETWORK`).
  network: [{ provider: 'claude', name: 'Ned', title: 'Tools', text: 'Which version of react is the newest on npm?' }],
  permissions: [{ provider: 'codex', name: 'Kit', title: 'Release', text: 'Add the dark mode toggle to the changelog in ../acme-docs.' }],
  // Options to pick from (Claude Code), and a question with a free answer (an MCP server asks through Codex).
  'question-long': [
    { provider: 'claude', name: 'Ned', title: 'Docs', text: 'Write the release note for the dark mode toggle.' },
    { provider: 'codex', name: 'Kit', title: 'Triage', text: 'Open an issue in Linear for the flaky checkout test.' },
  ],
};
/** `?approval=network`: the host the sandbox asks for. */
const NPM_HOST = 'registry.npmjs.org';
/**
 * How the command ends when the sandbox may not reach the host: npm's error, then the block
 * Claude Code's sandbox adds to the output (its shape as in the recording `r4-sandbox-network-deny`).
 */
const NPM_BLOCKED = `npm error code E403\nnpm error 403 403 user denied - GET https://${NPM_HOST}/react\n\n<sandbox_violations>\ndeny network-outbound ${NPM_HOST}:443 (user denied)\n</sandbox_violations>`;
/** `?approval=question-long`: each option is about 120 characters, so the closed row shows a long answer. */
const LONG_QUESTION = {
  question: 'The release note for the dark mode toggle can be one note for everyone, or one for users and one for developers. Which do you want?',
  header: 'Release note',
  options: [
    {
      label: 'One note for everyone: where the toggle is, that it follows the system by default, and that the choice is saved per user',
      description: 'Short, and enough for most readers. Developers find the new theme tokens in the pull request, not in the release note.',
    },
    {
      label: 'Two notes: a short one for users about where the toggle is, and a longer one for developers about the new theme tokens',
      description: 'More to write and to keep up to date, but a developer who builds a new screen knows which tokens to use from day one.',
    },
  ],
};

export class Story {
  /** "Always allow in this repo": the rules you saved, which answer later requests as in the core. */
  readonly rules: MockApprovalRules;
  private checks: ChecksDemo;
  private reqs = new Map<string, Req>();
  /** By thread: your messages sent while the agent's turn ran, in order, until it reads them. */
  private unread = new Map<string, Sent[]>();
  private busyLeads = new Set<string>();
  private running = new Set<string>(); // agent ids inside a task track
  private userAssigned = new Set<string>(); // task ids you assigned by hand
  private agentChains = new Map<string, Promise<void>>();
  private specOfPlan = new Map<string, Spec>();
  /** task id → its file key for `filesOf` (plans reuse keys t1, t2… so files are keyed apart). */
  private fileKeys = new Map<string, string>();
  private nextAdhoc = 4;
  /** Where the office's worktrees are. */
  private worktrees: string;
  /** agent id → what a 1:1 agent does in its next turn, in place of its stock answer (`seedAsk`). */
  private scripted = new Map<string, (act: Actor) => Promise<void>>();
  /** The tasks that were in work when Codly last closed, while they wait for your Resume (null: nothing is held). */
  private held: Set<string> | null = null;
  /** `?turn=failed`: the next turn of a 1:1 agent fails, once. */
  private failNext: boolean;
  /** The tasks that go on after your Resume: their worker of the last run takes them again, at the same attempt. */
  private resumed = new Set<string>();
  /** "<plan id>:<task key>" → the worker the plan's route chose for that task: it takes the task again. */
  private routed = new Map<string, string>();
  /** task id → the files it wrote whose text it decided as it worked (a fix, the release checklist): its diff. */
  private written = new Map<string, RepoFile[]>();

  constructor(private sim: Sim, private opts: StoryOptions = {}) {
    this.rules = new MockApprovalRules(sim, this);
    this.checks = opts.checks ?? new ChecksDemo();
    this.worktrees = opts.billing ? `${HOME}/.codly/worktrees/${BILLING.name}` : WORKTREES;
    this.failNext = opts.failFirstTurn ?? false;
    sim.afterStep = (act) => this.readMidTurn(act);
    sim.turnStopped = (act) => demoBackground(sim).turnStopped(act);
    sim.runStarts = (act) => this.sessionStarted(act);
  }

  // ───────────── workspace ─────────────

  openWorkspace(path: string): Workspace {
    const sim = this.sim;
    if (sim.workspace) return sim.workspace;
    if (this.opts.billing) return this.openBilling();
    const now = sim.now();
    const isDemo = /acme/i.test(path) || path === REPO;
    const name = isDemo ? 'acme/web' : (path.replace(/\/+$/, '').split('/').pop() ?? 'repo');
    sim.workspace = {
      // The demo repo keeps its id from one page load to the next, as a real office does.
      id: isDemo ? demoWorkspaceId() : uid('ws-'), repo_path: isDemo ? REPO : path, name, default_branch: 'main', current_branch: 'main',
      head: '4e1c9a2f7b0d3e5a8c6b1f2e9d0a7c3b5e4f6a1d', dirty: false, created_at: now - 40 * 86_400_000,
    };
    sim.recent = [{ repo_path: sim.workspace.repo_path, name, opened_at: now }, ...sim.recent.filter((r) => r.repo_path !== sim.workspace!.repo_path)];

    // The repo's existing board, memory and knowledge (silent: they arrive in the snapshot).
    const mk = (key: string, title: string, goal: string, status: TaskStatus, extra: Partial<Task>, ageH: number): Task => ({
      id: uid('t-'), plan_id: null, key, title, goal, acceptance: [], write_scope: [], verify: [], depends_on: [], status,
      assignee: null, agent_hint: null, account_hint: null, consultant: null, fresh_session: false, branch: null, worktree: null, attempt: 0, status_reason: null, summary: null,
      created_at: now - ageH * HOUR, updated_at: now - ageH * HOUR * 0.5, ...extra,
    });
    const a1 = mk('a1', 'Upgrade ESLint to v10', 'Move to ESLint 10 and fail CI on lint warnings.', 'pending', {
      acceptance: ['`npm run lint` passes with `--max-warnings 0`'], write_scope: ['package.json', 'eslint.config.js'], verify: ['npm run lint'],
    }, 30);
    const a2 = mk('a2', 'Fix flaky checkout e2e test', 'Make tests/e2e/checkout.spec.ts deterministic.', 'blocked', {
      status_reason: 'Needs STRIPE_TEST_KEY in .env.test — agents can’t read your keychain. Add it, then move this card to Ready.',
      write_scope: ['tests/e2e/checkout.spec.ts'], verify: ['npx playwright test checkout'], attempt: 1,
    }, 20);
    const a3 = mk('a3', 'Bump Next.js to 16.1', 'Upgrade next and eslint-config-next to 16.1.', 'done', {
      summary: 'Bumped next to 16.1.2; replaced `next lint` with `eslint .`. Build and tests green.', attempt: 1, verify: ['npm run build', 'npm test'],
    }, 52);
    for (const t of [a1, a2, a3]) sim.tasks.set(t.id, t);
    this.syncTeam();

    sim.memory('Prefer small, reviewable changes: one concern per task.', 'global', 'preference', 'user', { pinned: true, importance: 0.9, emit: false });
    sim.memory('Unit tests run with `npm test` (vitest); e2e with `npx playwright test`.', 'project', 'fact', 'codly', { importance: 0.8, emit: false });
    sim.memory('API routes live in `src/server/<area>/route.ts` and validate input with zod.', 'project', 'fact', 'codly', { importance: 0.7, emit: false });
    sim.memory('Nothing lands on `main` without a human merge.', 'project', 'decision', 'user', { pinned: true, importance: 0.9, emit: false });
    sim.memory('Next 16.1: `next lint` is gone — lint runs through `eslint .`.', 'project', 'lesson', 'codly', { scopeId: a3.id, importance: 0.5, emit: false });

    sim.triple('file:src/app/layout.tsx', 'imports', 'file:src/styles/tokens.css', 'observed', { emit: false });
    sim.triple('file:src/app/settings/page.tsx', 'renders', 'file:src/app/settings/LocaleSelect.tsx', 'observed', { emit: false });
    sim.triple('file:src/server/settings/route.ts', 'validates_with', 'concept:zod', 'agent', { emit: false });
    sim.triple('file:src/server/settings/route.test.ts', 'tests', 'file:src/server/settings/route.ts', 'observed', { emit: false });
    sim.triple(`task:${a3.id}`, 'modified', 'file:package.json', 'observed', { emit: false });
    sim.triple('concept:checkout e2e', 'flaky_because', 'concept:Stripe test key', 'agent', { emit: false, confidence: 0.6 });

    const ask = this.opts.ask;
    if (this.opts.planMode || ask === 'plan') this.seedPlanner();
    if (ask && ask !== 'plan') this.seedAsk(ask);
    if (this.opts.background) this.seedBackground();
    if (this.opts.resumeAsk) this.seedHeld();
    if (this.opts.yourRequestFirst) return sim.workspace;
    if (this.opts.threeCards) {
      void sim.sleep(1600).then(() => this.userChat('lead', REQUEST_CARDS));
      return sim.workspace;
    }
    // The demo's "you": ask for the feature, add a detail while the Lead is still planning (it
    // reads it after its current step, or it queues), then open a second request in parallel.
    const first = () => {
      if (!sim.chats.some((c) => c.thread === 'lead' && c.author === 'user')) this.userChat('lead', REQUEST);
    };
    const second = () => {
      if (!sim.chats.some((c) => c.thread.startsWith('lead:') && c.author === 'user')) this.userChat(REQUEST_B_THREAD, REQUEST_B);
    };
    // `?requests=two`: both are in the first snapshot, where the UI looks for the request you left.
    if (this.opts.twoRequests) {
      first();
      second();
    }
    void (async () => {
      await sim.sleep(1600);
      first();
      await sim.sleep(5400);
      const lead = this.leadActor('lead');
      if (lead?.inTurn && !this.req('lead').planId && !sim.chats.some((c) => c.thread === 'lead' && c.text === AMEND)) this.userChat('lead', AMEND);
      await sim.sleep(17_000);
      second();
    })();
    return sim.workspace;
  }

  /**
   * The demo's second office as you find it when you open it: the crew of three its tab
   * counts, one request whose plan runs (Otto and Rex each on a task), and the one request
   * that waits for you (Otto's). From there it plays on like the first office.
   */
  private openBilling(): Workspace {
    const sim = this.sim;
    const now = sim.now();
    sim.workspace = {
      id: BILLING.id, repo_path: BILLING.repo_path, name: BILLING.name, default_branch: 'main', current_branch: 'main',
      head: '9b2f4c1d7e0a3f5b8c6d1e2f9a0b7c3d5e4f6a1b', dirty: false, created_at: now - 12 * 86_400_000,
    };
    sim.recent = [{ repo_path: BILLING.repo_path, name: BILLING.name, opened_at: now }, ...sim.recent.filter((r) => r.repo_path !== BILLING.repo_path)];
    this.syncTeam();
    const r = this.req('lead');
    r.spec = 'retries';
    r.words.push(REQUEST_BILLING);
    sim.chat('lead', 'user', REQUEST_BILLING);
    const lead = this.ensureLead('lead');
    const plan = this.submitRetries(lead, r);
    this.postPlanReady(r, plan, 2);
    // Pia has no task in this plan: she is at home for seven more minutes (on the wall clock, as the office counts it).
    const pia = sim.newAgent({ name: 'Pia', provider: 'gemini', model: WORKER_MODELS.gemini, role: 'worker', color: 1, billing: 'subscription', status: 'idle' });
    sim.putAgent({ ...pia, run_id: null, away_since: Date.now() - 3 * 60_000, away_until: Date.now() + 7 * 60_000 });
    // Both tasks are in flight when you look: started here, not a moment later by `coordinate`.
    this.runPlan(plan.id);
    this.startPlanned(plan.id, 't1');
    this.startPlanned(plan.id, 't2');
    return sim.workspace;
  }

  private submitRetries(lead: Actor, r: Req): Plan {
    const sim = this.sim;
    const plan = this.newPlan(r, 'retries', {
      summary: 'Retries for failed invoice payments',
      assumptions: ['A payment is tried again once a day, three times at most', 'The email goes out only after the last try failed'],
      open_questions: [],
    });
    const t1 = this.newTask(plan, 'retries', 't1', 'Retry schedule for failed payments', 'Add `src/invoices/retry.ts`: a failed payment is tried again one day later, three times at most, then the invoice is `unpaid`.', {
      acceptance: ['A failed payment has its next try one day later', 'After the third retry the invoice is `unpaid`', 'Unit tests cover each retry and the last one'],
      write_scope: ['src/invoices/**'], verify: ['npm test -- retry'], agent_hint: 'codex',
    });
    const t2 = this.newTask(plan, 'retries', 't2', 'Email after the last failed try', 'Add `src/mail/paymentFailed.ts`: the email a customer gets when the last try failed, with the invoice number and the amount.', {
      acceptance: ['The email names the invoice and the amount', 'A unit test covers the text'],
      write_scope: ['src/mail/**'], verify: ['npm test -- mail'], agent_hint: 'claude',
    });
    sim.decision('plan_validated', 'rule', 'Plan is a valid DAG: 2 tasks, no dependencies, no cycles', { plan_id: plan.id, tasks: 2, edges: 0 }, { agent: lead.id });
    sim.decision('scope_check', 'rule', 'Write scopes are disjoint, so t1 and t2 can run in parallel', { plan_id: plan.id, t1: t1.write_scope, t2: t2.write_scope }, { agent: lead.id });
    return sim.patchPlan(plan.id, { status: 'awaiting_approval' });
  }

  // ───────────── chat ─────────────

  private req(thread: string): Req {
    let r = this.reqs.get(thread);
    if (!r) this.reqs.set(thread, (r = { thread, spec: null, words: [], planId: null, chain: Promise.resolve() }));
    return r;
  }

  /** `attachment`: what the core keeps with your message (the issue or pull request a request started from). */
  userChat(thread: string, text: string, attachment: JsonValue | null = null) {
    this.sim.chat(thread, 'user', text, attachment);
    // It waits for its agent: read during the turn that runs, or answered by a turn of its own.
    const sent: Sent = { text };
    this.unread.set(thread, [...(this.unread.get(thread) ?? []), sent]);
    if (thread === 'lead' || thread.startsWith('lead:')) {
      void this.withLead(thread, async (lead) => {
        if (this.take(thread, sent)) await this.leadRespond(lead, this.req(thread), text);
      });
    } else void this.agentRespond(thread.replace(/^agent:/, ''), sent);
  }

  /** The message leaves the ones that wait. False: the agent read it during a turn already. */
  private take(thread: string, sent: Sent): boolean {
    const waiting = this.unread.get(thread) ?? [];
    if (!waiting.includes(sent)) return false;
    const rest = waiting.filter((m) => m !== sent);
    if (rest.length) this.unread.set(thread, rest);
    else this.unread.delete(thread);
    return true;
  }

  /**
   * After each step of a running turn. Like Claude Code and Codex: the agent reads the messages
   * you sent meanwhile. They go into the turn (`prompt_steered`) and get no turn of their own;
   * a message sent after the turn's last step still runs as the next turn.
   */
  private async readMidTurn(act: Actor) {
    const a = act.a;
    if (!readsMidTurn(a.provider)) return;
    const thread = a.role === 'lead' ? (a.thread ?? 'lead') : `agent:${a.id}`;
    const texts = (this.unread.get(thread) ?? []).map((m) => m.text);
    if (!texts.length) return;
    this.unread.delete(thread);
    for (const input of texts) act.emit({ type: 'prompt_steered', input });
    // The Lead plans with everything you said; a worker says that it read you, and goes on.
    if (a.role === 'lead') this.req(thread).words.push(...texts);
    else await act.say(`Noted: ${quoted(texts[texts.length - 1], 60)}. I’ll take it into account before I hand this back.`, { phase: 'commentary', thread });
  }

  private leadActor(thread: string): Actor | null {
    const a = [...this.sim.agents.values()].find((x) => x.role === 'lead' && (x.thread ?? 'lead') === thread);
    return a ? this.sim.actor(a.id) : null;
  }

  /** The request's Lead: "Lead" for the first, "Lead 2", "Lead 3"… for parallel requests. */
  private ensureLead(thread: string): Actor {
    const sim = this.sim;
    const found = this.leadActor(thread);
    if (found) return found;
    const n = [...sim.agents.values()].filter((a) => a.role === 'lead').length;
    const provider = sim.settings.lead_provider ?? 'claude';
    const lead = sim.putAgent(sim.newAgent({
      name: n ? `Lead ${n + 1}` : 'Lead', provider, model: provider === 'claude' ? 'claude-opus-5-5' : WORKER_MODELS[provider], role: 'lead',
      color: [6, 5, 1, 0][n % 4], billing: 'subscription', worktree: null, thread,
    }));
    const act = sim.actor(lead.id);
    this.sessionStarted(act);
    // Like Claude Code when a session starts: where each of its MCP servers stands. The office's
    // first Lead has one of each kind, so its sheet shows the section "MCP servers".
    if (!n && provider === 'claude') for (const server of DEMO_MCP) act.emit({ type: 'mcp_server', ...server });
    act.status('idle');
    return act;
  }

  /**
   * Every turn of a request's Lead goes through one FIFO (like Codly's prompt queue): a message
   * sent while the Lead is mid-turn waits, and runs as its next turn unless the Lead read it
   * during the turn (`readMidTurn`).
   */
  private withLead(thread: string, job: (lead: Actor) => Promise<void>): Promise<void> {
    const r = this.req(thread);
    const run = async () => {
      const lead = this.ensureLead(thread);
      this.busyLeads.add(lead.id);
      try {
        await job(lead);
      } catch (e) {
        if (!(e instanceof Cancelled)) console.error(e);
      } finally {
        this.sim.cancelled.delete(lead.id);
        this.busyLeads.delete(lead.id);
        if (lead.a.status !== 'offline') lead.status('idle');
      }
    };
    const p = r.chain.then(run);
    r.chain = p;
    return p;
  }

  private specFor(thread: string, text: string): Spec | null {
    // billing-api has its one plan: the demo scripts no other there.
    if (this.opts.billing) return null;
    const used = new Set([...this.reqs.values()].map((r) => r.spec).filter(Boolean));
    if (this.opts.threeCards && thread === 'lead' && !used.has('cards')) return 'cards';
    if (/lock|sign.?in|login|password|attempt|brute/i.test(text) && !used.has('lockout')) return 'lockout';
    if (/dark|theme|toggle|appearance/i.test(text) && !used.has('theme')) return 'theme';
    if (thread === 'lead' && !used.has('theme') && !/explain|how does|how do|flaky|checkout/i.test(text)) return 'theme';
    return null;
  }

  private async leadRespond(lead: Actor, r: Req, text: string) {
    if (!r.planId && !r.spec) r.spec = this.specFor(r.thread, text);
    if (!r.planId && r.spec) {
      // No plan yet (first message, or the Lead was interrupted while planning): plan with
      // everything you said so far.
      r.words.push(text);
      if (r.spec === 'theme') await this.planTheme(lead, r);
      else if (r.spec === 'cards') await this.planCards(lead, r);
      else await this.planLockout(lead, r);
      return;
    }
    await this.leadChat(lead, r, text);
  }

  private plan(r: Req): Plan | null {
    return r.planId ? (this.sim.plans.get(r.planId) ?? null) : null;
  }

  /** Does this request run its plans at once? The core's `lead_mode`: Auto or Direct, by the request's own choice or by "Run plans at once" in Settings. */
  private runsAtOnce(thread: string): boolean {
    const mode = this.sim.requestPrefs[thread]?.mode ?? (this.sim.settings.auto_run_plans ? 'auto' : 'plan');
    return mode === 'auto' || mode === 'direct';
  }

  /** Did this request start from an issue or a pull request ("From issue…")? Your message has it as its attachment. */
  private fromGithub(thread: string): boolean {
    const intake = (a: JsonValue | null) => !!a && typeof a === 'object' && !Array.isArray(a) && 'intake' in a;
    return this.sim.chats.some((c) => c.thread === thread && c.author === 'user' && intake(c.attachment));
  }

  /** Does the plan start by itself (the core's `auto` in `submit_plan_inner`)? Its request runs plans at once, did not start from GitHub, and the plan asks you nothing. */
  private startsAtOnce(r: Req, plan: Plan): boolean {
    return this.runsAtOnce(r.thread) && !this.fromGithub(r.thread) && plan.open_questions.length === 0;
  }

  private postPlanReady(r: Req, plan: Plan, n: number) {
    const sim = this.sim;
    // Like the core (`submit_plan_inner`): in a request that runs its plans at once, a plan whose
    // request started from GitHub waits for you all the same, and so does a plan with open
    // questions. The record says why, and the plan's card reads it from there, above Run plan.
    const atOnce = this.runsAtOnce(r.thread);
    const github = atOnce && this.fromGithub(r.thread);
    if (github) {
      sim.decision('plan_validate', 'rule', 'not auto-running: this request started from GitHub, and other people write there, so the plan waits for you', { rule: 'intake', plan_id: plan.id });
    } else if (atOnce && !this.startsAtOnce(r, plan)) {
      sim.decision('plan_validate', 'rule', 'not auto-running: the plan has open questions for you', { rule: 'open_questions', plan_id: plan.id });
    }
    const next = this.startsAtOnce(r, plan) ? 'Running it now.'
      : github ? 'This request started from GitHub, and other people write there, so it waits for you: review it on the board, then click Run plan.'
      : 'Review it on the board, then click Run plan.';
    // Codly posts the plan in the request's thread; the card rides on this line.
    sim.chat(r.thread, 'codly', `Plan ready: ${plan.summary} (${n} tasks). ${next}`, { plan_id: plan.id });
  }

  private async autoRun(r: Req) {
    const sim = this.sim;
    const ready = this.plan(r);
    const atOnce = !!ready && this.startsAtOnce(r, ready);
    if (!(sim.opts.autoplay || atOnce)) return;
    await sim.sleep(atOnce ? 600 : 2800);
    // Let a queued message to the Lead land first, so the card shows the final plan.
    await r.chain;
    await sim.sleep(600);
    const p = this.plan(r);
    if (p?.status === 'awaiting_approval') this.runPlan(p.id);
  }

  private newPlan(r: Req, spec: Spec, o: Pick<Plan, 'summary' | 'assumptions' | 'open_questions'>): Plan {
    const sim = this.sim;
    const now = sim.now();
    const plan = sim.putPlan({
      id: uid('p-'), workspace_id: sim.workspace!.id, thread: r.thread, request: r.words.join('\n\n'), ...o,
      weak_checks: [], status: 'planning', base_commit: null, base_branch: null, integration_branch: null, integration_worktree: null, merged_sha: null, kept: false, created_at: now, updated_at: now,
    });
    r.planId = plan.id;
    this.specOfPlan.set(plan.id, spec);
    return plan;
  }

  private newTask(plan: Plan, spec: Spec, key: string, title: string, goal: string, o: Partial<Task>): Task {
    const sim = this.sim;
    const now = sim.now();
    const t = sim.putTask({
      id: uid('t-'), plan_id: plan.id, key, title, goal, acceptance: [], write_scope: [], verify: [], depends_on: [], status: 'pending',
      assignee: null, agent_hint: null, account_hint: null, consultant: null, fresh_session: false, branch: null, worktree: null, attempt: 0, status_reason: 'Plan awaiting your go', summary: null,
      created_at: now, updated_at: now, ...o,
    });
    // (one with no demo files of its own gets a key of its own: plans reuse t1, t2…)
    const files: string | undefined = FILES[spec][key];
    this.fileKeys.set(t.id, files ?? `${spec}:${key}`);
    return t;
  }

  private taskOf(planId: string, key: string): Task {
    for (const t of this.sim.tasks.values()) if (t.plan_id === planId && t.key === key) return t;
    throw new Error(`no task ${key} in ${planId}`);
  }

  private fileKey(task: Task): string {
    return this.fileKeys.get(task.id) ?? task.key;
  }

  // ───────────── request 1: the dark-mode toggle ─────────────

  private async planTheme(lead: Actor, r: Req) {
    const sim = this.sim;
    await lead.sleep(500);
    await lead.turn(r.words[r.words.length - 1], async () => {
      await lead.think(
        'The user wants a theme toggle that persists per user and defaults to the system. I need to know how tokens are defined, ' +
          'whether anything already switches themes, and where settings are stored. Read before planning.',
      );
      await lead.tool({ name: 'Read', activity: 'read', target: { path: 'README.md' }, ms: 900, output: README });
      await lead.tool({ name: 'Glob', activity: 'search', target: { query: 'src/**/*theme*' }, ms: 700, output: 'No files found' });
      await lead.tool({ name: 'Grep', activity: 'search', target: { query: 'dark|prefers-color-scheme|data-theme' }, ms: 1300,
        output: 'src/styles/tokens.css:11:.dark {\nsrc/hooks/useMediaQuery.ts:4:  const mql = window.matchMedia(query);' });
      lead.retrieval('dark mode toggle settings persist theme per user', [
        cite('C1', 'src/styles/tokens.css', 1, 23, ':root', ['path: tokens', 'text: dark, theme'], 12.4),
        cite('C2', 'src/app/settings/page.tsx', 1, 21, 'SettingsPage', ['path: settings', 'symbol: SettingsPage'], 10.9),
        cite('C3', 'src/server/settings/route.ts', 1, 27, 'PATCH', ['path: settings', 'text: patch, user settings'], 9.7),
        cite('C4', 'prisma/schema.prisma', 18, 25, 'UserSettings', ['symbol: UserSettings', 'text: settings'], 8.8),
        cite('C5', 'src/hooks/useMediaQuery.ts', 1, 19, 'useMediaQuery', ['text: prefers color scheme'], 6.1),
      ]);
      for (const path of ['src/styles/tokens.css', 'src/app/settings/page.tsx', 'src/server/settings/route.ts', 'prisma/schema.prisma', 'src/app/settings/LocaleSelect.tsx'])
        await lead.tool({ name: 'Read', activity: 'read', target: { path }, ms: 700 });
      await lead.say('Tokens already ship a dark palette under `.dark`, but nothing switches it, and `UserSettings` has no theme field. Splitting this into three tasks.', { phase: 'commentary' });
      await lead.tool({ name: 'mcp__codly__codly_submit_plan', activity: 'plan', target: { server: 'codly', tool: 'codly_submit_plan' }, ms: 1100 });
      const plan = this.submitTheme(lead, r);
      this.postPlanReady(r, plan, 3);
      const keyboard = r.words.some((w) => KEYBOARD.test(w));
      await lead.say(
        'Two tasks start right away and the toggle waits for both. I routed **t1** to Claude Code (TypeScript UI), **t2** to Codex (API and migration) ' +
          'and **t3** to Gemini CLI, so a different vendor exercises t1’s API.' +
          (keyboard ? ' Keyboard support is part of t3’s acceptance criteria, as you asked.' : ''),
        { thread: r.thread },
      );
    });
    lead.status('idle');
    void this.autoRun(r);
  }

  private submitTheme(lead: Actor, r: Req): Plan {
    const sim = this.sim;
    const plan = this.newPlan(r, 'theme', {
      summary: 'Dark mode toggle in Settings',
      assumptions: ['“System” stays the default for existing users', 'No server-side theme rendering in this pass (first paint may flash once)'],
      open_questions: ['Should the choice sync across devices? Assumed yes: it is stored per user.'],
    });
    const t1 = this.newTask(plan, 'theme', 't1', 'Theme provider and tokens', 'Add a `ThemeProvider` that resolves system / light / dark, sets `data-theme` on `<html>`, and exposes `useTheme()`.', {
      acceptance: ['`useTheme()` returns `{ theme, resolved, setTheme }`', 'System changes apply live, without a reload', 'Unit tests cover resolution and persistence'],
      write_scope: ['src/theme/**', 'src/styles/tokens.css', 'src/app/layout.tsx'], verify: ['npm test -- theme'], agent_hint: 'claude',
    });
    const t2 = this.newTask(plan, 'theme', 't2', 'Persist the theme preference', 'Store `theme` on `UserSettings` and expose it through `GET`/`PATCH /api/settings`.', {
      acceptance: ['A migration adds `theme` with default `system`', 'PATCH validates the value', 'API tests pass'],
      write_scope: ['prisma/**', 'src/server/settings/**'], verify: ['npm run build', 'npm test -- settings'], agent_hint: 'codex',
    });
    const keyboard = r.words.some((w) => KEYBOARD.test(w));
    const t3 = this.newTask(plan, 'theme', 't3', 'Settings toggle and e2e test', 'Add a System / Light / Dark toggle to `/settings`, wired to `useTheme()` and the settings API.', {
      acceptance: [
        'The toggle shows the saved value on load', 'Changing it updates `<html data-theme>` immediately',
        ...(keyboard ? ['Arrow keys move between the options (a radiogroup)'] : []),
        'A Playwright test covers all three states and a reload',
      ],
      write_scope: ['src/app/settings/**', 'tests/e2e/settings-theme.spec.ts'], verify: ['npx playwright test settings-theme'],
      depends_on: [t1.id, t2.id], agent_hint: 'gemini',
    });
    sim.decision('plan_validated', 'rule', 'Plan is a valid DAG: 3 tasks, 2 dependencies, no cycles', { plan_id: plan.id, tasks: 3, edges: 2 }, { agent: lead.id });
    sim.decision('scope_check', 'rule', 'Write scopes are disjoint, so t1 and t2 can run in parallel', {
      plan_id: plan.id, t1: t1.write_scope, t2: t2.write_scope, t3: t3.write_scope,
    }, { agent: lead.id });
    sim.decision('dependency', 'model', 't3 depends on t1 and t2: it imports useTheme() and calls PATCH /api/settings', { plan_id: plan.id, task: 't3' }, { agent: lead.id, task: t3.id });
    sim.triple(`task:${t3.id}`, 'depends_on', `task:${t1.id}`, 'observed');
    sim.triple(`task:${t3.id}`, 'depends_on', `task:${t2.id}`, 'observed');
    return sim.patchPlan(plan.id, { status: 'awaiting_approval' });
  }

  // ───────────── request 2: the sign-in lockout (in parallel) ─────────────

  private async planLockout(lead: Actor, r: Req) {
    await lead.sleep(500);
    await lead.turn(r.words[r.words.length - 1], async () => {
      await lead.think(
        'A lockout after repeated failures: find where sign-in happens and whether a limiter exists. Another request is changing settings ' +
          'and theme files right now, so this plan must stay out of their write scopes.',
      );
      await lead.tool({ name: 'Grep', activity: 'search', target: { query: 'signIn|rateLimit|hit\\(' }, ms: 1200,
        output: 'src/server/auth.ts:14:export async function signIn(email: string, password: string)\nsrc/server/rate-limit.ts:4:export async function hit(key: string, limit: number, windowSec: number)' });
      this.sim.actor(lead.id).retrieval('sign in failed attempts lockout rate limit', [
        cite('C1', 'src/server/auth.ts', 1, 44, 'signIn', ['symbol: signIn', 'text: sign in'], 12.1),
        cite('C2', 'src/server/rate-limit.ts', 1, 9, 'hit', ['path: rate-limit', 'text: limit window'], 10.4),
        cite('C3', 'src/app/(auth)/sign-in/SignInForm.tsx', 1, 19, 'SignInForm', ['path: sign-in', 'symbol: SignInForm'], 9.2),
      ]);
      for (const path of ['src/server/auth.ts', 'src/server/rate-limit.ts', 'src/app/(auth)/sign-in/SignInForm.tsx'])
        await lead.tool({ name: 'Read', activity: 'read', target: { path }, ms: 650 });
      await lead.say('`hit()` in `rate-limit.ts` already keeps fixed windows in Redis, so the lockout can reuse it; the form only needs a clear message.', { phase: 'commentary' });
      await lead.tool({ name: 'mcp__codly__codly_submit_plan', activity: 'plan', target: { server: 'codly', tool: 'codly_submit_plan' }, ms: 900 });
      const plan = this.submitLockout(lead, r);
      this.postPlanReady(r, plan, 2);
      await lead.say(
        'Codex counts failures and locks the email (**t1**); OpenCode then shows the message on the sign-in page with an e2e test (**t2**). ' +
          'Neither touches the files the dark-mode plan is writing, so both plans can run side by side.',
        { thread: r.thread },
      );
    });
    lead.status('idle');
    void this.autoRun(r);
  }

  private submitLockout(lead: Actor, r: Req): Plan {
    const sim = this.sim;
    const plan = this.newPlan(r, 'lockout', {
      summary: 'Sign-in lockout after failed attempts',
      assumptions: ['Failures count per email address, not per IP', 'The lock lifts on its own after 15 minutes (no admin unlock in this pass)'],
      open_questions: [],
    });
    const t1 = this.newTask(plan, 'lockout', 't1', 'Count failed sign-ins and lock the email', 'Track failures per email in `rate-limit.ts`; after 5 within 15 minutes, `signIn()` answers `LOCKED` until the window passes.', {
      acceptance: ['5 failures within 15 minutes lock the email for 15 minutes', 'A successful sign-in clears the count', 'Unit tests cover the lock and its expiry'],
      write_scope: ['src/server/rate-limit.ts', 'src/server/auth.ts', 'src/server/rate-limit.test.ts'], verify: ['npm test -- rate-limit'], agent_hint: 'codex',
    });
    const t2 = this.newTask(plan, 'lockout', 't2', 'Lockout message on the sign-in page', 'When `signIn()` answers `LOCKED`, show “Too many attempts — try again in N minutes” on `/sign-in`, with a Playwright test.', {
      acceptance: ['The message says how long to wait', 'The form is disabled while locked, with a reset-password link', 'A Playwright test covers five failures'],
      write_scope: ['src/app/(auth)/sign-in/**', 'tests/e2e/auth-lockout.spec.ts'], verify: ['npx playwright test auth-lockout'], depends_on: [t1.id], agent_hint: 'opencode',
    });
    sim.decision('plan_validated', 'rule', 'Plan is a valid DAG: 2 tasks, 1 dependency, no cycles', { plan_id: plan.id, tasks: 2, edges: 1 }, { agent: lead.id });
    sim.decision('scope_check', 'rule', 'No overlap with the running dark-mode plan’s write scopes', { plan_id: plan.id, t1: t1.write_scope, t2: t2.write_scope }, { agent: lead.id });
    return sim.patchPlan(plan.id, { status: 'awaiting_approval' });
  }

  // ───────────── `?cards=3`: the release checklist ─────────────

  private async planCards(lead: Actor, r: Req) {
    await lead.sleep(500);
    await lead.turn(r.words[r.words.length - 1], async () => {
      await lead.think('Four small chores that touch different files: each can be its own task, and none waits for another.');
      await lead.tool({ name: 'Read', activity: 'read', target: { path: 'README.md' }, ms: 700, output: README });
      await lead.tool({ name: 'Glob', activity: 'search', target: { query: '{LICENSE,CHANGELOG.md,.husky/*}' }, ms: 600, output: 'LICENSE\nCHANGELOG.md' });
      await lead.tool({ name: 'mcp__codly__codly_submit_plan', activity: 'plan', target: { server: 'codly', tool: 'codly_submit_plan' }, ms: 900 });
      const plan = this.submitCards(lead, r);
      this.postPlanReady(r, plan, 4);
      await lead.say('Four tasks, and none waits for another, so all four start at once: **t1** the lint hook, **t2** the environment docs, **t3** the licence year and **t4** the changelog.', { thread: r.thread });
    });
    lead.status('idle');
    void this.autoRun(r);
  }

  private submitCards(lead: Actor, r: Req): Plan {
    const sim = this.sim;
    const plan = this.newPlan(r, 'cards', {
      summary: 'Release checklist',
      assumptions: ['The release is 2.5.0', 'The licence keeps its holder, Acme Inc.'],
      open_questions: [],
    });
    const tasks = [
      this.newTask(plan, 'cards', 't1', 'Lint staged files before each commit', 'Add a lint-staged config and a pre-commit hook that runs it.', {
        acceptance: ['A commit runs eslint and prettier on the staged files only'], write_scope: ['.lintstagedrc.json', '.husky/**'], verify: ['npx lint-staged --diff HEAD'], agent_hint: 'claude',
      }),
      this.newTask(plan, 'cards', 't2', 'Document the environment variables', 'List every environment variable in `docs/env.md`, and point to it from the README.', {
        acceptance: ['Each variable the code reads is listed, with what it is'], write_scope: ['docs/env.md', 'README.md'], verify: ['npx markdownlint docs/env.md README.md'], agent_hint: 'codex',
      }),
      this.newTask(plan, 'cards', 't3', 'Update the licence year', 'The licence still says 2024: make it name the years the code was written in.', {
        acceptance: ['LICENSE names this year'], write_scope: ['LICENSE'], verify: ['grep -q 2026 LICENSE'], agent_hint: 'claude',
      }),
      this.newTask(plan, 'cards', 't4', 'Add the release to the changelog', 'Add a `2.5.0` section on top of `CHANGELOG.md` with what this release brings.', {
        acceptance: ['CHANGELOG.md starts with the 2.5.0 section'], write_scope: ['CHANGELOG.md'], verify: ['npx markdownlint CHANGELOG.md'], agent_hint: 'gemini',
      }),
    ];
    sim.decision('plan_validated', 'rule', 'Plan is a valid DAG: 4 tasks, no dependencies, no cycles', { plan_id: plan.id, tasks: 4, edges: 0 }, { agent: lead.id });
    sim.decision('scope_check', 'rule', 'Write scopes are disjoint, so all four tasks can run in parallel', { plan_id: plan.id, ...Object.fromEntries(tasks.map((t) => [t.key, t.write_scope])) }, { agent: lead.id });
    return sim.patchPlan(plan.id, { status: 'awaiting_approval' });
  }

  /** A file a task writes, with the text its worker decided on: it is in the task's diff from then on. */
  private wrote(task: Task, file: RepoFile) {
    this.written.set(task.id, [...(this.written.get(task.id) ?? []).filter((f) => f.path !== file.path), file]);
  }

  /** The paths a task wrote with a text of its own (`wrote`), for its commit. */
  private writtenPaths(task: Task): string[] {
    return (this.written.get(task.id) ?? []).map((f) => f.path);
  }

  /** The release checklist's t1. Max writes the config, then asks before the command that adds the git hook. */
  private async trackC1(max: Actor, task: Task) {
    const steps = ['Write the lint-staged config', 'Add the pre-commit hook', 'Tell the Lead'];
    await max.sleep(900);
    await max.turn(`Task t1 · ${task.title}\n\n${task.goal}`, async () => {
      max.plan(listAt(steps, 0));
      await max.think('A config for lint-staged first, then husky adds the hook that runs it.', { pace: 40 });
      this.wrote(task, { path: '.lintstagedrc.json', before: null, after: LINT_STAGED });
      await max.tool({ name: 'Write', activity: 'create', target: { path: '.lintstagedrc.json' }, input: { file_path: '.lintstagedrc.json', content: LINT_STAGED }, ms: 1200, files: ['.lintstagedrc.json'], taskKey: task.id });
      max.plan(listAt(steps, 1));
      // Left for you, also with `?autoplay=1`: the three cards are there to look at.
      const hook = await this.approval(max, 'npx husky init', 'Adds a pre-commit hook in .husky/ that runs lint-staged before each commit.', ['changes the git hooks'], ['husky - Git hooks set up in .husky/'], null);
      if (hook === 'approved') {
        this.wrote(task, { path: '.husky/pre-commit', before: null, after: PRE_COMMIT });
        await max.tool({ name: 'Write', activity: 'create', target: { path: '.husky/pre-commit' }, input: { file_path: '.husky/pre-commit', content: PRE_COMMIT }, ms: 800, files: ['.husky/pre-commit'], taskKey: task.id });
      }
      max.plan(listAt(steps, 2));
      await max.say(hook === 'approved' ? 'A commit now runs eslint and prettier on the staged files.' : 'The config is there; without the hook it runs only when you call `npx lint-staged`.');
    });
    await this.gate(max, task, {
      files: this.writtenPaths(task), message: `t1: ${task.title}`, verify: task.verify.join(' && '), tail: 'ok',
      summary: 'lint-staged runs eslint and prettier on the staged files.', integration: true,
    });
  }

  /** The release checklist's t2. Noa writes the docs, then asks before it changes the README, with the change. */
  private async trackC2(noa: Actor, task: Task) {
    const readme: RepoFile = { path: 'README.md', before: README, after: README_ENV };
    const steps = ['List the variables in docs/env.md', 'Point to it from the README'];
    await noa.sleep(900);
    await noa.turn(`Task t2 · ${task.title}\n\n${task.goal}`, async () => {
      noa.plan(listAt(steps, 0));
      await noa.think('Three variables are read in the code: the database, the session secret and the Stripe test key.', { pace: 40 });
      this.wrote(task, { path: 'docs/env.md', before: null, after: ENV_DOC });
      await noa.tool({ name: 'fileChange', activity: 'create', target: { path: 'docs/env.md' }, ms: 1200, files: ['docs/env.md'], taskKey: task.id });
      noa.plan(listAt(steps, 1));
      const no = await this.editApproval(noa, readme.path, diffOf([readme]), null);
      if (no === null) {
        this.wrote(task, readme);
        await noa.tool({ name: 'fileChange', activity: 'edit', target: { path: readme.path }, ms: 900, files: [readme.path], taskKey: task.id });
      }
      noa.plan(listAt(steps, 2));
      await noa.say(no === null ? 'The variables are in `docs/env.md`, and the README points there.' : 'The variables are in `docs/env.md`; the README stays as it was.');
    });
    await this.gate(noa, task, {
      files: this.writtenPaths(task), message: `t2: ${task.title}`, verify: task.verify.join(' && '), tail: 'ok',
      summary: 'docs/env.md lists the environment variables.', integration: true,
    });
  }

  /** The release checklist's t3. Rio asks which years the licence names before it changes it. */
  private async trackC3(rio: Actor, task: Task) {
    const options = [
      { label: '2024–2026', description: 'Every year the code was written in (recommended)' },
      { label: '2026', description: 'This year only' },
    ];
    await rio.sleep(900);
    await rio.turn(`Task t3 · ${task.title}\n\n${task.goal}`, async () => {
      await rio.think('A licence can name the first year or every year: that is the owner’s choice, so ask.', { pace: 40 });
      const years = (await this.question(rio, 'The licence says 2024. Which years should it name?', 'Licence', options, null)) ?? options[0].label;
      this.wrote(task, { path: 'LICENSE', before: licence('2024'), after: licence(years) });
      await rio.tool({ name: 'Edit', activity: 'edit', target: { path: 'LICENSE' }, input: { file_path: 'LICENSE', old_string: 'Copyright (c) 2024', new_string: `Copyright (c) ${years}` }, ms: 700, files: ['LICENSE'], taskKey: task.id });
      await rio.say(`The licence names ${years} now.`);
    });
    await this.gate(rio, task, {
      files: ['LICENSE'], message: `t3: ${task.title}`, verify: task.verify.join(' && '), tail: 'ok', summary: 'LICENSE names the years of the code.', integration: true,
    });
  }

  /** The release checklist's t4: Sam asks nothing, and finishes while the other three wait for you. */
  private async trackC4(sam: Actor, task: Task) {
    await sam.sleep(900);
    await sam.turn(`Task t4 · ${task.title}\n\n${task.goal}`, async () => {
      await sam.think('The changelog keeps the newest release on top.', { pace: 40 });
      await sam.tool({ name: 'read_file', activity: 'read', target: { path: 'CHANGELOG.md' }, ms: 600, output: CHANGELOG_BEFORE });
      this.wrote(task, { path: 'CHANGELOG.md', before: CHANGELOG_BEFORE, after: CHANGELOG_AFTER });
      await sam.tool({ name: 'replace', activity: 'edit', target: { path: 'CHANGELOG.md' }, ms: 1200, files: ['CHANGELOG.md'], taskKey: task.id });
      await sam.say('`CHANGELOG.md` starts with the 2.5.0 section.');
    });
    await this.gate(sam, task, {
      files: ['CHANGELOG.md'], message: `t4: ${task.title}`, verify: task.verify.join(' && '), tail: 'ok', summary: 'CHANGELOG.md has the 2.5.0 section.', integration: true,
    });
  }

  private async leadChat(lead: Actor, r: Req, text: string) {
    const sim = this.sim;
    const plan = this.plan(r);
    const tasks = plan ? [...sim.tasks.values()].filter((t) => t.plan_id === plan.id) : [];
    const done = tasks.filter((t) => t.status === 'done').length;
    let reply: string;
    let amend: (() => void) | null = null;
    if (plan && this.specOfPlan.get(plan.id) === 'theme' && KEYBOARD.test(text)) {
      const t3 = this.taskOf(plan.id, 't3');
      if (t3.status === 'pending' || t3.status === 'ready') {
        amend = () => {
          if (!t3.acceptance.some((a) => KEYBOARD.test(a)))
            sim.patchTask(t3.id, { acceptance: [...t3.acceptance.slice(0, 2), 'Arrow keys move between the options (a radiogroup)', ...t3.acceptance.slice(2)] });
          sim.decision('plan_amended', 'model', 'Added keyboard support to t3’s acceptance criteria (you asked while the plan was being drafted)', { plan_id: plan.id, task: 't3' }, { agent: lead.id, task: t3.id });
        };
        reply = 'Good call — added to **t3**: arrow keys move between System, Light and Dark, and the toggle is a proper radiogroup. The Playwright test will cover it too.';
      } else reply = 't3 is already underway; I’ll ask Cy to make arrow keys move between the options before it hands back, and check it in review.';
    } else if (!plan && !r.spec) {
      if (this.opts.billing) reply = 'Tell me what you’d like built and I’ll read the code and draft a plan for it. (This browser demo scripts its plans in acme/web; billing-api has the one you see.)';
      else if (/flaky|checkout/i.test(text))
        reply = 'There’s already a card for that: **a2 · Fix flaky checkout e2e test** is blocked because it needs `STRIPE_TEST_KEY` in `.env.test`. Add the key, move a2 to Ready, and a free agent picks it up.';
      else if (/auth|session/i.test(text))
        reply = '`requireUser()` in `src/server/auth.ts` reads the session cookie through `readSession()` (`src/server/session.ts`) and redirects to `/sign-in` when it is missing. Sessions live in Postgres; `createSession()` sets an httpOnly cookie for 30 days.';
      else reply = 'Tell me what you’d like built and I’ll read the code and draft a plan for it. (This browser demo scripts two plans: a dark-mode toggle and a sign-in lockout.)';
    } else if (!plan) reply = 'Tell me what you’d like built and I’ll draft a plan.';
    else if (plan.status === 'awaiting_approval')
      reply = 'The plan is waiting on your go — **Run plan** starts the first tasks in parallel. If you want to change scope first, say what and I’ll revise it.';
    else if (plan.status === 'running' || plan.status === 'integrating') {
      const live = tasks.filter((t) => t.status !== 'done' && t.status !== 'pending').map((t) => `${t.key} is ${t.status === 'running' ? 'in progress' : t.status.replace('_', ' ')}`);
      reply = `${done} of ${tasks.length} tasks merged${live.length ? '; ' + live.join(', ') : ''}. Noted “${text.slice(0, 60)}${text.length > 60 ? '…' : ''}” — anything that changes scope goes into a follow-up plan you approve.`;
    } else if (plan.status === 'done') reply = `Happy to plan a follow-up. ${FOLLOW_UP[this.specOfPlan.get(plan.id) ?? 'lockout']} — want a plan for that?`;
    else reply = 'That plan is cancelled. Tell me what you’d like instead and I’ll draft a new one.';
    await lead.sleep(400);
    await lead.turn(text, async () => {
      await lead.think('Answer from the current plan state; do not start new work without a plan.', { pace: 40 });
      amend?.();
      await lead.say(reply, { thread: r.thread });
    });
    lead.status('idle');
  }

  private async agentRespond(agentId: string, sent: Sent) {
    const sim = this.sim;
    const thread = `agent:${agentId}`;
    // Nobody is there to read it (the agent is gone): it does not wait for ever.
    if (!sim.agents.get(agentId)) return void this.take(thread, sent);
    const act = sim.actor(agentId);
    const text = sent.text;
    // One FIFO per agent: a message to a busy worker waits for its turn to end, unless the
    // worker read it during that turn (`readMidTurn`).
    const prev = this.agentChains.get(agentId) ?? Promise.resolve();
    const next = prev.then(async () => {
      await sim.sleep(500);
      await sim.waitFor(() => !act.inTurn);
      await sim.sleep(300);
      if (!this.take(thread, sent)) return;
      sim.cancelled.delete(agentId);
      const a = act.a;
      const task = a.task_id ? sim.tasks.get(a.task_id) : null;
      const busy = this.running.has(agentId) && !!task;
      // A 1:1 Claude agent does what you ask, in its mode; the demo's other agents only answer.
      const works = !busy && a.role === 'solo' && a.provider === 'claude';
      // `?approval=…`: what this agent does with the message the demo sent it, once.
      const script = busy ? undefined : this.scripted.get(agentId);
      if (script) this.scripted.delete(agentId);
      // `?turn=failed`: this turn fails as an overloaded API makes it fail; your message can be sent again.
      const fails = !busy && !script && a.role === 'solo' && this.failNext;
      if (fails) this.failNext = false;
      const reply = busy
        ? `Noted. I’m finishing **${task!.key} · ${task!.title}** — I’ll take “${text.slice(0, 48)}${text.length > 48 ? '…' : ''}” into account before Codly merges it.`
        : 'I’m free. Assign me a task from the Plan room, or ask a Lead to plan something and I’ll pick it up.';
      try {
        await act.turn(text, async () => {
          if (fails) {
            await act.think('Read the request, then plan the change.', { pace: 40 });
            throw new TurnFailed(overloaded(a.provider));
          }
          if (script) await script(act);
          else if (works) await this.soloWork(act, text);
          else await act.say(reply, { thread });
        });
      } catch (e) {
        if (!(e instanceof Cancelled)) console.error(e);
        sim.cancelled.delete(agentId);
      }
      // However the turn ended (done, "Stop the turn", Send now), the agent waits for you
      // again, unless you stopped the agent itself or it is on a task.
      if (act.a.status !== 'offline' && !this.running.has(agentId)) act.status('idle');
    });
    this.agentChains.set(agentId, next);
    await next;
  }

  // ───────────── work that waits for your Resume ─────────────

  /**
   * `?resume=ask`: the office as a restart of Codly leaves it while "Resume work when Codly
   * opens" is on Ask (orchestrator/resume.rs). The first request's plan was running: its two
   * tasks that were in work are Ready again and say why they wait, the third waits for them,
   * and nothing starts before your Resume. Each held task keeps its worker of the last run
   * (offline: its process ended with Codly), its branch and its worktree.
   */
  private seedHeld() {
    const sim = this.sim;
    const r = this.req('lead');
    r.spec = 'theme';
    r.words.push(REQUEST);
    sim.chat('lead', 'user', REQUEST);
    const plan = this.submitTheme(this.ensureLead('lead'), r);
    this.postPlanReady(r, plan, 3);
    this.held = new Set(['t1', 't2'].map((key) => this.taskOf(plan.id, key).id));
    this.runPlan(plan.id);
    for (const id of this.held) {
      const key = sim.tasks.get(id)!.key;
      const route = ROUTES.theme[key];
      const worktree = `${this.worktrees}/${key}-${slug(route.name)}`;
      const worker = sim.newAgent({
        name: route.name, provider: route.provider, model: WORKER_MODELS[route.provider], role: 'worker', color: route.color, billing: 'subscription',
        taskId: id, worktree, status: 'offline',
      });
      sim.putAgent({ ...worker, run_id: null });
      sim.patchTask(id, { assignee: worker.id, branch: `${BRANCH.theme}/${key}`, worktree, attempt: 1, status_reason: HELD });
    }
  }

  /**
   * `resume_work`, as a read: what waits for your Resume (the core's `resume_offer`). The count
   * is of the tasks that were in work and are still ready: one you cancelled since is not
   * counted. Null: nothing is held. Like the core, a hold that waits for nothing any more ends
   * here by itself: you cancelled every task it held, and nothing else would start.
   */
  resumeOffer(): ResumeOffer | null {
    const held = this.held;
    if (!held) return null;
    const all = [...this.sim.tasks.values()];
    const tasks = all.filter((t) => held.has(t.id) && t.status === 'ready').length;
    if (tasks > 0 || this.startsByItself(all)) return { tasks };
    this.resumeWork();
    return null;
  }

  /**
   * Would work start by itself now (the core's `starts_by_itself`)? A running plan with a task
   * that can start, or with only its last checks left, or a ready task of no plan. (The demo
   * has no budget, and its pause holds nothing back.)
   */
  private startsByItself(tasks: Task[]): boolean {
    const done = (id: string) => tasks.some((t) => t.id === id && t.status === 'done');
    const starts = (t: Task) => t.status === 'ready' || (t.status === 'pending' && t.depends_on.every(done));
    const over = (t: Task) => t.status === 'done' || t.status === 'cancelled';
    for (const p of this.sim.plans.values()) {
      if (p.status !== 'running' && p.status !== 'integrating') continue;
      const own = tasks.filter((t) => t.plan_id === p.id);
      if (own.some(starts) || (own.some((t) => t.status === 'done') && own.every(over))) return true;
    }
    return tasks.some((t) => !t.plan_id && t.status === 'ready');
  }

  /**
   * `resume_work` with your Resume: the hold ends (the core's `end_hold`). Each task that was
   * in work and still waits goes on, and its history says so; another task that only showed
   * the hold's reason loses it. From here on tasks start as ever.
   */
  resumeWork() {
    const sim = this.sim;
    const held = this.held;
    if (!held) return;
    this.held = null;
    for (const t of [...sim.tasks.values()].filter((t) => t.status === 'ready')) {
      if (held.has(t.id)) {
        this.resumed.add(t.id);
        sim.patchTask(t.id, { status_reason: 'resuming after Codly restarted' });
        sim.decision('retry', 'rule', 'Resumed after restart: its worker goes on where it stopped', { plan_id: t.plan_id }, { task: t.id });
      } else if (t.status_reason === HELD) sim.patchTask(t.id, { status_reason: null });
    }
  }

  /**
   * Before a task starts. Like the core's scheduler: no task starts while the work of the last
   * run waits for your Resume, and a ready task says so meanwhile. Resolves when nothing is
   * held (any more).
   */
  private async notHeld(taskId: string) {
    const sim = this.sim;
    while (this.resumeOffer()) {
      const t = sim.tasks.get(taskId);
      if (t?.status === 'ready' && t.status_reason !== HELD) {
        sim.patchTask(taskId, { status_reason: HELD });
        sim.decision('schedule', 'rule', HELD, { task: t.key }, { task: taskId });
      }
      await sim.sleep(300);
    }
  }

  // ───────────── plan execution ─────────────

  runPlan(planId: string) {
    const sim = this.sim;
    const plan = sim.plans.get(planId);
    const spec = this.specOfPlan.get(planId);
    if (!plan || !spec || plan.status !== 'awaiting_approval') throw new Error('This plan is not waiting to run.');
    const base = sim.workspace?.head ?? sha();
    const branch = `${BRANCH[spec]}/integration`;
    sim.patchPlan(planId, { status: 'running', base_commit: base, base_branch: 'main', integration_branch: branch, integration_worktree: `${this.worktrees}/${spec}-integration` });
    sim.decision('integration_branch', 'rule', `Created ${branch} from main @ ${base.slice(0, 7)}`, { plan_id: planId, base });
    const tasks = [...sim.tasks.values()].filter((t) => t.plan_id === planId);
    for (const t of tasks) {
      if (!t.depends_on.length) sim.patchTask(t.id, { status: 'ready', status_reason: 'Dependencies met; starting' });
      else {
        sim.patchTask(t.id, { status: 'pending' });
        this.sayWait(t.id);
      }
    }
    void this.coordinate(planId);
  }

  /**
   * Why a pending task waits for the tasks it depends on, in the core's words (orchestrator/
   * scheduler.rs): one of them that cannot finish by itself (failed, blocked, cancelled) is named
   * with its status. Null: it waits on none any more.
   */
  private waitReason(t: Task): string | null {
    const undone = t.depends_on.flatMap((d) => this.sim.tasks.get(d) ?? []).filter((d) => d.status !== 'done');
    if (!undone.length) return t.depends_on.every((d) => this.sim.tasks.get(d)?.status === 'done') ? null : 'waiting: depends on a task that no longer exists';
    const stuck = undone.filter((d) => d.status === 'failed' || d.status === 'blocked' || d.status === 'cancelled');
    if (stuck.length) return `blocked: depends on ${stuck.map((d) => `${d.key} (${d.status})`).join(', ')}`;
    return `waiting: depends on ${undone.map((d) => d.key).join(', ')}`;
  }

  /** Like each pass of the core's scheduler: a pending task says why it waits, and the record says so each time that changes. */
  private sayWait(taskId: string) {
    const sim = this.sim;
    const t = sim.tasks.get(taskId);
    const reason = t?.status === 'pending' ? this.waitReason(t) : null;
    if (!t || !reason || reason === t.status_reason) return;
    sim.patchTask(t.id, { status_reason: reason });
    sim.decision('schedule', 'rule', reason, { task: t.key, plan_id: t.plan_id, reason }, { task: t.id });
  }

  /**
   * `home`: where a 1:1 agent works. A worker on a task works in that task's worktree: the one
   * the task has already (a task on its plan's own branch works in the plan's), else a new one.
   */
  private spawnWorker(provider: Provider, name: string, color: number, task: Task | null, model?: string | null, home: string | null = null): Actor {
    const sim = this.sim;
    const worktree = task ? (task.worktree ?? `${this.worktrees}/${task.key}-${slug(name)}`) : home;
    const a = sim.putAgent(sim.newAgent({
      name, provider, model: model ?? WORKER_MODELS[provider], role: task ? 'worker' : 'solo', color, billing: provider === 'opencode' ? 'third_party' : 'subscription',
      taskId: task?.id ?? null, worktree,
    }));
    const act = sim.actor(a.id);
    this.sessionStarted(act);
    return act;
  }

  /**
   * Like a CLI when its process starts (a new agent, or one that starts again for a turn): its
   * session, in the folder it works in, with its tools. A Lead plans in the repo.
   */
  private sessionStarted(act: Actor) {
    const sim = this.sim;
    const a = act.a;
    const transport = a.provider === 'claude' ? 'stream-json' : a.provider === 'codex' ? 'app-server' : 'acp';
    const lead = a.role === 'lead';
    act.emit({
      type: 'session_started', provider: a.provider, model: a.model, cwd: (lead ? null : a.worktree) ?? sim.workspace?.repo_path ?? REPO,
      cli_version: lead || a.provider === 'claude' ? '2.1.309' : a.provider === 'codex' ? '0.161.0' : a.provider === 'gemini' ? '0.62.0' : '1.18.31',
      session_id: a.session_id, billing: a.billing,
      tools: lead ? ['Read', 'Grep', 'Glob', 'WebFetch', 'mcp__codly__codly_search', 'mcp__codly__codly_submit_plan', 'mcp__codly__codly_memory_search']
        : transport === 'stream-json' ? ['Bash', 'Read', 'Edit', 'Write', 'Grep', 'Glob', 'TodoWrite', 'mcp__codly__codly_search', 'mcp__codly__codly_post_note']
        : transport === 'app-server' ? ['shell', 'apply_patch', 'update_plan', 'codly.codly_search']
        : ['read_file', 'write_file', 'replace', 'run_shell_command', 'glob', 'search_file_content', 'codly_search'],
    });
  }

  /**
   * A worker that Codly can give a task to by itself: at rest (not stopped, on no task, in no
   * turn). Never a Lead, a reviewer or a 1:1 agent you hired: you talk to that one.
   */
  private free(a: Agent): boolean {
    return a.role === 'worker' && a.status !== 'offline' && !this.running.has(a.id) && !this.sim.actor(a.id).inTurn;
  }

  /** `again`: the worker goes on with the task where it stopped (after your Resume): the same attempt. */
  private assign(task: Task, act: Actor, branchRoot: string, again = false) {
    const sim = this.sim;
    const worktree = `${this.worktrees}/${task.key}-${slug(act.a.name)}`;
    act.set({ task_id: task.id, worktree, role: act.a.role === 'lead' ? 'lead' : 'worker' });
    sim.patchTask(task.id, {
      status: 'running', assignee: act.id, branch: `${branchRoot}/${task.key}`, worktree, attempt: again ? task.attempt : task.attempt + 1, status_reason: null,
    });
    sim.triple(`agent:${act.id}`, 'works_on', `task:${task.id}`, 'observed', { agent: act.id });
  }

  private startPlanned(planId: string, key: string, reuse?: Actor) {
    const sim = this.sim;
    const plan = sim.plans.get(planId);
    const spec = this.specOfPlan.get(planId);
    if (!plan || !spec || plan.status === 'cancelled') return;
    const task = this.taskOf(planId, key);
    const route = ROUTES[spec][key];
    let act = reuse ?? null;
    const preassigned = task.assignee ? sim.agents.get(task.assignee) : null;
    if (!act && preassigned && !this.running.has(preassigned.id)) act = sim.actor(preassigned.id);
    if (!act) {
      // The worker the plan's route chose for this task before takes it again (it may have a
      // second name: "Fay 2" when you hired a "Fay"), else a worker of the route's name. A 1:1
      // agent you hired is never given a plan's task.
      const before = this.routed.get(`${planId}:${key}`);
      const takes = [before ? sim.agents.get(before) : null, sim.agentByName(route.name)].find((x) => x?.role === 'worker' && !this.running.has(x.id));
      act = takes ? sim.actor(takes.id) : null;
    }
    const chosenByYou = !!(preassigned && act && act.id === preassigned.id && this.userAssigned.has(task.id));
    // After your Resume the task's worker of the last run goes on: no new route.
    const again = this.resumed.delete(task.id) && !!preassigned && act?.id === preassigned.id;
    if (!act) act = this.spawnWorker(route.provider, this.unusedName(route.name), route.color, task);
    this.routed.set(`${planId}:${key}`, act.id);
    const a = act.a;
    const label = `${a.name} · ${PROVIDER_NAME[a.provider]}`;
    if (!again) {
      sim.decision('route', chosenByYou ? 'human' : 'rule', chosenByYou ? `${key} → ${label}: you assigned it` : `${key} → ${label}: ${route.why}`, {
        plan_id: plan.id, task: key, chosen: a.provider, agent: a.name, alternatives: route.alternatives,
      }, { agent: a.id, task: task.id });
    }
    this.assign(task, act, BRANCH[spec], again);
    const worker = act;
    const tracks: Record<Spec, Record<string, () => Promise<void>>> = {
      theme: { t1: () => this.trackT1(worker, task), t2: () => this.trackT2(worker, task), t3: () => this.trackT3(worker, task) },
      lockout: { t1: () => this.trackL1(worker, task), t2: () => this.trackL2(worker, task) },
      retries: { t1: () => this.trackB1(worker, task), t2: () => this.trackB2(worker, task) },
      cards: { t1: () => this.trackC1(worker, task), t2: () => this.trackC2(worker, task), t3: () => this.trackC3(worker, task), t4: () => this.trackC4(worker, task) },
    };
    const body = tracks[spec][key];
    if (body) void this.runTrack(act, task, body);
  }

  private async runTrack(act: Actor, task: Task, body: () => Promise<void>) {
    const sim = this.sim;
    this.running.add(act.id);
    try {
      await body();
    } catch (e) {
      if (!(e instanceof Cancelled)) {
        console.error(e);
        return;
      }
      sim.cancelled.delete(act.id);
      const t = sim.tasks.get(task.id)!;
      const plan = t.plan_id ? sim.plans.get(t.plan_id) : null;
      // Only a task still in flight: one you stopped on the board (or retried since) keeps its status.
      if (['running', 'verifying', 'merging'].includes(t.status) && plan?.status !== 'cancelled')
        sim.patchTask(t.id, { status: 'blocked', status_reason: 'Interrupted by you. Retry to resume.' });
      if (act.a.status !== 'offline') act.status('idle');
    } finally {
      this.running.delete(act.id);
      // (a command the agent still had in the background ends with its work on the task)
      for (const left of demoBackground(sim).of(act.id)) demoBackground(sim).end(act.id, left.id, 'stopped');
    }
  }

  private async gate(act: Actor, task: Task, o: { files: string[]; message: string; verify: string; tail: string; summary: string; integration: boolean }) {
    const sim = this.sim;
    // Like the core: the gate ends when the task was stopped or cancelled meanwhile.
    const live = () => {
      const s = sim.tasks.get(task.id)!.status;
      if (s !== 'verifying' && s !== 'merging') throw new Cancelled();
    };
    act.status('done', 'done');
    sim.patchTask(task.id, { status: 'verifying', status_reason: `Committing and running ${o.verify}` });
    await sim.sleep(900);
    live();
    const commit = sha();
    sim.emit({ type: 'committed', sha: commit, files: o.files, message: o.message }, { agent: act.id, task: task.id });
    sim.committedKeys.add(this.fileKey(task));
    await sim.sleep(1700);
    live();
    // Like the core: one result for each of the task's verify commands, under its exact text.
    task.verify.forEach((command, i) => sim.emit({ type: 'verified', command, ok: true, exit_code: 0, tail: i === task.verify.length - 1 ? o.tail : '' }, { agent: act.id, task: task.id }));
    // Like the core: then the repo's required checks, while you have them turned on (a command the task ran already is not run twice).
    for (const command of this.checks.required()) {
      if (!task.verify.includes(command)) sim.emit({ type: 'verified', command, ok: true, exit_code: 0, tail: 'ok' }, { agent: act.id, task: task.id });
    }
    if (o.integration) {
      const branch = (task.plan_id && sim.plans.get(task.plan_id)?.integration_branch) || 'codly/integration';
      sim.patchTask(task.id, { status: 'merging', status_reason: 'In the merge queue' });
      await sim.sleep(1100);
      live();
      sim.decision('merge', 'rule', `${task.key} merged into ${branch} — merge-tree pre-check clean`, { plan_id: task.plan_id, task: task.key, sha: commit }, { agent: act.id, task: task.id });
      sim.emit({ type: 'merged', sha: sha(), branch }, { agent: act.id, task: task.id });
    }
    sim.patchTask(task.id, { status: 'done', status_reason: null, summary: o.summary });
    sim.memory(o.summary, 'task', 'summary', 'codly', { scopeId: task.id, importance: 0.5, title: task.title, refs: [`task:${task.id}`] });
    await sim.sleep(2200);
    // The worker is free again. (One you stopped meanwhile stays stopped.)
    if (!this.running.has(act.id) || act.a.task_id === task.id) act.set(act.a.status === 'offline' ? { task_id: null } : { status: 'idle', activity: null, activity_label: null, task_id: null });
  }

  /**
   * Wait for your answer to a request that is on screen. With `?autoplay=1` autoplay allows it
   * after `auto` ms, and the request says so (`answers`: what autoplay picks for a question);
   * `auto` null: it waits for you all the same.
   */
  private async answered(act: Actor, ap: Approval, label: string, auto: number | null, answers: unknown = null): Promise<Approval['status']> {
    const sim = this.sim;
    act.status('waiting_approval', 'await_approval', label);
    while (sim.approvals.get(ap.id)!.status === 'pending') {
      await act.sleep(250);
      if (sim.opts.autoplay && auto !== null && sim.now() - ap.created_at > auto) this.resolveApproval(ap.id, 'allow_once', null, answers, AUTOPLAY);
    }
    return sim.approvals.get(ap.id)!.status;
  }

  /** A command that asks first. `output`: what it prints once it is allowed. `auto`: as for `answered`. */
  private async approval(act: Actor, command: string, reason: string, risks: string[], output: string[], auto: number | null = 3500): Promise<'approved' | 'denied'> {
    const sim = this.sim;
    const a = act.a;
    const call = uid('call-');
    act.emit({
      type: 'tool_started', call_id: call, name: shellTool(a.provider), activity: 'execute',
      target: { path: null, paths: [], command, query: null, url: null, server: null, tool: null }, input: { command }, confidence: 'observed',
    });
    const ap: Approval = {
      id: uid('ap-'), agent_id: a.id, run_id: a.run_id ?? '', task_id: a.task_id, call_id: call,
      subject: { kind: 'command', command, cwd: a.worktree }, reason, choices: ['allow_once', 'allow_session', 'deny'], risks,
      status: 'pending', decided_by: null, created_at: sim.now(),
    };
    sim.approvals.set(ap.id, ap);
    act.emit({ type: 'approval_requested', approval: ap });
    const rule = this.rules.covering(ap);
    if (rule) {
      // Like the core's broker: a rule you saved for this repo answers in your place.
      sim.approvals.set(ap.id, { ...ap, status: 'approved', decided_by: 'policy' });
      act.emit({ type: 'approval_resolved', approval_id: ap.id, status: 'approved', by: 'policy' });
      sim.decision('approval_policy', 'rule', `auto-allowed your rule for this repo: ${describeRule(rule)}`, { approval_id: ap.id, rule: rule.id }, { agent: a.id, task: a.task_id });
    }
    const st = rule ? 'approved' : await this.answered(act, ap, `${command.split(' ').slice(0, 3).join(' ')}…`, auto);
    if (st === 'approved') {
      act.status('working', 'execute', command.split(' ').slice(0, 3).join(' '));
      for (const l of output) {
        await act.sleep(320 + Math.random() * 300);
        act.emit({ type: 'tool_output', call_id: call, chunk: l + '\n' });
      }
      act.emit({ type: 'tool_completed', call_id: call, ok: true, output: output.join('\n'), exit_code: 0, duration_ms: sim.now() - ap.created_at, tests_passed: null, tests_failed: null });
      return 'approved';
    }
    act.emit({ type: 'tool_completed', call_id: call, ok: false, output: 'Declined by the user.', exit_code: null, duration_ms: sim.now() - ap.created_at, tests_passed: null, tests_failed: null });
    act.status('working', 'thinking');
    return 'denied';
  }

  private answers = new Map<string, unknown>();
  /** approval id → the note you sent with a "no". */
  private notes = new Map<string, string>();

  /** AskUserQuestion-style question; resolves with the chosen label (or null when skipped). `auto`: as for `answered`. */
  private async question(act: Actor, question: string, header: string, options: { label: string; description: string }[], auto: number | null = 4000): Promise<string | null> {
    const sim = this.sim;
    const a = act.a;
    const ap: Approval = {
      id: uid('ap-'), agent_id: a.id, run_id: a.run_id ?? '', task_id: a.task_id, call_id: null,
      subject: { kind: 'question', questions: { questions: [{ question, header, options, multiSelect: false }] } },
      reason: null, choices: ['allow_once', 'deny'], risks: [], status: 'pending', decided_by: null, created_at: sim.now(),
    };
    sim.approvals.set(ap.id, ap);
    act.emit({ type: 'approval_requested', approval: ap });
    const st = await this.answered(act, ap, header, auto, { [question]: options[0].label });
    act.status('working', 'thinking');
    if (st !== 'approved') return null;
    const ans = this.answers.get(ap.id) as Record<string, unknown> | undefined;
    const v = ans ? (ans[question] ?? Object.values(ans)[0]) : null;
    return typeof v === 'string' && v ? v : Array.isArray(v) && typeof v[0] === 'string' ? v[0] : null;
  }

  /**
   * Claude's plan mode: the plan arrives as a request (`ExitPlanMode`) that no rule answers in
   * your place. Resolves null when you accept it, and your note ("" for none) when you send it
   * back; "Stop the turn" ends the turn.
   */
  private async showPlan(act: Actor, plan: string): Promise<string | null> {
    const sim = this.sim;
    const a = act.a;
    const call = uid('call-');
    act.emit({
      type: 'tool_started', call_id: call, name: 'ExitPlanMode', activity: 'plan',
      target: { path: null, paths: [], command: null, query: null, url: null, server: null, tool: 'ExitPlanMode' }, input: { plan }, confidence: 'observed',
    });
    const ap: Approval = {
      id: uid('ap-'), agent_id: a.id, run_id: a.run_id ?? '', task_id: a.task_id, call_id: call,
      subject: { kind: 'tool', name: 'ExitPlanMode', input: { plan } }, reason: null, choices: ['allow_once', 'deny', 'cancel'], risks: [],
      status: 'pending', decided_by: null, created_at: sim.now(),
    };
    sim.approvals.set(ap.id, ap);
    act.emit({ type: 'approval_requested', approval: ap });
    const st = await this.answered(act, ap, 'its plan', 5000);
    const note = this.notes.get(ap.id) ?? '';
    act.emit({
      type: 'tool_completed', call_id: call, ok: st === 'approved', output: st === 'approved' ? 'Plan accepted.' : note || 'Plan not accepted.', exit_code: null,
      duration_ms: sim.now() - ap.created_at, tests_passed: null, tests_failed: null,
    });
    act.status('working', 'thinking');
    if (st === 'cancelled') throw new Cancelled();
    return st === 'approved' ? null : note;
  }

  /**
   * In "ask for edits" a 1:1 Claude agent asks before an edit, with the change it wants to make.
   * Resolves null when you allow it, and your note ("" for none) when you say no. `auto`: as for `answered`.
   */
  private async editApproval(act: Actor, path: string, diff: string, auto: number | null = 3500): Promise<string | null> {
    const sim = this.sim;
    const a = act.a;
    const ap: Approval = {
      id: uid('ap-'), agent_id: a.id, run_id: a.run_id ?? '', task_id: a.task_id, call_id: null,
      subject: { kind: 'file_change', paths: [path], diff }, reason: null, choices: ['allow_once', 'deny', 'cancel'], risks: [],
      status: 'pending', decided_by: null, created_at: sim.now(),
    };
    sim.approvals.set(ap.id, ap);
    act.emit({ type: 'approval_requested', approval: ap });
    const st = await this.answered(act, ap, path.split('/').pop() ?? path, auto);
    act.status('working', 'thinking');
    if (st === 'cancelled') throw new Cancelled();
    return st === 'approved' ? null : (this.notes.get(ap.id) ?? '');
  }

  /**
   * A request of another kind (a tool, more permissions, a question with a free answer): it
   * waits for your answer as the others do. `label`: what the agent's status line says
   * meanwhile. Resolves with the request as you answered it; the caller ends the call it was
   * for, and the turn when you chose "Stop the turn".
   */
  private async request(act: Actor, subject: Approval['subject'], o: { label: string; choices: Approval['choices']; reason?: string; risks?: string[]; callId?: string }): Promise<Approval> {
    const sim = this.sim;
    const a = act.a;
    const ap: Approval = {
      id: uid('ap-'), agent_id: a.id, run_id: a.run_id ?? '', task_id: a.task_id, call_id: o.callId ?? null,
      subject, reason: o.reason ?? null, choices: o.choices, risks: o.risks ?? [], status: 'pending', decided_by: null, created_at: sim.now(),
    };
    sim.approvals.set(ap.id, ap);
    act.emit({ type: 'approval_requested', approval: ap });
    await this.answered(act, ap, o.label, 3500);
    act.status('working', 'thinking');
    return sim.approvals.get(ap.id)!;
  }

  /** The start of a command that waits for your answer before it runs (`ended` is its end). */
  private started(act: Actor, name: string, target: Partial<Target>, input: JsonValue): string {
    const call = uid('call-');
    act.emit({
      type: 'tool_started', call_id: call, name, activity: 'execute', target: { path: null, paths: [], command: null, query: null, url: null, server: null, tool: null, ...target }, input,
      confidence: 'observed',
    });
    return call;
  }

  private ended(act: Actor, call: string, since: number, ok: boolean, output: string, exit: number | null = null) {
    act.emit({ type: 'tool_completed', call_id: call, ok, output, exit_code: exit, duration_ms: this.sim.now() - since, tests_passed: null, tests_failed: null });
  }

  /**
   * `?approval=…` (docs/DEMO.md): a 1:1 agent, asked for something small, asks that kind of
   * request once, and takes your answer as the story's other agents do.
   */
  private seedAsk(kind: Exclude<AskKind, 'plan'>) {
    if (kind === 'file') {
      // A 1:1 Claude agent in "ask for edits" asks before its edit, with the change (`soloWork`).
      this.userChat(`agent:${this.hire('claude', null, 'Docs').id}`, 'Add a short “Theming” section to the README.');
      return;
    }
    for (const who of ASKERS[kind]) {
      const a = this.hire(who.provider, null, who.title, who.name);
      this.scripted.set(a.id, (act) => this.asking(kind, act));
      this.userChat(`agent:${a.id}`, who.text);
    }
  }

  private asking(kind: Exclude<AskKind, 'plan' | 'file'>, act: Actor): Promise<void> {
    const claude = act.a.provider === 'claude';
    if (kind === 'network') return this.askSandboxHost(act);
    if (kind === 'permissions') return this.askPermissions(act);
    return claude ? this.askLongQuestion(act) : this.askFreeAnswer(act);
  }

  /**
   * Claude Code's shell sandbox: a command wants a host that the sandbox does not allow yet,
   * and the CLI asks with a request of its own (`SandboxNetworkAccess`). As in the recording
   * `r4-sandbox-network-allow`, the request has the CLI's question as its reason and an id of
   * its own as its call: it does not say which command it is for. The thread's step is the
   * command; after a "no" it ends with the network error the sandbox gives.
   */
  private async askSandboxHost(act: Actor) {
    const command = 'npm view react version';
    await act.think('The registry knows the newest version: ask npm.', { pace: 40 });
    const since = this.sim.now();
    const call = this.started(act, 'Bash', { command }, { command });
    const ap = await this.request(act, { kind: 'tool', name: 'SandboxNetworkAccess', input: { host: NPM_HOST } }, {
      label: NPM_HOST, choices: ['allow_once', 'deny', 'cancel'], risks: [`network (${NPM_HOST})`],
      reason: `Allow network connection to ${NPM_HOST}?`, callId: uid(),
    });
    const ok = ap.status === 'approved';
    this.ended(act, call, since, ok, ok ? '19.2.3' : NPM_BLOCKED, ok ? 0 : 1);
    if (ap.status === 'cancelled') throw new Cancelled();
    await act.say(ok ? 'The newest react on npm is 19.2.3.' : `The sandbox did not let npm reach ${NPM_HOST}, so I cannot tell from here.`, { thread: `agent:${act.id}` });
  }

  /**
   * Codex asks for more than its sandbox gives it: to write outside its worktree. The detail
   * has the shape of `PermissionsRequestApprovalParams` of codex-cli 0.166.0 (written from its
   * schema types, not from a recording).
   */
  private async askPermissions(act: Actor) {
    const a = act.a;
    const thread = `agent:${act.id}`;
    const docs = `${HOME}/Code/acme-docs`;
    const reason = 'The changelog is in ../acme-docs, outside my worktree.';
    await act.think('The changelog is not in this repo: it is in the docs repo next to it, and I may not write there.', { pace: 40 });
    const detail = {
      threadId: a.session_id ?? '', turnId: uid('turn-'), itemId: uid('item-'), environmentId: null, startedAtMs: this.sim.now(), cwd: a.worktree ?? REPO, reason,
      permissions: { network: null, fileSystem: { read: null, write: [docs] } },
    };
    const ap = await this.request(act, { kind: 'permissions', detail }, { label: 'more permissions', choices: ['allow_once', 'allow_session', 'deny'], reason });
    if (ap.status !== 'approved') {
      await act.say('Understood: I stay inside my worktree. The line for the changelog is: “Added: a dark mode toggle in Settings.”', { thread });
      return;
    }
    await act.tool({ name: 'fileChange', activity: 'edit', target: { path: `${docs}/CHANGELOG.md` }, ms: 1200, files: [`${docs}/CHANGELOG.md`] });
    await act.say('Done: the changelog in `../acme-docs` has a line about the dark mode toggle.', { thread });
  }

  /** Claude Code's AskUserQuestion with long options: the one you pick is the answer the closed row shows. */
  private async askLongQuestion(act: Actor) {
    const [one, two] = LONG_QUESTION.options;
    await act.think('Who reads the release note decides what goes into it: ask before writing.', { pace: 40 });
    const picked = await this.question(act, LONG_QUESTION.question, LONG_QUESTION.header, LONG_QUESTION.options);
    const reply = picked === two.label ? 'Two notes then: a short one for users, and one for developers about the theme tokens.'
      : picked === one.label ? 'One note for everyone then: where the toggle is, the system default, and the saved choice.'
      : 'No answer: I write one short note for everyone, and you can ask for the second one later.';
    await act.say(reply, { thread: `agent:${act.id}` });
  }

  /**
   * A question with no options: an MCP server asks through Codex, and you write the answer.
   * The request has the shape of `McpServerElicitationRequestParams` of codex-cli 0.166.0, mode
   * "form" (written from its schema types, not from a recording).
   */
  private async askFreeAnswer(act: Actor) {
    const message = 'Which team gets the new issue? Write the team’s name, and why it is that team if the checkout is not theirs.';
    await act.think('Linear wants a team for the issue, and I do not know who owns the checkout tests.', { pace: 40 });
    const questions = {
      threadId: act.a.session_id ?? '', turnId: uid('turn-'), serverName: 'linear', mode: 'form', _meta: null, message,
      requestedSchema: { type: 'object', properties: { team: { type: 'string', title: 'Team' } }, required: ['team'] },
    };
    const ap = await this.request(act, { kind: 'question', questions }, { label: 'Linear', choices: ['allow_once', 'deny', 'cancel'], reason: message });
    if (ap.status === 'cancelled') throw new Cancelled();
    const said = (this.answers.get(ap.id) as { answer?: unknown } | undefined)?.answer;
    const team = ap.status === 'approved' && typeof said === 'string' ? said.trim() : '';
    await act.say(team ? `The issue is open in Linear, for the team you named: ${quoted(team, 200)}.` : 'No team, so I did not open an issue.', { thread: `agent:${act.id}` });
  }

  /**
   * A 1:1 Claude agent does what you ask in its mode (its sheet → Controls → Mode): "plan first"
   * shows its plan before it changes anything, "ask for edits" asks before the edit, "accept
   * edits" makes it without asking. Like the plan's workers, it keeps a list of four steps and
   * ticks them off as it goes.
   */
  private async soloWork(act: Actor, ask: string) {
    const sim = this.sim;
    const thread = `agent:${act.id}`;
    const path = 'README.md';
    const steps = [`Read ${path} and package.json`, `Add the note to ${path}`, 'Run the tests', 'Tell you what changed'];
    act.plan(listAt(steps, 0));
    await act.think('Read how the project is set up before changing anything.', { pace: 40 });
    await act.tool({ name: 'Read', activity: 'read', target: { path }, ms: 600 });
    await act.tool({ name: 'Read', activity: 'read', target: { path: 'package.json' }, ms: 500 });
    act.plan(listAt(steps, 1));
    if (act.a.permission_mode === 'plan') {
      // Like Claude Code: a plan you send back is changed and shown again; an accepted plan
      // ends plan mode, and the agent asks for edits from then on.
      for (let note = await this.showPlan(act, soloPlan(ask, null)); note !== null; note = await this.showPlan(act, soloPlan(ask, note))) {
        await act.say(note ? 'I changed the plan as you asked.' : 'The plan stays as it is until you say what to change.', { phase: 'commentary' });
      }
      act.set({ permission_mode: 'ask' });
      sim.decision('permission_mode', 'rule', `Claude Code changed its mode to "${MODE_LABEL.ask}"`, { mode: 'ask' }, { agent: act.id });
    }
    // As Claude Code edits: the README's last line, and that line with the note after it.
    const added = `${README_TAIL}\n\n## Notes\n\n- ${ask.replace(/\s+/g, ' ').slice(0, 200)}`;
    const asks = (act.a.permission_mode ?? 'ask') === 'ask';
    const denied = asks ? await this.editApproval(act, path, editPreview(README_TAIL, added)) : null;
    if (denied !== null) {
      // Your note says what to do instead: the agent takes it up.
      await act.say(denied ? `Understood: ${quoted(denied, 120)}. I will do that.` : 'Understood: I changed nothing. Tell me what to do instead.', { thread });
      return;
    }
    await act.tool({ name: 'Edit', activity: 'edit', target: { path }, input: { file_path: path, old_string: README_TAIL, new_string: added }, ms: 1200, files: [path] });
    act.plan(listAt(steps, 2));
    await act.tool({
      name: 'Bash', activity: 'test', target: { command: 'npm test' }, ms: 2400, passed: 41, failed: 0,
      lines: ['> acme-web@2.4.0 test', '> vitest run', '', ' Test Files  9 passed (9)', '      Tests  41 passed (41)'],
    });
    act.plan(listAt(steps, 3));
    await act.say(`Done: I added a short note to \`${path}\` about ${quoted(ask, 80)}. 41 tests pass.`, { thread });
    act.plan(listAt(steps, 4));
  }

  /**
   * A Claude Code Task-tool helper: a sub-agent with its kind, its own calls (`steps`, over `ms`
   * in all), how far it is, and its report. False: you stopped it from the agent's sheet before it reported.
   */
  private async subagent(act: Actor, description: string, steps: HelperStep[], ms: number, result: string) {
    const sim = this.sim;
    const started = sim.now();
    const background = demoBackground(sim);
    const call = this.helperStarts(act, description, false);
    // Its call waits for it: when you stop the turn meanwhile, the helper ends with it (demo/background.ts `turnStopped`).
    background.waits(act.id, call, description);
    // `?helper=slow`: 20 times as long, so the row can be looked at while the helper works.
    const each = (ms * (this.opts.slowHelper ? 20 : 1)) / steps.length;
    for (const [i, s] of steps.entries()) {
      const own = this.helperCalls(act, call, s, i + 1);
      background.waitedIn(act.id, call, own);
      // In short slices: stopped from the agent's sheet meanwhile, its calls ended with it, and
      // the turn goes on at once (recorded: the agent answers about a second after the stop).
      for (let left = each; left > 0; left -= 250) {
        await act.sleep(Math.min(250, left));
        if (background.waiting(act.id, call) === null) return false;
      }
      act.emit({ type: 'tool_completed', call_id: own, ok: true, output: null, exit_code: null, duration_ms: Math.round(each), tests_passed: null, tests_failed: null });
    }
    background.reported(act.id, call);
    act.emit({ type: 'sub_agent_progress', call_id: call, name: null, step: null, tool_call_id: null, tool_uses: null, tokens: null, report: result });
    act.emit({ type: 'sub_agent', call_id: call, description, status: 'completed' });
    act.emit({ type: 'tool_completed', call_id: call, ok: true, output: result, exit_code: null, duration_ms: sim.now() - started, tests_passed: null, tests_failed: null });
    return true;
  }

  /** A helper begins: its `Agent` call (`background`: with `run_in_background`), its row, and its kind. Returns the call's id. */
  private helperStarts(act: Actor, description: string, background: boolean): string {
    const call = uid('call-');
    act.emit({
      type: 'tool_started', call_id: call, name: 'Task', activity: 'delegate', target: { ...NO_TARGET, tool: 'Task' },
      input: { description, subagent_type: 'Explore', ...(background ? { run_in_background: true } : {}) }, confidence: 'observed',
    });
    act.status('working', 'delegate', 'Explore');
    act.emit({ type: 'sub_agent', call_id: call, description, status: 'running' });
    act.emit({ type: 'sub_agent_progress', call_id: call, name: 'Explore', step: null, tool_call_id: null, tool_uses: null, tokens: null, report: null });
    return call;
  }

  /** A call of the helper's own begins (its `n`th). Returns that call's id. */
  private helperCalls(act: Actor, call: string, s: HelperStep, n: number): string {
    const own = uid('call-');
    act.emit({ type: 'tool_started', call_id: own, name: s.name, activity: s.activity, target: { ...NO_TARGET, ...s.target }, input: {}, confidence: 'observed' });
    // Its `tool_started` came just before: this says that the call is the helper's own.
    act.emit({ type: 'sub_agent_progress', call_id: call, name: null, step: s.step, tool_call_id: own, tool_uses: n, tokens: 2100 * n, report: null });
    return own;
  }

  /**
   * A helper a Claude Code agent starts with `run_in_background`: its `Agent` call returns at
   * once, the CLI lists the helper with the agent's background work, and the helper works on
   * beside the agent's turns and past their end (`steps`, over `ms` in all). It ends with its
   * `report`, or sooner when you stop it from the agent's sheet (demo/background.ts).
   */
  private helperInBackground(act: Actor, description: string, steps: HelperStep[], ms: number, report: string) {
    const sim = this.sim;
    const background = demoBackground(sim);
    const call = this.helperStarts(act, description, true);
    act.emit({ type: 'tool_completed', call_id: call, ok: true, output: 'Async agent launched successfully.', exit_code: null, duration_ms: 0, tests_passed: null, tests_failed: null });
    const id = background.start(act, description, call);
    const each = ms / steps.length;
    void (async () => {
      for (const [i, s] of steps.entries()) {
        const own = this.helperCalls(act, call, s, i + 1);
        background.helperIn(act.id, id, own);
        await sim.sleep(each);
        // Stopped meanwhile, by you or with the agent's process: the call it was in ended with it.
        if (!background.of(act.id).some((t) => t.id === id)) return;
        act.emit({ type: 'tool_completed', call_id: own, ok: true, output: null, exit_code: null, duration_ms: Math.round(each), tests_passed: null, tests_failed: null });
        background.helperIn(act.id, id, null);
      }
      background.end(act.id, id, 'completed', report);
    })();
  }

  /**
   * A command a Claude Code agent starts with `run_in_background`: the call returns at once, and
   * the CLI lists the command until it is over (demo/background.ts). Returns the CLI's id of it.
   */
  private async inBackground(act: Actor, command: string): Promise<string> {
    await act.tool({ name: 'Bash', activity: 'execute', target: { command }, input: { command, run_in_background: true }, ms: 400, output: 'Command running in background' });
    return demoBackground(this.sim).start(act, command);
  }

  /** codly_ask_lead: a worker asks its plan's Lead (in that request's thread) and waits for the answer. */
  private async askLead(asker: Actor, question: string, answer: string) {
    const sim = this.sim;
    const call = uid('call-');
    asker.emit({
      type: 'tool_started', call_id: call, name: 'mcp__codly__codly_ask_lead', activity: 'mcp',
      target: { path: null, paths: [], command: null, query: null, url: null, server: 'codly', tool: 'codly_ask_lead' },
      input: { question }, confidence: 'observed',
    });
    asker.status('working', 'mcp', 'codly_ask_lead');
    const started = sim.now();
    const task = asker.a.task_id ? sim.tasks.get(asker.a.task_id) : null;
    const thread = (task?.plan_id && sim.plans.get(task.plan_id)?.thread) || 'lead';
    sim.chat(thread, asker.id, question, { ask_lead: true });
    await this.withLead(thread, async (lead) => {
      await lead.sleep(700);
      await lead.turn(`${asker.a.name} asks: ${question}`, async () => {
        await lead.think('Answer from the plan and the repo’s conventions. Keep it to one decision.', { pace: 30 });
        await lead.say(answer, { thread, attachment: { reply_to: asker.id } });
      });
    });
    asker.emit({ type: 'tool_completed', call_id: call, ok: true, output: answer, exit_code: null, duration_ms: sim.now() - started, tests_passed: null, tests_failed: null });
  }

  /** `by`: who answered (`AUTOPLAY` for `?autoplay=1`'s answers), as the request's rows say it. */
  resolveApproval(id: string, choice: string, message: string | null, answers: unknown = null, by = 'user') {
    const sim = this.sim;
    const ap = sim.approvals.get(id);
    if (!ap || ap.status !== 'pending') throw new Error('This request was already answered.');
    if (answers != null) this.answers.set(id, answers);
    if (message) this.notes.set(id, message);
    const status = choice === 'deny' || choice === 'cancel' ? (choice === 'cancel' ? 'cancelled' : 'denied') : 'approved';
    sim.approvals.set(id, { ...ap, status, decided_by: by });
    // Like the core: what you said with the answer is a row of its own, just before the answer.
    const said = saidWith(ap, sim.agents.get(ap.agent_id)?.provider, choice, message, answers);
    if (said) sim.emit({ type: 'approval_said', approval_id: id, ...said }, { agent: ap.agent_id, task: ap.task_id, run: ap.run_id });
    sim.emit({ type: 'approval_resolved', approval_id: id, status, by }, { agent: ap.agent_id, task: ap.task_id, run: ap.run_id });
    if (message) sim.decision('approval', 'human', `You denied ${'command' in ap.subject ? '`' + ap.subject.command + '`' : 'the request'}: “${message}”`, { approval: id }, { agent: ap.agent_id, task: ap.task_id });
  }

  private async trackT1(ada: Actor, task: Task) {
    const sim = this.sim;
    const files = TASK_FILES.t1.map((f) => f.path);
    await ada.sleep(900);
    await ada.turn(`Task t1 · ${task.title}\n\n${task.goal}`, async () => {
      ada.plan([['Find where tokens and theme classes live', 'in_progress'], ['Add ThemeProvider with a system default', 'pending'], ['Wire the provider into the root layout', 'pending'], ['Unit tests for resolution and persistence', 'pending']]);
      await ada.think('Tokens already define a dark palette under `.dark`. The provider only has to resolve system/light/dark, set data-theme on <html>, and persist an explicit choice. Check useMediaQuery before writing a new one.');
      ada.retrieval('theme tokens provider root layout media query', [
        cite('C1', 'src/styles/tokens.css', 1, 23, ':root', ['path: tokens', 'text: dark'], 13.1),
        cite('C2', 'src/app/layout.tsx', 1, 22, 'RootLayout', ['symbol: RootLayout', 'text: tokens.css'], 9.4),
        cite('C3', 'src/hooks/useMediaQuery.ts', 1, 19, 'useMediaQuery', ['symbol: useMediaQuery', 'text: prefers color scheme'], 8.2),
      ]);
      const reported = await this.subagent(ada, 'Explore: find every place that reads or switches the theme', [
        { name: 'Grep', activity: 'search', target: { query: 'theme|prefers-color-scheme' }, step: 'Searching for code that reads the theme' },
        { name: 'Read', activity: 'read', target: { path: 'src/styles/tokens.css' }, step: 'Reading src/styles/tokens.css' },
        { name: 'Read', activity: 'read', target: { path: 'src/hooks/useMediaQuery.ts' }, step: 'Reading src/hooks/useMediaQuery.ts' },
      ], 2800, 'Three places: tokens.css defines `.dark` but nothing applies it; useMediaQuery.ts already wraps matchMedia; layout.tsx has no provider.');
      // Stopped from her sheet: the turn that waited for it says so, and goes on (recorded: part 4 section 2).
      if (!reported) await ada.say('The helper was stopped before it reported, so I look for myself.', { phase: 'commentary' });
      await ada.tool({ name: 'Grep', activity: 'search', target: { query: 'data-theme|\\.dark|prefers-color-scheme' }, ms: 900,
        output: 'src/styles/tokens.css:11:.dark {\nsrc/hooks/useMediaQuery.ts:4:  const mql = window.matchMedia(query);' });
      for (const path of ['src/styles/tokens.css', 'src/app/layout.tsx', 'src/hooks/useMediaQuery.ts'])
        await ada.tool({ name: 'Read', activity: 'read', target: { path }, ms: 650 });
      await ada.say('Tokens already have a dark palette under `.dark`; switching them to `[data-theme]` and adding a provider that resolves the system preference.', { phase: 'commentary' });
      ada.plan([['Find where tokens and theme classes live', 'completed'], ['Add ThemeProvider with a system default', 'in_progress'], ['Wire the provider into the root layout', 'pending'], ['Unit tests for resolution and persistence', 'pending']]);
      // A type check she leaves running in the background while she writes: her sheet lists it
      // under "In the background", with Stop, until she stops it before her report.
      const typeCheck = await this.inBackground(ada, 'npx tsc --noEmit --watch');
      await ada.tool({ name: 'Write', activity: 'create', target: { path: files[0] }, ms: 2600, files: [files[0]], taskKey: 't1' });
      await ada.tool({ name: 'Write', activity: 'create', target: { path: files[1] }, ms: 1200, files: [files[1]], taskKey: 't1' });
      await ada.tool({ name: 'Edit', activity: 'edit', target: { path: files[2] }, ms: 1500, files: [files[2]], taskKey: 't1' });
      ada.plan([['Find where tokens and theme classes live', 'completed'], ['Add ThemeProvider with a system default', 'completed'], ['Wire the provider into the root layout', 'in_progress'], ['Unit tests for resolution and persistence', 'pending']]);
      await ada.tool({ name: 'Edit', activity: 'edit', target: { path: files[3] }, ms: 1300, files: [files[3]], taskKey: 't1' });
      sim.triple(`file:${files[3]}`, 'imports', `file:${files[0]}`, 'agent', { agent: ada.id });
      ada.plan([['Find where tokens and theme classes live', 'completed'], ['Add ThemeProvider with a system default', 'completed'], ['Wire the provider into the root layout', 'completed'], ['Unit tests for resolution and persistence', 'in_progress']]);
      await ada.tool({ name: 'Write', activity: 'create', target: { path: files[4] }, ms: 2000, files: [files[4]], taskKey: 't1' });
      // A package download: the card offers "Always allow in this repo", for this command or for `npm install`.
      const installed = await this.approval(ada, 'npm install -D @testing-library/react', 'The provider’s tests render it with React Testing Library, which this repo does not have yet.', [PACKAGE_INSTALL], [
        'added 9 packages, and audited 412 packages in 2s',
      ]);
      if (installed === 'denied') await ada.say('Understood — testing the provider with plain React instead.', { phase: 'commentary' });
      await ada.tool({
        name: 'Bash', activity: 'test', target: { command: 'npm test -- theme' }, ms: 3600, passed: 12, failed: 0,
        lines: ['> acme-web@2.4.0 test', '> vitest run theme', '', ' ✓ src/theme/ThemeProvider.test.tsx (6 tests) 41ms', ' ✓ src/theme/useTheme.test.ts (6 tests) 18ms', '', ' Test Files  2 passed (2)', '      Tests  12 passed (12)', '   Duration  1.21s'],
      });
      ada.plan([['Find where tokens and theme classes live', 'completed'], ['Add ThemeProvider with a system default', 'completed'], ['Wire the provider into the root layout', 'completed'], ['Unit tests for resolution and persistence', 'completed']]);
      await ada.tool({ name: 'Write', activity: 'create', target: { path: 'docs/theme/contrast.svg' }, ms: 1100 });
      sim.artifact({ kind: 'svg', mime: 'image/svg+xml', path: 'docs/theme/contrast.svg', title: 'Token contrast, light vs dark', bytes: bytesOf(contrastSvg()), agent: ada.a, caption: 'WCAG contrast of each text token on its background, in both themes' });
      sim.artifact({ kind: 'csv', mime: 'text/csv', path: 'docs/theme/contrast.csv', title: 'Token contrast data', bytes: bytesOf(contrastCsv()), agent: ada.a, caption: 'The numbers behind the chart' });
      const keep = await this.question(ada, 'Keep the localStorage fallback once t2’s settings API lands?', 'Fallback', [
        { label: 'Keep it', description: 'Instant first paint and offline; the server value wins once loaded (recommended)' },
        { label: 'Remove it', description: 'The server is the only source of truth' },
      ]);
      await ada.say(keep === 'Remove it' ? 'Removing the localStorage fallback; the provider will read the server value only.' : 'Keeping the localStorage fallback; the server value wins once it loads.', { phase: 'commentary' });
      await ada.tool({ name: 'mcp__codly__codly_memory_write', activity: 'memory', target: { server: 'codly', tool: 'codly_memory_write' }, ms: 500 });
      sim.memory('Theme tokens key on `[data-theme]` on <html>. Use `setTheme()` from `useTheme()`; never toggle a `.dark` class.', 'project', 'lesson', ada.id, { importance: 0.75, sourceEvent: sim.lastId, title: 'Theme tokens key on data-theme', refs: ['concept:theming'] });
      sim.triple(`agent:${ada.id}`, 'created', `file:${files[0]}`, 'observed', { agent: ada.id });
      demoBackground(sim).end(ada.id, typeCheck, 'stopped'); // (over already when you stopped it from her sheet)
      await ada.say('Added `ThemeProvider` and `useTheme()`. Tokens now key on `[data-theme]`, the root layout wraps the app, and an explicit choice persists until t2’s API lands. 12 tests pass.');
    });
    await this.gate(ada, task, {
      files, message: 't1: Theme provider and tokens', verify: 'npm test -- theme', tail: ' Test Files  2 passed (2)\n      Tests  12 passed (12)',
      summary: 'ThemeProvider + useTheme(); tokens key on [data-theme]; layout wraps the app. 12 unit tests.', integration: true,
    });
  }

  private async trackT2(bo: Actor, task: Task) {
    const sim = this.sim;
    const files = TASK_FILES.t2.map((f) => f.path);
    const cmd = (command: string, path?: string) => ({ command, path: path ?? null, paths: path ? [path] : [] });
    await bo.sleep(1200);
    await bo.turn(`Task t2 · ${task.title}\n\n${task.goal}`, async () => {
      bo.plan([['Add a theme column and migration', 'in_progress'], ['Expose theme in GET/PATCH /api/settings', 'pending'], ['Build and run the API tests', 'pending']]);
      await bo.think('Prisma enum first so the column is typed, then validate PATCH with zod. A stale dist/ can hide route build errors.');
      await bo.tool({ name: 'commandExecution', activity: 'read', target: cmd("sed -n '1,80p' prisma/schema.prisma", 'prisma/schema.prisma'), ms: 600, confidence: 'inferred', output: 'model UserSettings {\n  id          String   @id @default(cuid())\n  …' });
      await bo.tool({ name: 'commandExecution', activity: 'search', target: { command: 'rg -n "UserSettings|settingsPatch" src/server', query: 'UserSettings|settingsPatch' }, ms: 700, confidence: 'inferred',
        output: 'src/server/settings/route.ts:8:  const settings = await db.userSettings.findUnique({ where: { userId: user.id } });\nsrc/server/settings/schema.ts:3:export const settingsPatch = z' });
      await bo.tool({ name: 'commandExecution', activity: 'read', target: cmd("sed -n '1,60p' src/server/settings/route.ts", 'src/server/settings/route.ts'), ms: 500, confidence: 'inferred' });
      await bo.tool({ name: 'fileChange', activity: 'edit', target: { path: files[0] }, ms: 1300, files: [files[0]], taskKey: 't2' });
      await bo.tool({ name: 'fileChange', activity: 'create', target: { path: files[1] }, ms: 900, files: [files[1]], taskKey: 't2' });
      bo.plan([['Add a theme column and migration', 'completed'], ['Expose theme in GET/PATCH /api/settings', 'in_progress'], ['Build and run the API tests', 'pending']]);
      await bo.tool({ name: 'fileChange', activity: 'edit', target: { path: files[2] }, ms: 900, files: [files[2]], taskKey: 't2' });
      await bo.tool({ name: 'fileChange', activity: 'edit', target: { path: files[3] }, ms: 1600, files: [files[3]], taskKey: 't2' });
      await bo.tool({ name: 'fileChange', activity: 'edit', target: { path: files[4] }, ms: 1200, files: [files[4]], taskKey: 't2' });
      sim.triple(`agent:${bo.id}`, 'edited', `file:${files[0]}`, 'observed', { agent: bo.id });
      sim.triple('concept:UserSettings.theme', 'stored_in', `file:${files[0]}`, 'agent', { agent: bo.id });
      bo.plan([['Add a theme column and migration', 'completed'], ['Expose theme in GET/PATCH /api/settings', 'completed'], ['Build and run the API tests', 'in_progress']]);
      await bo.say('Schema, migration and route are done. Cleaning `dist/` before the build so stale route bundles can’t hide errors.', { phase: 'commentary' });
      const answer = await this.approval(bo, 'rm -rf dist && npm run build', 'Clean stale build output so old route bundles can’t mask type errors in the new handler.', [
        'deletes files (rm -rf)', 'stays inside the worktree', 'no network',
      ], BUILD_OUTPUT);
      if (answer === 'denied') {
        await bo.say('Understood — building without cleaning `dist/`.', { phase: 'commentary' });
        await bo.tool({ name: 'commandExecution', activity: 'execute', target: { command: 'npm run build' }, ms: 4200,
          lines: ['> acme-web@2.4.0 build', '> next build', '   ▲ Next.js 16.1.2', ' ✓ Compiled successfully in 8.7s', ' ✓ Generating static pages (14/14)'] });
      }
      await bo.tool({
        name: 'commandExecution', activity: 'test', target: { command: 'npm test -- settings' }, ms: 3000, passed: 8, failed: 0, confidence: 'inferred',
        lines: ['> acme-web@2.4.0 test', '> vitest run settings', '', ' ✓ src/server/settings/route.test.ts (4 tests) 63ms', ' ✓ src/server/settings/schema.test.ts (4 tests) 9ms', '', ' Test Files  2 passed (2)', '      Tests  8 passed (8)'],
      });
      bo.plan([['Add a theme column and migration', 'completed'], ['Expose theme in GET/PATCH /api/settings', 'completed'], ['Build and run the API tests', 'completed']]);
      await bo.tool({ name: 'codly.codly_post_note', activity: 'note', target: { server: 'codly', tool: 'codly_post_note' }, ms: 500 });
      sim.memory('Settings API: `theme` is `system | light | dark` (default `system`). PATCH /api/settings accepts `{ theme }` and returns the full settings object.', 'task', 'note', bo.id, { scopeId: task.id, importance: 0.8, sourceEvent: sim.lastId, title: 'Settings API contract', refs: [`task:${task.id}`, 'concept:theming'] });
      await bo.tool({ name: 'fileChange', activity: 'create', target: { path: 'docs/api/settings.md' }, ms: 900 });
      sim.artifact({ kind: 'markdown', mime: 'text/markdown', path: 'docs/api/settings.md', title: 'Settings API', bytes: bytesOf(SETTINGS_API_MD), agent: bo.a, caption: 'GET and PATCH /api/settings, with the new theme field' });
      await bo.say('Added a `Theme` enum and a `theme` column (default `system`) with a migration; `PATCH /api/settings` validates it with zod and both handlers return it. Build is clean, 8 API tests pass.');
    });
    await this.gate(bo, task, {
      files, message: 't2: Persist the theme preference', verify: 'npm run build && npm test -- settings', tail: ' ✓ Compiled successfully\n      Tests  8 passed (8)',
      summary: 'theme enum + column (default system) with a migration; validated in PATCH /api/settings. 8 API tests.', integration: true,
    });
  }

  private async trackT3(cy: Actor, task: Task) {
    const sim = this.sim;
    const files = TASK_FILES.t3.map((f) => f.path);
    await cy.sleep(1000);
    await cy.turn(`Task t3 · ${task.title}\n\n${task.goal}`, async () => {
      cy.plan([['Read useTheme() and the settings API', 'in_progress'], ['Add ThemeToggle to /settings', 'pending'], ['Playwright test for the three states', 'pending']]);
      await cy.think('useTheme() gives theme and setTheme; the API takes { theme }. A radiogroup keeps it accessible. The e2e test should cover a reload — that is where races hide.');
      cy.retrieval('settings page theme toggle useTheme PATCH settings', [
        cite('C1', 'src/theme/useTheme.ts', 1, 9, 'useTheme', ['symbol: useTheme', 'path: theme'], 12.8),
        cite('C2', 'src/app/settings/page.tsx', 1, 21, 'SettingsPage', ['path: settings', 'symbol: SettingsPage'], 11.2),
        cite('C3', 'src/server/settings/route.ts', 7, 27, 'PATCH', ['symbol: PATCH', 'text: theme'], 9.9),
      ]);
      await cy.tool({ name: 'read_file', activity: 'read', target: { path: 'src/theme/useTheme.ts' }, ms: 600 });
      await cy.tool({ name: 'read_file', activity: 'read', target: { path: 'src/app/settings/page.tsx' }, ms: 600 });
      await cy.tool({ name: 'read_file', activity: 'read', target: { path: 'src/app/settings/LocaleSelect.tsx' }, ms: 500 });
      await this.askLead(cy, 'Should the theme toggle sit above or below Language on /settings?',
        'Above Language: Appearance is the first group on the settings page, so the toggle stays visible without scrolling.');
      cy.plan([['Read useTheme() and the settings API', 'completed'], ['Add ThemeToggle to /settings', 'in_progress'], ['Playwright test for the three states', 'pending']]);
      await cy.tool({ name: 'write_file', activity: 'create', target: { path: files[0] }, ms: 2400, files: [files[0]], taskKey: 't3' });
      await cy.tool({ name: 'replace', activity: 'edit', target: { path: files[1] }, ms: 1100, files: [files[1]], taskKey: 't3' });
      sim.triple(`file:${files[0]}`, 'calls', 'file:src/server/settings/route.ts', 'agent', { agent: cy.id });
      await cy.tool({ name: 'generate_image', activity: 'create', target: { path: 'public/illustrations/appearance.png' }, ms: 2600, input: { prompt: ILLUSTRATION_PROMPT, size: '960x600' } });
      const png = await illustrationPng();
      sim.artifact({ kind: 'image', mime: 'image/png', path: 'public/illustrations/appearance.png', title: 'Appearance illustration', bytes: png, agent: cy.a, caption: ILLUSTRATION_PROMPT });
      await cy.tool({ name: 'write_file', activity: 'create', target: { path: 'artifacts/theme-toggle-preview.html' }, ms: 1200 });
      sim.artifact({ kind: 'html', mime: 'text/html', path: 'artifacts/theme-toggle-preview.html', title: 'ThemeToggle preview', bytes: bytesOf(TOGGLE_PREVIEW_HTML), agent: cy.a, caption: 'The three states of the toggle, rendered with the real tokens' });
      cy.plan([['Read useTheme() and the settings API', 'completed'], ['Add ThemeToggle to /settings', 'completed'], ['Playwright test for the three states', 'in_progress']]);
      await cy.tool({ name: 'write_file', activity: 'create', target: { path: files[2] }, ms: 1800, files: [files[2]], taskKey: 't3' });
      // Another package download: a rule you saved for `npm install` on Ada's card answers it.
      const axe = await this.approval(cy, 'npm install -D @axe-core/playwright', 'The e2e test checks the toggle for accessibility with axe, which this repo does not have yet.', [PACKAGE_INSTALL], [
        'added 3 packages, and audited 415 packages in 1s',
      ]);
      if (axe === 'denied') await cy.say('Understood — the e2e test checks the three states without axe.', { phase: 'commentary' });
      await cy.tool({
        name: 'run_shell_command', activity: 'test', target: { command: 'npx playwright test settings-theme' }, ms: 4200, ok: false, exit: 1, passed: 2, failed: 1,
        lines: ['Running 3 tests using 3 workers', '  ✓  1 settings · theme › defaults to the system theme (812ms)', '  ✓  2 settings · theme › switching applies immediately (904ms)',
          '  ✘  3 settings · theme › dark mode persists after reload (1.4s)', '', '  1) settings · theme › dark mode persists after reload', '     Error: expect(locator).toHaveAttribute(expected)',
          '     Expected: "dark"', '     Received: "light"', '       at tests/e2e/settings-theme.spec.ts:19:42', '', '  1 failed', '  2 passed (3.1s)'],
      });
      cy.emit({ type: 'error', message: 'e2e: dark mode persists after reload — expected "dark", received "light"', fatal: false });
      await cy.think('The reload happens before the PATCH resolves, so the server still says “system” and the light OS theme wins. Await the write in the toggle and disable it while saving.');
      await cy.tool({ name: 'replace', activity: 'edit', target: { path: files[0] }, ms: 1300, files: [files[0]], taskKey: 't3' });
      await cy.tool({
        name: 'run_shell_command', activity: 'test', target: { command: 'npx playwright test settings-theme' }, ms: 4000, passed: 3, failed: 0,
        lines: ['Running 3 tests using 3 workers', '  ✓  1 settings · theme › defaults to the system theme (790ms)', '  ✓  2 settings · theme › switching applies immediately (861ms)',
          '  ✓  3 settings · theme › dark mode persists after reload (1.6s)', '', '  3 passed (2.9s)'],
      });
      cy.plan([['Read useTheme() and the settings API', 'completed'], ['Add ThemeToggle to /settings', 'completed'], ['Playwright test for the three states', 'completed']]);
      sim.triple(`agent:${cy.id}`, 'created', `file:${files[2]}`, 'observed', { agent: cy.id });
      sim.memory('Await settings writes before resolving UI state; Playwright reloads expose the race.', 'project', 'lesson', cy.id, { importance: 0.7, sourceEvent: sim.lastId });
      await cy.say('Added a System / Light / Dark toggle to Settings, wired to `useTheme()` and `PATCH /api/settings`. The e2e test caught a reload race — the toggle now awaits the write. 3 Playwright tests pass.');
    });
    await this.gate(cy, task, {
      files, message: 't3: Settings toggle and e2e test', verify: 'npx playwright test settings-theme', tail: '  3 passed (2.9s)',
      summary: 'System / Light / Dark toggle on Settings, awaits PATCH (fixed a reload race). 3 e2e tests.', integration: true,
    });
  }

  private async trackL1(fay: Actor, task: Task) {
    const sim = this.sim;
    const files = TASK_FILES.l1.map((f) => f.path);
    await fay.sleep(1000);
    await fay.turn(`Task t1 · ${task.title}\n\n${task.goal}`, async () => {
      fay.plan([['Read signIn() and the limiter', 'in_progress'], ['Count failures and lock the email', 'pending'], ['Unit tests for lock and expiry', 'pending']]);
      await fay.think('Reuse the Redis fixed window from hit(): one key per email, TTL = the window. signIn() checks the lock before touching the database.');
      await fay.tool({ name: 'commandExecution', activity: 'read', target: { command: "sed -n '1,60p' src/server/auth.ts", path: 'src/server/auth.ts', paths: ['src/server/auth.ts'] }, ms: 600, confidence: 'inferred' });
      await fay.tool({ name: 'commandExecution', activity: 'read', target: { command: "sed -n '1,40p' src/server/rate-limit.ts", path: 'src/server/rate-limit.ts', paths: ['src/server/rate-limit.ts'] }, ms: 500, confidence: 'inferred' });
      fay.plan([['Read signIn() and the limiter', 'completed'], ['Count failures and lock the email', 'in_progress'], ['Unit tests for lock and expiry', 'pending']]);
      await fay.tool({ name: 'fileChange', activity: 'edit', target: { path: files[0] }, ms: 1800, files: [files[0]], taskKey: 'l1' });
      await fay.tool({ name: 'fileChange', activity: 'edit', target: { path: files[1] }, ms: 1500, files: [files[1]], taskKey: 'l1' });
      sim.triple(`file:${files[1]}`, 'calls', `file:${files[0]}`, 'agent', { agent: fay.id });
      fay.plan([['Read signIn() and the limiter', 'completed'], ['Count failures and lock the email', 'completed'], ['Unit tests for lock and expiry', 'in_progress']]);
      await fay.tool({ name: 'fileChange', activity: 'edit', target: { path: files[2] }, ms: 1400, files: [files[2]], taskKey: 'l1' });
      await fay.say('Lockout logic is in. Clearing the local test Redis database first so stale counters from earlier runs can’t make the tests pass by accident.', { phase: 'commentary' });
      await this.approval(fay, 'redis-cli -n 9 FLUSHDB', 'Stale sign-in counters in the local test database would make the lockout tests pass for the wrong reason.', [
        'deletes data (local Redis db 9)', 'touches a local service', 'no network',
      ], ['OK']);
      await fay.tool({
        name: 'commandExecution', activity: 'test', target: { command: 'npm test -- rate-limit' }, ms: 2600, passed: 5, failed: 0, confidence: 'inferred',
        lines: ['> acme-web@2.4.0 test', '> vitest run rate-limit', '', ' ✓ src/server/rate-limit.test.ts (5 tests) 38ms', '', ' Test Files  1 passed (1)', '      Tests  5 passed (5)'],
      });
      fay.plan([['Read signIn() and the limiter', 'completed'], ['Count failures and lock the email', 'completed'], ['Unit tests for lock and expiry', 'completed']]);
      sim.memory('Sign-in lockout: `signIn()` answers `{ error: "LOCKED", retryInSec }` after 5 failures in 15 minutes (per email, Redis).', 'project', 'fact', fay.id, { importance: 0.75, sourceEvent: sim.lastId });
      await fay.say('`signIn()` now answers `LOCKED` with `retryInSec` after 5 failures within 15 minutes; a success clears the count. 5 unit tests pass.');
    });
    await this.gate(fay, task, {
      files, message: 't1: Count failed sign-ins and lock the email', verify: 'npm test -- rate-limit', tail: '      Tests  5 passed (5)',
      summary: 'Failures counted per email in Redis; 5 in 15 min → LOCKED with retryInSec. 5 unit tests.', integration: true,
    });
  }

  private async trackL2(dee: Actor, task: Task) {
    const sim = this.sim;
    const files = TASK_FILES.l2.map((f) => f.path);
    await dee.sleep(900);
    await dee.turn(`Task t2 · ${task.title}\n\n${task.goal}`, async () => {
      dee.plan([['Read the sign-in form', 'in_progress'], ['Show the lockout message', 'pending'], ['Playwright test', 'pending']]);
      await dee.think('signIn() now returns LOCKED with retryInSec. Round up to minutes, disable the form while locked, and point to reset.');
      await dee.tool({ name: 'read_file', activity: 'read', target: { path: files[0] }, ms: 600 });
      await dee.tool({ name: 'read_file', activity: 'read', target: { path: 'src/server/auth.ts' }, ms: 500 });
      dee.plan([['Read the sign-in form', 'completed'], ['Show the lockout message', 'in_progress'], ['Playwright test', 'pending']]);
      await dee.tool({ name: 'replace', activity: 'edit', target: { path: files[0] }, ms: 1700, files: [files[0]], taskKey: 'l2' });
      dee.plan([['Read the sign-in form', 'completed'], ['Show the lockout message', 'completed'], ['Playwright test', 'in_progress']]);
      await dee.tool({ name: 'write_file', activity: 'create', target: { path: files[1] }, ms: 1500, files: [files[1]], taskKey: 'l2' });
      await dee.tool({
        name: 'run_shell_command', activity: 'test', target: { command: 'npx playwright test auth-lockout' }, ms: 3400, passed: 2, failed: 0,
        lines: ['Running 2 tests using 2 workers', '  ✓  1 five wrong passwords lock the account for 15 minutes (1.8s)', '  ✓  2 a correct password before the limit still works (744ms)', '', '  2 passed (2.6s)'],
      });
      dee.plan([['Read the sign-in form', 'completed'], ['Show the lockout message', 'completed'], ['Playwright test', 'completed']]);
      await dee.say('The sign-in page now says “Too many attempts. Try again in N minutes”, disables the form while locked and links to password reset. 2 Playwright tests pass.');
    });
    await this.gate(dee, task, {
      files, message: 't2: Lockout message on the sign-in page', verify: 'npx playwright test auth-lockout', tail: '  2 passed (2.6s)',
      summary: 'Lockout message with minutes left; form disabled while locked; reset link. 2 e2e tests.', integration: true,
    });
  }

  /** billing-api, t1. Otto's request is the one that waits for you there: it is asked as the task starts, with no pause before it. */
  private async trackB1(otto: Actor, task: Task) {
    const files = BILLING_FILES.b1.map((f) => f.path);
    const steps = ['Read how an invoice is charged', 'Add the retry schedule', 'Unit tests for each retry'];
    const at = (n: number) => steps.map((text, i): [string, 'pending' | 'in_progress' | 'completed'] => [text, i < n ? 'completed' : i === n ? 'in_progress' : 'pending']);
    await otto.turn(`Task t1 · ${task.title}\n\n${task.goal}`, async () => {
      otto.plan(at(1));
      const up = await this.approval(otto, 'docker compose up -d stripe-mock', 'The retry tests charge a card: they need the local Stripe mock to answer.', [
        'starts a local service', 'network (pulls an image)',
      ], [' ✔ Container billing-api-stripe-mock-1  Started']);
      if (up === 'denied') await otto.say('Understood: the tests use a stub of the Stripe client, not the mock server.', { phase: 'commentary' });
      await otto.tool({ name: 'fileChange', activity: 'create', target: { path: files[0] }, ms: 1800, files: [files[0]], taskKey: 'b1' });
      otto.plan(at(2));
      await otto.tool({ name: 'fileChange', activity: 'create', target: { path: files[1] }, ms: 1300, files: [files[1]], taskKey: 'b1' });
      await otto.tool({
        name: 'commandExecution', activity: 'test', target: { command: 'npm test -- retry' }, ms: 2400, passed: 2, failed: 0, confidence: 'inferred',
        lines: ['> billing-api@1.9.0 test', '> vitest run retry', '', ' ✓ src/invoices/retry.test.ts (2 tests) 12ms', '', ' Test Files  1 passed (1)', '      Tests  2 passed (2)'],
      });
      otto.plan(at(3));
      await otto.say('A failed payment is now tried again one day later, three times at most; after the last try the invoice is `unpaid`. 2 unit tests pass.');
    });
    await this.gate(otto, task, {
      files, message: 't1: Retry schedule for failed payments', verify: 'npm test -- retry', tail: '      Tests  2 passed (2)',
      summary: 'A failed payment is tried again one day later, three times at most, then the invoice is unpaid. 2 unit tests.', integration: true,
    });
  }

  /** billing-api, t2. */
  private async trackB2(rex: Actor, task: Task) {
    const files = BILLING_FILES.b2.map((f) => f.path);
    await rex.sleep(900);
    await rex.turn(`Task t2 · ${task.title}\n\n${task.goal}`, async () => {
      rex.plan([['Read how the other emails are written', 'in_progress'], ['Write the “payment failed” email', 'pending'], ['Unit test for its text', 'pending']]);
      await rex.think('The other emails are plain functions that return `to`, `subject` and `text`. This one needs the invoice number and the amount.');
      await rex.tool({ name: 'Glob', activity: 'search', target: { query: 'src/mail/*.ts' }, ms: 700, output: 'src/mail/send.ts\nsrc/mail/receipt.ts\nsrc/mail/welcome.ts' });
      await rex.tool({ name: 'Read', activity: 'read', target: { path: 'src/mail/receipt.ts' }, ms: 650 });
      rex.plan([['Read how the other emails are written', 'completed'], ['Write the “payment failed” email', 'in_progress'], ['Unit test for its text', 'pending']]);
      await rex.tool({ name: 'Write', activity: 'create', target: { path: files[0] }, ms: 2200, files: [files[0]], taskKey: 'b2' });
      rex.plan([['Read how the other emails are written', 'completed'], ['Write the “payment failed” email', 'completed'], ['Unit test for its text', 'in_progress']]);
      await rex.tool({ name: 'Write', activity: 'create', target: { path: files[1] }, ms: 1500, files: [files[1]], taskKey: 'b2' });
      await rex.tool({
        name: 'Bash', activity: 'test', target: { command: 'npm test -- mail' }, ms: 2600, passed: 1, failed: 0,
        lines: ['> billing-api@1.9.0 test', '> vitest run mail', '', ' ✓ src/mail/paymentFailed.test.ts (1 test) 9ms', '', ' Test Files  1 passed (1)', '      Tests  1 passed (1)'],
      });
      rex.plan([['Read how the other emails are written', 'completed'], ['Write the “payment failed” email', 'completed'], ['Unit test for its text', 'completed']]);
      await rex.say('Added the “payment failed” email: it names the invoice and the amount, and asks the customer to update the card. 1 unit test passes.');
    });
    await this.gate(rex, task, {
      files, message: 't2: Email after the last failed try', verify: 'npm test -- mail', tail: '      Tests  1 passed (1)',
      summary: 'The “payment failed” email, with the invoice number and the amount. 1 unit test.', integration: true,
    });
  }

  /** Starts each task once what it waits on is merged; when all are, integrates and tells its Lead. */
  private async coordinate(planId: string) {
    const sim = this.sim;
    const spec = this.specOfPlan.get(planId)!;
    const cancelled = () => sim.plans.get(planId)?.status === 'cancelled';
    const tasks = () => [...sim.tasks.values()].filter((t) => t.plan_id === planId);
    const started = new Set<string>();
    await sim.sleep(700);
    while (tasks().some((t) => t.status !== 'done')) {
      if (cancelled()) return;
      for (const t of tasks()) this.sayWait(t.id);
      for (const { id } of tasks().sort((a, b) => a.key.localeCompare(b.key, undefined, { numeric: true }))) {
        // As it is now: this pass waits between two tasks, and you can cancel one meanwhile.
        const t = sim.tasks.get(id);
        if (!t || started.has(t.id) || t.status === 'done' || t.status === 'cancelled') continue;
        if (t.depends_on.some((d) => sim.tasks.get(d)?.status !== 'done')) continue;
        started.add(t.id);
        // At work already (billing-api's tasks are, when you open it): not started a second time.
        if (t.status !== 'pending' && t.status !== 'ready') continue;
        if (t.status === 'pending') {
          sim.patchTask(t.id, { status: 'ready', status_reason: 'Dependencies merged; starting' });
          await sim.sleep(900);
        }
        if (this.held) await this.notHeld(t.id);
        // Cancelled, or started by you, while it waited: it is not started here (Retry starts it, `moveTask`).
        if (sim.tasks.get(t.id)?.status !== 'ready') continue;
        this.startPlanned(planId, t.key);
        await sim.sleep(500);
      }
      await sim.sleep(400);
    }
    sim.patchPlan(planId, { status: 'integrating' });
    await sim.sleep(1500);
    const plan = sim.plans.get(planId)!;
    const all = tasks().flatMap((t) => this.taskFiles(t));
    const s = stats(all);
    const n = tasks().length;
    sim.decision('plan_done', 'rule', `All ${n} tasks merged into ${plan.integration_branch}: ${s.files} files, +${s.added} −${s.removed}`, { plan_id: planId, ...s });
    sim.patchPlan(planId, { status: 'done' });
    if (!sim.settings.lead_followups) return;
    const text = spec === 'theme'
      ? `All three tasks are merged into \`${plan.integration_branch}\` — ${s.files} files, +${s.added} −${s.removed}.\n\n` +
        '- **t1** `ThemeProvider` + `useTheme()`; tokens key on `[data-theme]` · 12 unit tests\n' +
        '- **t2** `theme` column with a migration, validated in `PATCH /api/settings` · 8 API tests\n' +
        '- **t3** System / Light / Dark toggle on Settings · 3 e2e tests (Cy caught and fixed a reload race)\n\n' +
        '`main` is untouched. Review the diff, then merge when you’re ready.'
      : spec === 'cards'
      ? `All four tasks are merged into \`${plan.integration_branch}\` — ${s.files} files, +${s.added} −${s.removed}.\n\n` +
        '- **t1** lint-staged runs on the staged files before each commit\n' +
        '- **t2** the environment variables are in `docs/env.md`\n' +
        '- **t3** the licence names its years\n' +
        '- **t4** `CHANGELOG.md` starts with the 2.5.0 section\n\n' +
        '`main` is untouched until you merge.'
      : spec === 'retries'
      ? `Both tasks are merged into \`${plan.integration_branch}\` — ${s.files} files, +${s.added} −${s.removed}.\n\n` +
        '- **t1** a failed payment is tried again one day later, three times at most · 2 unit tests\n' +
        '- **t2** the “payment failed” email, with the invoice number and the amount · 1 unit test\n\n' +
        '`main` is untouched until you merge.'
      : `Both tasks are merged into \`${plan.integration_branch}\` — ${s.files} files, +${s.added} −${s.removed}.\n\n` +
        '- **t1** failures counted per email in Redis; 5 in 15 minutes → `LOCKED` · 5 unit tests\n' +
        '- **t2** clear lockout message on `/sign-in`, form disabled while locked · 2 e2e tests\n\n' +
        'It merges cleanly next to the dark-mode work. `main` is untouched until you merge.';
    await this.withLead(plan.thread, async (lead) => {
      await lead.turn(`Plan finished: ${n} of ${n} tasks merged.`, async () => {
        await lead.think('Summarize what landed, with evidence (tests), and hand the merge decision back to the user.', { pace: 40 });
        await lead.say(text, { thread: plan.thread, attachment: { plan_id: planId } });
      });
    });
  }

  /** Like the core: live plans cancelled, the thread's messages, plans and Lead leave it. */
  closeThread(thread: string) {
    const sim = this.sim;
    const archive = `closed:${thread}:${sim.now()}`;
    for (const p of [...sim.plans.values()].filter((p) => (p.thread || 'lead') === thread)) {
      if (p.status !== 'done' && p.status !== 'cancelled' && p.status !== 'failed') this.cancelPlan(p.id);
      sim.patchPlan(p.id, { thread: archive });
    }
    for (const a of [...sim.agents.values()]) if (a.role === 'lead' && (a.thread ?? 'lead') === thread) sim.agents.delete(a.id);
    sim.chats = sim.chats.filter((c) => c.thread !== thread);
    this.unread.delete(thread);
  }

  cancelPlan(planId: string) {
    const sim = this.sim;
    const plan = sim.plans.get(planId);
    if (!plan) throw new Error('Unknown plan');
    if (plan.status === 'done' || plan.status === 'cancelled') throw new Error(`The plan is already ${plan.status}.`);
    sim.patchPlan(planId, { status: 'cancelled' });
    for (const t of sim.tasks.values()) {
      if (t.plan_id !== planId || t.status === 'done') continue;
      if (t.assignee && this.running.has(t.assignee)) sim.cancelled.add(t.assignee);
      sim.patchTask(t.id, { status: 'cancelled', status_reason: 'Plan cancelled by you' });
    }
    sim.decision('plan_cancelled', 'human', 'You cancelled the plan; finished work stays on the integration branch', { plan_id: planId });
  }

  /**
   * Before a plan is merged, in the core's words and order (orchestrator/workspace.rs
   * `merge_plan`): a running plan is not merged, nor one that never ran. A cancelled plan
   * merges the work it finished.
   */
  mergeable(planId: string): Plan {
    const plan = this.sim.plans.get(planId);
    if (!plan) throw new Error('Unknown plan');
    if (plan.status === 'running' || plan.status === 'integrating') throw new Error('the plan is still running; wait until it is done (or cancel it)');
    if (!plan.integration_branch) throw new Error('the plan never ran, so there is nothing to merge');
    return plan;
  }

  mergePlan(planId: string): string {
    const sim = this.sim;
    const plan = this.mergeable(planId);
    const head = sha();
    if (sim.workspace) sim.workspace = { ...sim.workspace, head };
    sim.patchPlan(planId, { merged_sha: head });
    sim.decision('merge_plan', 'human', `You merged ${plan.integration_branch} into ${sim.workspace?.current_branch ?? 'main'} @ ${head.slice(0, 7)}`, { plan_id: planId, sha: head });
    return head;
  }

  /**
   * `?merge=conflict`: the files of the plan's finished work that your branch changed too. In
   * the demo that is the last file the plan edited (a file the plan created cannot conflict).
   */
  conflictFiles(planId: string): string[] {
    const done = [...this.sim.tasks.values()].filter((t) => t.plan_id === planId && t.status === 'done');
    const edited = done.flatMap((t) => this.taskFiles(t)).filter((f) => f.before !== null);
    return edited.slice(-1).map((f) => f.path);
  }

  /**
   * "Have an agent resolve it". Like the core: one more task of the plan resolves the conflict
   * with `target` (your branch) in the plan's own worktree, the plan runs again, and it is
   * done again when that task passed the plan's checks. Refused in the core's words.
   */
  resolveConflict(planId: string, target: string, files: string[]): Task {
    const sim = this.sim;
    const plan = this.resolvable(planId, false);
    const branch = plan.integration_branch!;
    const names = files.join(', ');
    const task = this.mergeTask(plan, 'resolve', {
      title: `Resolve conflicts with ${target}`,
      goal: `The plan is finished, but ${target} (your branch) changed the same lines meanwhile. Codly started merging ${target} into the plan’s worktree, and these files have conflict markers: ${names}. Resolve every conflict so that the plan’s work and the changes on ${target} are both kept, and remove all markers.`,
      acceptance: ['No conflict marker (<<<<<<<, =======, >>>>>>>) is left in any file', `The plan’s work and the changes on ${target} are both kept and work together`],
      write_scope: files,
    });
    sim.decision('merge', 'human', `you had an agent resolve the conflicts of ${branch} with ${target} in ${names}: task ${task.key}`, { plan_id: planId, files }, { task: task.id });
    sim.patchPlan(planId, { status: 'running', base_branch: target });
    sim.chat(plan.thread, 'codly', `${target} and this plan both changed ${names}. Codly brought ${target} into the plan’s branch, in its own worktree (your checkout is not touched). An agent resolves the conflicts there: that is task ${task.key}. Then the plan’s checks run again and you can merge it.`);
    void this.runResolution(planId, task.id, target, null);
    return task;
  }

  /**
   * `?merge=checks`: the checks of the plan's finished work that fail on its merge with your
   * branch. In the demo that is the last check of its finished tasks that `MERGE_FAILURES`
   * knows why it fails; none, when it knows none of them.
   */
  failingChecks(planId: string): string[] {
    const done = [...this.sim.tasks.values()].filter((t) => t.plan_id === planId && t.status === 'done');
    return done.flatMap((t) => t.verify).filter((c) => c in MERGE_FAILURES).slice(-1);
  }

  /**
   * "Ask an agent to fix it": the plan's checks failed on its merge with `target` (your
   * branch), with no conflict. Like the core (merge_conflict.rs, `fix_task`): Codly brings
   * `target` into the plan's own worktree, one more task of the plan makes the checks pass
   * there, the plan runs again, and it is done again when that task passed the plan's checks.
   */
  fixChecks(planId: string, target: string, failed: string[]): Task {
    const sim = this.sim;
    const plan = this.resolvable(planId, true);
    const branch = plan.integration_branch!;
    const list = failed.map((c) => `\`${c}\``).join(', ');
    // Its write scope is the files the plan changed.
    const done = [...sim.tasks.values()].filter((t) => t.plan_id === planId && t.status === 'done');
    const scope = [...new Set(done.flatMap((t) => this.taskFiles(t).map((f) => f.path)))];
    const task = this.mergeTask(plan, 'fix-checks', {
      title: `Fix the checks with ${target} merged in`,
      goal: `The plan is finished, and ${target} (your branch) got new commits meanwhile. The two merge without a conflict, but on the result of that merge these checks fail: ${list}. Codly merged ${target} into this worktree, which holds the plan’s finished work. Find out why the checks fail now, and fix it so that the plan’s work and the changes on ${target} are both kept and work together.`,
      acceptance: [...failed.map((c) => `\`${c}\` passes`), `The plan’s work and the changes on ${target} are both kept and work together`],
      write_scope: scope,
    });
    sim.decision('merge', 'human', `you had an agent fix ${list} on ${branch} with ${target} merged in: task ${task.key}`, { plan_id: planId, failed_checks: failed }, { task: task.id });
    sim.patchPlan(planId, { status: 'running', base_branch: target });
    sim.chat(plan.thread, 'codly', `${target} and this plan merge without a conflict, but ${list} failed on the result. Codly brought ${target} into the plan’s branch, in its own worktree (your checkout is not touched), and task ${task.key} makes the checks pass there. Then the plan’s checks run again and you can merge it.`);
    void this.runResolution(planId, task.id, target, failed);
    return task;
  }

  /** The plan, when an agent can take the work its merge left (`fixChecks`: a fix of its checks, else a conflict). Refused in the core's words. */
  private resolvable(planId: string, fixChecks: boolean): Plan {
    const plan = this.sim.plans.get(planId);
    if (!plan) throw new Error('Unknown plan');
    if (plan.merged_sha) throw new Error('this plan is already merged into your branch');
    if (plan.status === 'running' || plan.status === 'integrating') throw new Error('the plan is still running; wait until it is done');
    if (plan.status === 'cancelled') throw new Error(`this plan was cancelled, so no agent works on it any more; ${fixChecks ? 'fix the checks' : 'resolve the conflict'} yourself`);
    if (!plan.integration_branch || (plan.status !== 'done' && plan.status !== 'failed')) throw new Error('the plan never ran, so there is nothing to merge');
    return plan;
  }

  /** One more task of a finished plan, on the plan's own branch and in its worktree, with the checks of its finished tasks. */
  private mergeTask(plan: Plan, stem: string, o: Pick<Task, 'title' | 'goal' | 'acceptance' | 'write_scope'>): Task {
    const sim = this.sim;
    const tasks = [...sim.tasks.values()].filter((t) => t.plan_id === plan.id);
    const now = sim.now();
    return sim.putTask({
      id: uid('t-'), plan_id: plan.id, key: nextKey(tasks, stem), ...o,
      verify: [...new Set(tasks.filter((t) => t.status === 'done').flatMap((t) => t.verify))], depends_on: [], status: 'ready',
      assignee: null, agent_hint: null, account_hint: null, consultant: null, fresh_session: true, branch: plan.integration_branch, worktree: plan.integration_worktree, attempt: 0,
      status_reason: 'Ready: a free agent picks it up', summary: null, created_at: now, updated_at: now,
    });
  }

  /** A free worker takes the task that resolves the conflict (or that makes the `failed` checks pass); then the plan is done again, finished or not. */
  private async runResolution(planId: string, taskId: string, target: string, failed: string[] | null) {
    const fix = failed !== null;
    const sim = this.sim;
    await sim.sleep(900);
    await this.notHeld(taskId);
    const task = sim.tasks.get(taskId)!;
    if (task.status === 'ready') {
      // First a worker that knows the plan's work, then any free one.
      const free = (a: Agent | undefined): a is Agent => !!a && this.free(a);
      const ofPlan = [...sim.tasks.values()].filter((t) => t.plan_id === planId && t.assignee).map((t) => sim.agents.get(t.assignee!)).reverse();
      const agent = ofPlan.find(free) ?? [...sim.agents.values()].find(free);
      // (an agent started for this task only starts in the plan's worktree, and has none of its own)
      const act = agent ? sim.actor(agent.id) : this.spawnWorker('claude', this.freeName('claude'), 5, task);
      const own = agent ? agent.worktree : null;
      act.set({ task_id: taskId, worktree: task.worktree });
      sim.patchTask(taskId, { status: 'running', assignee: act.id, attempt: 1, status_reason: null });
      sim.decision('route', 'rule', `${task.key} → ${act.a.name}: free, and it works in the plan’s own worktree`, { plan_id: planId, task: task.key, agent: act.a.name }, { agent: act.id, task: taskId });
      const check = failed?.[0];
      const track = check !== undefined ? () => this.trackFix(act, sim.tasks.get(taskId)!, target, check) : () => this.trackResolve(act, sim.tasks.get(taskId)!, target);
      await this.runTrack(act, task, track);
      // The plan's worktree was the worker's for this task only: it has its own again (none, when it had none).
      if (sim.agents.get(act.id)?.worktree === task.worktree) act.set({ worktree: own });
    }
    const plan = sim.plans.get(planId)!;
    const end = sim.tasks.get(taskId)!;
    if (end.status === 'done') {
      const what = fix ? `The plan’s checks pass on ${plan.integration_branch} with ${target} merged in` : `The conflict with ${target} is resolved on ${plan.integration_branch}, and the plan’s checks passed again`;
      sim.decision('plan_done', 'rule', what, { plan_id: planId });
    } else {
      // Like the core: a resolution that does not finish leaves nothing in the plan's branch.
      if (end.status !== 'cancelled') sim.patchTask(taskId, { status: 'cancelled', status_reason: fix ? 'the fix of the plan’s checks did not finish' : 'the conflict was not resolved' });
      sim.chat(plan.thread, 'codly', fix
        ? 'The plan’s checks still fail with your branch merged in. Codly took that merge back: the plan’s branch is as its tasks left it.'
        : 'The conflict with your branch was not resolved. Codly took the unfinished merge back: the plan’s branch is as its tasks left it.');
    }
    if (plan.status === 'running') sim.patchPlan(planId, { status: 'done' });
  }

  private async trackResolve(act: Actor, task: Task, target: string) {
    const files = task.write_scope;
    const check = task.verify[0] ?? 'npm test';
    const steps = ['Read both sides of the conflict', 'Keep both changes', 'Run the checks'];
    const at = (n: number) => steps.map((text, i): [string, 'pending' | 'in_progress' | 'completed'] => [text, i < n ? 'completed' : i === n ? 'in_progress' : 'pending']);
    await act.sleep(800);
    await act.turn(`Task ${task.key} · ${task.title}\n\n${task.goal}`, async () => {
      act.plan(at(0));
      await act.think(`${target} and the plan both changed ${files.join(', ')}. Keep the plan’s work and the change on ${target}, and leave no marker.`);
      await act.tool({ name: 'Grep', activity: 'search', target: { query: '<<<<<<<|>>>>>>>' }, ms: 700, output: files.map((f) => `${f}:8:<<<<<<< HEAD`).join('\n') });
      for (const path of files) await act.tool({ name: 'Read', activity: 'read', target: { path }, ms: 600 });
      act.plan(at(1));
      for (const path of files) await act.tool({ name: 'Edit', activity: 'edit', target: { path }, ms: 1400, files: [path] });
      act.plan(at(2));
      await act.tool({ name: 'Bash', activity: 'test', target: { command: check }, ms: 2600, lines: [`$ ${check}`, 'ok'] });
      act.plan(at(3));
      await act.say(`Both sides are kept in ${files.map((f) => `\`${f}\``).join(', ')}: the plan’s work and the change on \`${target}\`. No conflict marker is left and \`${check}\` passes.`);
    });
    await this.gate(act, task, {
      files, message: `codly: merge ${target} into ${task.branch}`, verify: task.verify.join(' && '), tail: 'ok',
      summary: `Conflict with ${target} resolved in ${files.join(', ')}; both sides kept.`, integration: false,
    });
  }

  /** `check`: the plan's check that fails with `target` merged in (one of `MERGE_FAILURES`). */
  private async trackFix(act: Actor, task: Task, target: string, check: string) {
    const failure = MERGE_FAILURES[check];
    // The test file as the plan's task left it, and as the fix leaves it.
    const theirs = filesOf(failure.files)?.find((f) => f.path === failure.path)?.after;
    if (!theirs?.includes(failure.from)) throw new Error(`the demo cannot fix ${check}: ${failure.path} has no line to change`);
    const fixed: RepoFile = { path: failure.path, before: theirs, after: theirs.replace(failure.from, failure.to) };
    const path = failure.path;
    const steps = ['Run the check that fails', 'Fix it on the merge', 'Run the checks again'];
    await act.sleep(800);
    await act.turn(`Task ${task.key} · ${task.title}\n\n${task.goal}`, async () => {
      act.plan(listAt(steps, 0));
      await act.think(`${target} merged in without a conflict, but \`${check}\` fails now. Find what changed under the plan’s work.`);
      await act.tool({ name: 'Bash', activity: 'test', target: { command: check }, ms: 2400, ok: false, exit: 1, passed: 2, failed: 1, lines: [`$ ${check}`, `  ✘  ${path}: ${failure.test}: ${failure.why}`, '  1 failed', '  2 passed'] });
      act.plan(listAt(steps, 1));
      await act.tool({ name: 'Read', activity: 'read', target: { path }, ms: 600 });
      this.wrote(task, fixed);
      await act.tool({ name: 'Edit', activity: 'edit', target: { path }, input: { file_path: path, old_string: failure.from, new_string: failure.to }, ms: 1400, files: [path], taskKey: task.id });
      act.plan(listAt(steps, 2));
      await act.tool({ name: 'Bash', activity: 'test', target: { command: check }, ms: 2600, lines: [`$ ${check}`, 'ok'] });
      act.plan(listAt(steps, 3));
      await act.say(`\`${check}\` passes again with \`${target}\` merged in: I changed \`${path}\` so the plan’s work and the change on \`${target}\` work together.`);
    });
    await this.gate(act, task, {
      files: [path], message: `codly: fix ${check} with ${target} merged in`, verify: task.verify.join(' && '), tail: 'ok',
      summary: `\`${check}\` passes again on ${task.branch} with ${target} merged in; fixed in ${path}.`, integration: false,
    });
  }

  /** "Keep the branch" on a finished plan: like the core (`keep_plan`), the plan is kept, and comes back as it is now. */
  keepPlan(planId: string): Plan {
    if (!this.sim.plans.has(planId)) throw new Error('Unknown plan');
    return this.sim.patchPlan(planId, { kept: true });
  }

  /** Undo merge: the plan is done and not merged again; its work stays on the integration branch. */
  undoMerge(planId: string) {
    const sim = this.sim;
    const plan = sim.plans.get(planId);
    if (!plan) throw new Error('Unknown plan');
    sim.patchPlan(planId, { merged_sha: null });
    sim.decision('undo_merge', 'human', `You undid the merge of ${plan.integration_branch} into ${sim.workspace?.current_branch ?? 'main'}`, { plan_id: planId });
  }

  // ───────────── board ─────────────

  createTask(title: string, goal: string): Task {
    const sim = this.sim;
    const now = sim.now();
    return sim.putTask({
      id: uid('t-'), plan_id: null, key: `a${this.nextAdhoc++}`, title, goal, acceptance: [], write_scope: [], verify: ['npm test'], depends_on: [],
      status: 'pending', assignee: null, agent_hint: null, account_hint: null, consultant: null, fresh_session: false, branch: null, worktree: null, attempt: 0, status_reason: null, summary: null,
      created_at: now, updated_at: now,
    });
  }

  moveTask(id: string, status: TaskStatus): Task {
    const sim = this.sim;
    const t = sim.tasks.get(id);
    if (!t) throw new Error('Unknown task');
    const from = t.status;
    const retry = ['blocked', 'failed', 'cancelled'].includes(from);
    // Like the core. Stop: a task in flight goes Blocked and Retry resumes it; not while it
    // merges (it is done in a moment). Cancel: from anything but done and cancelled.
    const stop = status === 'blocked' && ['running', 'verifying', 'reviewing', 'changes_requested'].includes(from);
    const cancel = status === 'cancelled' && from !== 'cancelled' && from !== 'done';
    const legal =
      (from === 'pending' && status === 'ready') ||
      (from === 'ready' && status === 'pending') ||
      (retry && (status === 'ready' || status === 'pending')) ||
      stop || cancel;
    if (!legal) throw new Error('Running, review and done are Codly’s stages — tasks move there on their own.');
    const plan = t.plan_id ? sim.plans.get(t.plan_id) : null;
    if (plan && plan.status === 'awaiting_approval' && status === 'ready') throw new Error('Run the plan first — its tasks start together.');
    // Like the core (`move_task`): a task starts only when what it depends on is done.
    if (status === 'ready' && t.depends_on.some((d) => sim.tasks.get(d)?.status !== 'done')) throw new Error(`${t.key} still waits for its dependencies`);
    const reason = status === 'ready'
      ? t.assignee ? 'Starting…' : 'Ready: a free agent picks it up'
      : status === 'cancelled' ? 'Cancelled by you' : stop ? 'stopped by you; Retry to resume' : null;
    const next = sim.patchTask(id, { status, status_reason: reason });
    // After the status: the worker that holds the task lets go at its next step.
    if ((stop || cancel) && t.assignee && this.running.has(t.assignee) && sim.agents.get(t.assignee)?.task_id === id) sim.cancelled.add(t.assignee);
    sim.decision('move', 'human', `You moved ${t.key} to ${status}`, { task: t.key, from, to: status }, { task: id });
    if (status === 'ready') void this.autoStart(next);
    return next;
  }

  /** Like the core: the task and every task that waits for it are cancelled; the task comes first. */
  cancelTaskTree(id: string): Task[] {
    const root = this.sim.tasks.get(id);
    if (!root) throw new Error('Unknown task');
    const plan = root.plan_id ? [...this.sim.tasks.values()].filter((t) => t.plan_id === root.plan_id) : [];
    const behind = waitingOn(plan, id);
    return [root.status === 'cancelled' ? root : this.moveTask(id, 'cancelled'), ...behind.map((t) => this.moveTask(t.id, 'cancelled'))];
  }

  /** Keep what is merged: the plan's unfinished tasks are cancelled and it is done with the merged
   *  ones (cancelled, with nothing merged). */
  finishPlan(planId: string): Plan {
    const sim = this.sim;
    const plan = sim.plans.get(planId);
    if (!plan) throw new Error('Unknown plan');
    if (plan.status !== 'running') throw new Error(`Only a running plan can finish with what is merged; this one is ${plan.status}.`);
    let merged = 0;
    for (const t of [...sim.tasks.values()].filter((t) => t.plan_id === planId && t.status !== 'cancelled')) {
      if (t.status === 'done') {
        merged += 1;
        continue;
      }
      if (t.assignee && this.running.has(t.assignee)) sim.cancelled.add(t.assignee);
      sim.patchTask(t.id, { status: 'cancelled', status_reason: 'cancelled: you kept what is merged' });
    }
    sim.decision('schedule', 'human', 'You kept what is merged; the other tasks are cancelled', { plan_id: planId });
    return sim.patchPlan(planId, { status: merged ? 'done' : 'cancelled' });
  }

  /** Re-run checks, from a stopped task's card. No demo check has a risk, so none waits for your OK. */
  recheckTask(id: string): HeldCheck[] {
    const t = this.stoppedTask(id, 'only a blocked or failed task can be checked again');
    // The core's record of a re-run that is yours (gate.rs `recheck_task`). Its other record,
    // "Codly re-ran the checks of … (you changed the required checks)", has no place here: no
    // demo task waits for the required checks, so none is checked again when you change them.
    this.sim.decision('gate', 'human', `you re-ran the checks of ${t.key}`, { allowed: [] }, { task: id });
    this.landStopped(t, true);
    return [];
  }

  /** Merge anyway, from a stopped task's card: it lands without its checks. */
  acceptTask(id: string): Task {
    const t = this.stoppedTask(id, 'only a task that its checks stopped can be merged anyway');
    this.sim.decision('gate', 'human', `You chose to merge ${t.key} anyway, without its checks`, { task: t.key, not_run: t.verify }, { task: id });
    return this.landStopped(t, false);
  }

  /** Send back, from a task's diff. Like the core: the comments go to the task's worker (the demo's
   *  worker goes on with its script), a stopped task starts again, and a done task takes none. */
  sendBackTask(id: string, comments: string): Task {
    const sim = this.sim;
    const t = sim.tasks.get(id);
    const text = comments.trim();
    if (!t) throw new Error('Unknown task');
    if (!text) throw new Error('there is no comment to send');
    if (t.status === 'done') throw new Error(`${t.key} is done and its work is merged; ask for the changes in a new request`);
    if (t.status === 'cancelled') throw new Error(`${t.key} is cancelled; reopen it first`);
    if (t.status === 'pending') throw new Error(`${t.key} has not started yet`);
    if (t.status === 'merging') throw new Error(`Codly is working on ${t.key} right now; send your comments again in a moment`);
    const stopped = t.status === 'blocked' || t.status === 'failed';
    const asked = { status: 'changes_requested' as const, status_reason: 'you asked for changes; Codly checks the work again when its worker is done' };
    const next = stopped ? this.moveTask(id, 'ready') : t.status === 'ready' ? t : sim.patchTask(id, asked);
    if (t.assignee) sim.chat(`agent:${t.assignee}`, 'user', text);
    sim.decision('review', 'human', `You sent ${t.key} back with review comments`, { task: t.key, comments: text }, { task: id });
    return next;
  }

  /** Like the core: only a blocked or failed task. `only` says which tasks the card's action is for. */
  private stoppedTask(id: string, only: string): Task {
    const t = this.sim.tasks.get(id);
    if (!t) throw new Error('Unknown task');
    if (t.status !== 'blocked' && t.status !== 'failed') throw new Error(`${t.key} is ${t.status}; ${only}`);
    return t;
  }

  /** The demo's short gate for a stopped task: its checks (when asked for), then done. */
  private landStopped(t: Task, verify: boolean): Task {
    const sim = this.sim;
    const next = sim.patchTask(t.id, { status: 'verifying', status_reason: verify ? 'Codly is checking the work' : 'Merging without its checks' });
    void (async () => {
      await sim.sleep(1200);
      // Stopped or cancelled meanwhile: the gate ends there.
      if (sim.tasks.get(t.id)?.status !== 'verifying') return;
      if (verify) for (const command of t.verify) sim.emit({ type: 'verified', command, ok: true, exit_code: 0, tail: '' }, { agent: t.assignee, task: t.id });
      sim.patchTask(t.id, { status: 'done', status_reason: null });
    })();
    return next;
  }

  assignTask(id: string, agentId: string | null): Task {
    const sim = this.sim;
    const t = sim.tasks.get(id);
    if (!t) throw new Error('Unknown task');
    if (['running', 'verifying', 'merging', 'reviewing'].includes(t.status)) throw new Error(`${t.key} is already being worked on.`);
    if (t.status === 'done') throw new Error(`${t.key} is done.`);
    const a = agentId ? sim.agents.get(agentId) : null;
    if (agentId && !a) throw new Error('Unknown agent');
    if (a?.role === 'lead') throw new Error('Leads plan and review; they don’t take tasks. Pick a worker.');
    const next = sim.patchTask(id, { assignee: agentId });
    if (agentId) this.userAssigned.add(id);
    else this.userAssigned.delete(id);
    sim.decision('assign', 'human', a ? `You assigned ${t.key} to ${a.name}` : `You unassigned ${t.key}`, { task: t.key, agent: a?.name ?? null }, { agent: agentId, task: id });
    if (a && (t.status === 'ready' || t.status === 'pending') && !t.depends_on.some((d) => sim.tasks.get(d)?.status !== 'done') && !(t.plan_id && sim.plans.get(t.plan_id)?.status === 'awaiting_approval'))
      void this.autoStart(next);
    return next;
  }

  private async autoStart(t: Task) {
    const sim = this.sim;
    await sim.sleep(700);
    // A task you stopped, then retried: its worker lets go of the old run at its next step.
    while (t.assignee && sim.cancelled.has(t.assignee) && this.running.has(t.assignee)) await sim.sleep(300);
    await this.notHeld(t.id);
    const task = sim.tasks.get(t.id)!;
    if (task.status !== 'ready' && task.status !== 'pending') return;
    const spec = task.plan_id ? this.specOfPlan.get(task.plan_id) : undefined;
    if (task.plan_id && spec && ROUTES[spec][task.key]) {
      if (sim.plans.get(task.plan_id)?.status !== 'running') return;
      this.startPlanned(task.plan_id, task.key);
      return;
    }
    let agent: Agent | null = task.assignee ? sim.agents.get(task.assignee) ?? null : null;
    if (agent && this.running.has(agent.id)) {
      sim.patchTask(task.id, { status: 'ready', status_reason: `Queued: ${agent.name} is busy` });
      while (this.running.has(agent.id)) await sim.sleep(600);
    }
    if (!agent) {
      agent = [...sim.agents.values()].find((a) => this.free(a)) ?? null;
      if (!agent) {
        const used = new Set([...sim.agents.values()].map((a) => a.color));
        const color = [5, 1, 6, 0].find((c) => !used.has(c)) ?? 5;
        agent = this.spawnWorker('claude', this.freeName('claude'), color, task).a;
      }
      sim.decision('route', 'rule', `${task.key} → ${agent.name}: first free worker (${agent.provider})`, { task: task.key, agent: agent.name }, { agent: agent.id, task: task.id });
    }
    const act = sim.actor(agent.id);
    this.assign(task, act, 'codly/adhoc');
    void this.runTrack(act, task, () => this.trackAdhoc(act, sim.tasks.get(task.id)!));
  }

  private async trackAdhoc(act: Actor, task: Task) {
    await act.sleep(800);
    await act.turn(`Task ${task.key} · ${task.title}\n\n${task.goal}`, async () => {
      act.plan([['Understand the task', 'in_progress'], ['Make the change', 'pending'], ['Verify', 'pending']]);
      await act.think(`Small task: “${task.title}”. Read the manifest, change the minimum, run the tests.`);
      await act.tool({ name: 'Read', activity: 'read', target: { path: 'package.json' }, ms: 600 });
      await act.tool({ name: 'Grep', activity: 'search', target: { query: task.title.split(/\s+/).slice(0, 2).join('|') }, ms: 800, output: 'package.json:12:    "eslint": "^9.38.0",' });
      act.plan([['Understand the task', 'completed'], ['Make the change', 'in_progress'], ['Verify', 'pending']]);
      await act.tool({ name: 'Edit', activity: 'edit', target: { path: 'package.json' }, ms: 1400, files: ['package.json'], taskKey: task.key });
      act.plan([['Understand the task', 'completed'], ['Make the change', 'completed'], ['Verify', 'in_progress']]);
      await act.tool({ name: 'Bash', activity: 'test', target: { command: 'npm test' }, ms: 2800, passed: 41, failed: 0,
        lines: ['> acme-web@2.4.0 test', '> vitest run', '', ' Test Files  9 passed (9)', '      Tests  41 passed (41)', '   Duration  3.02s'] });
      act.plan([['Understand the task', 'completed'], ['Make the change', 'completed'], ['Verify', 'completed']]);
      await act.say(`Done: ${task.title}. Changed \`package.json\`; 41 tests pass.`);
    });
    await this.gate(act, task, {
      files: ['package.json'], message: `${task.key}: ${task.title}`, verify: 'npm test', tail: '      Tests  41 passed (41)',
      summary: `${task.title}: package.json updated; 41 tests pass.`, integration: false,
    });
  }

  // ───────────── agents ─────────────

  /** The team members of the open repo as idle agents (like the core's sync_team). */
  syncTeam() {
    const sim = this.sim;
    if (!sim.workspace) return;
    const repo = sim.workspaceTeam;
    const team = [...sim.settings.team.filter((m) => !repo.off.includes(m.id)), ...repo.members].filter((m) => m.enabled);
    const members = [...sim.agents.values()].filter((a) => a.member);
    for (const m of team) {
      const title = memberTitle(m);
      const a = members.find((x) => x.member === m.id);
      if (a) {
        if (a.name !== m.name || a.title !== title || a.model !== m.model || a.provider !== m.provider)
          sim.patchAgent(a.id, { name: m.name, title, model: m.model, provider: m.provider, role: m.kind === 'reviewer' ? 'reviewer' : 'worker' });
        continue;
      }
      const used = new Set([...sim.agents.values()].map((x) => x.color));
      const color = [0, 1, 3, 5, 2, 4, 7, 6].find((c) => !used.has(c)) ?? sim.agents.size % 8;
      const agent = sim.newAgent({
        name: m.name, provider: m.provider, model: m.model, role: m.kind === 'reviewer' ? 'reviewer' : 'worker', color,
        billing: m.provider === 'opencode' ? 'third_party' : 'subscription', status: 'idle',
      });
      sim.putAgent({ ...agent, member: m.id, title, run_id: null });
    }
    for (const a of members.filter((x) => !team.some((m) => m.id === x.member))) {
      sim.putAgent({ ...a, archived: true, status: 'offline', updated_at: sim.now() });
      sim.agents.delete(a.id);
    }
  }

  /** A name for a new agent that no agent of the office has: its CLI's stock name, then "Eli 2", "Eli 3"… */
  private freeName(provider: Provider): string {
    return this.unusedName(WORKER_NAMES[provider]);
  }

  /** `base` when no agent of the office has it, else "<base> 2", "<base> 3"… */
  private unusedName(base: string): string {
    const names = new Set([...this.sim.agents.values()].map((a) => a.name));
    let name = base;
    for (let i = 2; names.has(name); i++) name = `${base} ${i}`;
    return name;
  }

  /**
   * A new 1:1 agent. `named` is the name the hire wizard gave it (checked there to be free);
   * without one it gets its CLI's stock name. `isolate` false: Git isolation is off, so it works
   * in your project folder.
   */
  hire(provider: Provider, model: string | null = null, title: string | null = null, named: string | null = null, isolate = true): Agent {
    const sim = this.sim;
    const name = named ?? this.freeName(provider);
    const used = new Set([...sim.agents.values()].map((a) => a.color));
    const color = [0, 1, 3, 5, 2, 4, 7, 6].find((c) => !used.has(c)) ?? sim.agents.size % 8;
    // Like the core: a 1:1 agent works in a worktree of its own, called after its name. Two
    // names can read the same in a path, so a path that is taken gets a number. Its session
    // starts there: the folder is known before the agent is.
    const taken = new Set([...sim.agents.values()].map((a) => a.worktree));
    let worktree = `${this.worktrees}/${slug(name)}`;
    for (let i = 2; taken.has(worktree); i++) worktree = `${this.worktrees}/${slug(name)}-${i}`;
    const act = this.spawnWorker(provider, name, color, null, model, isolate ? worktree : (sim.workspace?.repo_path ?? REPO));
    act.status('idle');
    return title ? sim.patchAgent(act.a.id, { title }) : act.a;
  }

  /** `?mode=plan` (and `?approval=plan`): a 1:1 Claude agent in "plan first", asked for a small change, so its plan waits for your answer. */
  private seedPlanner() {
    const a = this.hire('claude', null, 'Docs');
    this.sim.patchAgent(a.id, { permission_mode: 'plan' });
    this.userChat(`agent:${a.id}`, 'Add a short “Theming” section to the README.');
  }

  /**
   * `?background=on`: a 1:1 Claude agent, asked for two things that take long, starts both in
   * the background and ends its turn. Its sheet lists them under "In the background", each with
   * Stop (demo/background.ts).
   */
  private seedBackground() {
    const a = this.hire('claude', null, 'Docs');
    this.scripted.set(a.id, (act) => this.leaveAtWork(act));
    this.userChat(`agent:${a.id}`, 'Start the dev server, and have a helper check every link in the docs. Do not wait for them.');
  }

  private async leaveAtWork(act: Actor) {
    await act.think('Both can run beside me: the server as a background command, the links with a helper in the background.', { pace: 40 });
    // The dev server never ends by itself; the helper reports after about three minutes.
    await this.inBackground(act, 'npm run dev');
    this.helperInBackground(act, 'Check every link in the docs', [
      { name: 'Glob', activity: 'search', target: { query: 'docs/**/*.md' }, step: 'Listing the pages in docs/' },
      { name: 'Grep', activity: 'search', target: { query: '\\]\\((?!#)' }, step: 'Collecting the links of each page' },
      { name: 'WebFetch', activity: 'fetch', target: { url: 'http://localhost:3000/docs' }, step: 'Following each link' },
    ], 180_000, 'All 42 links in `docs/` answer.');
    await act.say('The dev server runs on port 3000, and a helper checks the links in `docs/`. Both work in the background: I will tell you what the helper finds.', { thread: `agent:${act.id}` });
  }

  /** Interrupt the agent's turn (Send now): its next queued message runs right after. */
  interrupt(agentId: string, stop = false) {
    const sim = this.sim;
    const a = sim.agents.get(agentId);
    if (!a) throw new Error('Unknown agent');
    const act = sim.actor(agentId);
    if (this.running.has(agentId) || this.busyLeads.has(agentId) || act.inTurn) sim.cancelled.add(agentId);
    else if (!stop) throw new Error(`${a.name} isn’t running anything.`);
    // Like the core: what the agent asked you does not outlive its turn (its CLI takes the
    // request back) or its process (Codly closes the request).
    for (const ap of [...sim.approvals.values()].filter((ap) => ap.agent_id === agentId && ap.status === 'pending')) {
      const [status, by] = stop ? (['expired', 'codly'] as const) : (['cancelled', 'provider'] as const);
      sim.approvals.set(ap.id, { ...ap, status, decided_by: by });
      sim.emit({ type: 'approval_resolved', approval_id: ap.id, status, by }, { agent: ap.agent_id, task: ap.task_id, run: ap.run_id });
    }
    if (stop) {
      act.emit({ type: 'exited', code: 0 });
      // Like the core: the run ends with the process, and what it left in the background with it
      // (no list says so). The agent's next turn starts a new run (`Actor.turn`).
      act.set({ status: 'offline', activity: null, activity_label: null, run_id: null });
    }
  }

  // ───────────── diffs ─────────────

  /** The files a task writes, with their text: the ones it decided as it worked (`wrote`), else its
   *  demo files; a board task of its own (no plan) edits the manifest, as its diff shows. */
  taskFiles(t: Task): RepoFile[] {
    return this.written.get(t.id) ?? filesOf(this.fileKey(t)) ?? (t.plan_id ? [] : TASK_FILES.adhoc);
  }

  taskDiff(id: string): string {
    const t = this.sim.tasks.get(id);
    if (!t) throw new Error('Unknown task');
    if (t.key === 'a3' && !t.plan_id) return diffOf(NEXT_BUMP);
    // (a task that wrote files of its own text grows its diff by its id: plans reuse keys)
    const own = this.written.get(id);
    const fk = own ? id : this.fileKey(t);
    const files = own ?? filesOf(fk) ?? TASK_FILES.adhoc;
    const touched = this.sim.touched.get(fk) ?? new Set<string>();
    return diffOf(files.filter((f) => touched.has(f.path)));
  }

  planDiff(id: string): string {
    const tasks = [...this.sim.tasks.values()].filter((t) => t.plan_id === id && t.status === 'done');
    // One part per file, as git shows a branch: a file two tasks wrote (a fix of the plan's own
    // test) goes from its text before the first to its text after the last.
    const byPath = new Map<string, RepoFile>();
    for (const f of tasks.flatMap((t) => this.taskFiles(t))) {
      const first = byPath.get(f.path);
      byPath.set(f.path, first ? { ...f, before: first.before } : f);
    }
    return diffOf([...byPath.values()]);
  }

  searchIndex() {
    const done = [...this.sim.tasks.values()].filter((t) => t.status === 'done').map((t) => this.fileKey(t));
    return [...INDEX, ...done.flatMap((k) => INDEX_AFTER[k] ?? [])];
  }
}
