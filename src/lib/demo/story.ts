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
// docs/DEMO.md lists the URL parameters that start the demo in a state the script does not reach.
import type { Agent } from '../gen/Agent';
import type { Approval } from '../gen/Approval';
import type { Citation } from '../gen/Citation';
import type { HeldCheck } from '../gen/HeldCheck';
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
import { MODE_LABEL } from './controls';
import { diffOf, INDEX, INDEX_AFTER, NEXT_BUMP, TASK_FILES, type RepoFile } from './repo';
import { Actor, Cancelled, REPO, Sim, WORKTREES, sha, uid, type ToolSpec } from './sim';
import { waitingOn } from '../util/plan';
import { memberTitle } from '../util/team';

export const REQUEST =
  'Add a dark mode toggle to Settings. Persist the choice per user, follow the system by default, and cover it with tests.';
/** Sent while the Lead is still planning: a Claude Code or Codex Lead reads it after its current step. */
export const AMEND = 'Also make the toggle fully keyboard-accessible: arrow keys should move between the options.';
/** The second request, opened in parallel (⌘N) a little later. */
export const REQUEST_B = 'Lock sign-in for 15 minutes after 5 failed attempts, and tell people clearly when they’re locked out.';

const HOUR = 3_600_000;
const cite = (id: string, path: string, s: number, e: number, symbol: string | null, why: string[], score: number): Citation => ({
  id, path, start_line: s, end_line: e, symbol, why, score,
});

const WORKER_NAMES: Record<Provider, string> = { claude: 'Eli', codex: 'Fay', gemini: 'Gus', opencode: 'Dee', cursor: 'Hal', copilot: 'Ivy' };
const WORKER_MODELS: Record<Provider, string | null> = {
  claude: 'claude-sonnet-5-5', codex: 'gpt-6.1-sol', gemini: 'gemini-3.8-flash', opencode: 'claude-haiku-5-5', cursor: null, copilot: null,
};
const PROVIDER_NAME: Record<Provider, string> = { claude: 'Claude Code', codex: 'Codex', gemini: 'Gemini CLI', opencode: 'OpenCode', cursor: 'Cursor', copilot: 'Copilot' };

type Spec = 'theme' | 'lockout';
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
};
const BRANCH: Record<Spec, string> = { theme: 'codly/dark-mode', lockout: 'codly/sign-in-lockout' };
/** Task key → the demo files it writes (TASK_FILES key). */
const FILES: Record<Spec, Record<string, string>> = { theme: { t1: 't1', t2: 't2', t3: 't3' }, lockout: { t1: 'l1', t2: 'l2' } };
const KEYBOARD = /keyboard|arrow|a11y|accessib/i;
/** Claude Code and Codex take a message into the turn that runs; the other CLIs run it as the next turn (state/queue.ts). */
const readsMidTurn = (provider: Provider) => provider === 'claude' || provider === 'codex';
const quoted = (text: string, max: number) => `“${text.slice(0, max)}${text.length > max ? '…' : ''}”`;
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
  /** task id → its TASK_FILES key (plans reuse keys t1, t2… so files are keyed apart). */
  private fileKeys = new Map<string, string>();
  private nextAdhoc = 4;

  constructor(private sim: Sim, private opts: StoryOptions = {}) {
    this.rules = new MockApprovalRules(sim, this);
    this.checks = opts.checks ?? new ChecksDemo();
    sim.afterStep = (act) => this.readMidTurn(act);
  }

  // ───────────── workspace ─────────────

  openWorkspace(path: string): Workspace {
    const sim = this.sim;
    if (sim.workspace) return sim.workspace;
    const now = sim.now();
    const isDemo = /acme/i.test(path) || path === REPO;
    const name = isDemo ? 'acme/web' : (path.replace(/\/+$/, '').split('/').pop() ?? 'repo');
    sim.workspace = {
      id: uid('ws-'), repo_path: isDemo ? REPO : path, name, default_branch: 'main', current_branch: 'main',
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

    if (this.opts.planMode) this.seedPlanner();
    if (this.opts.yourRequestFirst) return sim.workspace;
    // The demo's "you": ask for the feature, add a detail while the Lead is still planning (it
    // reads it after its current step, or it queues), then open a second request in parallel.
    void (async () => {
      await sim.sleep(1600);
      if (!sim.chats.some((c) => c.thread === 'lead' && c.author === 'user')) this.userChat('lead', REQUEST);
      await sim.sleep(5400);
      const lead = this.leadActor('lead');
      if (lead?.inTurn && !this.req('lead').planId && !sim.chats.some((c) => c.thread === 'lead' && c.text === AMEND)) this.userChat('lead', AMEND);
      await sim.sleep(17_000);
      const others = sim.chats.some((c) => c.thread.startsWith('lead:') && c.author === 'user');
      if (!others) this.userChat(`lead:${Math.random().toString(36).slice(2, 8)}`, REQUEST_B);
    })();
    return sim.workspace;
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
    this.unread.set(thread, waiting.filter((m) => m !== sent));
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
    act.emit({
      type: 'session_started', provider, model: lead.model, cwd: sim.workspace?.repo_path ?? REPO, cli_version: '2.1.309',
      session_id: lead.session_id, billing: 'subscription',
      tools: ['Read', 'Grep', 'Glob', 'WebFetch', 'mcp__codly__codly_search', 'mcp__codly__codly_submit_plan', 'mcp__codly__codly_memory_search'],
    });
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
    const used = new Set([...this.reqs.values()].map((r) => r.spec).filter(Boolean));
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
      else await this.planLockout(lead, r);
      return;
    }
    await this.leadChat(lead, r, text);
  }

  private plan(r: Req): Plan | null {
    return r.planId ? (this.sim.plans.get(r.planId) ?? null) : null;
  }

  private postPlanReady(r: Req, plan: Plan, n: number) {
    // Like the core: Codly posts the plan in the request's thread; the card rides on this line.
    this.sim.chat(r.thread, 'codly', `Plan ready: ${plan.summary} (${n} tasks). Review it on the board, then click Run plan.`, { plan_id: plan.id });
  }

  private async autoRun(r: Req) {
    const sim = this.sim;
    if (!(sim.opts.autoplay || sim.settings.auto_run_plans)) return;
    await sim.sleep(sim.settings.auto_run_plans ? 600 : 2800);
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
    this.fileKeys.set(t.id, FILES[spec][key]);
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
      await lead.tool({ name: 'Read', activity: 'read', target: { path: 'README.md' }, ms: 900,
        output: '# Acme web\n\nNext.js 16 (app router) · Prisma · Postgres · vitest · Playwright\n\n## Scripts\n- npm run dev\n- npm test\n- npx playwright test' });
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
      if (/flaky|checkout/i.test(text))
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
    } else if (plan.status === 'done')
      reply = this.specOfPlan.get(plan.id) === 'theme'
        ? 'Happy to plan a follow-up. A good next step is rendering the theme on the server so the first paint never flashes — want a plan for that?'
        : 'Happy to plan a follow-up. A good next step is an admin unlock and an email to the account owner after a lockout — want a plan for that?';
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
    if (!sim.agents.get(agentId)) return;
    const act = sim.actor(agentId);
    const thread = `agent:${agentId}`;
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
      const reply = busy
        ? `Noted. I’m finishing **${task!.key} · ${task!.title}** — I’ll take “${text.slice(0, 48)}${text.length > 48 ? '…' : ''}” into account before Codly merges it.`
        : 'I’m free. Assign me a task from the Plan room, or ask a Lead to plan something and I’ll pick it up.';
      try {
        await act.turn(text, async () => {
          if (works) await this.soloWork(act, text);
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

  // ───────────── plan execution ─────────────

  runPlan(planId: string) {
    const sim = this.sim;
    const plan = sim.plans.get(planId);
    const spec = this.specOfPlan.get(planId);
    if (!plan || !spec || plan.status !== 'awaiting_approval') throw new Error('This plan is not waiting to run.');
    const base = sim.workspace?.head ?? sha();
    const branch = `${BRANCH[spec]}/integration`;
    sim.patchPlan(planId, { status: 'running', base_commit: base, base_branch: 'main', integration_branch: branch, integration_worktree: `${WORKTREES}/${spec}-integration` });
    sim.decision('integration_branch', 'rule', `Created ${branch} from main @ ${base.slice(0, 7)}`, { plan_id: planId, base });
    const tasks = [...sim.tasks.values()].filter((t) => t.plan_id === planId);
    for (const t of tasks) {
      if (!t.depends_on.length) sim.patchTask(t.id, { status: 'ready', status_reason: 'Dependencies met; starting' });
      else {
        const waits = t.depends_on.map((d) => sim.tasks.get(d)?.key).join(', ');
        sim.patchTask(t.id, { status: 'pending', status_reason: `Waiting on ${waits}` });
        sim.decision('schedule', 'rule', `${t.key} waits for ${waits} (dependencies not merged yet)`, { plan_id: planId, task: t.key, waiting_on: waits }, { task: t.id });
      }
    }
    void this.coordinate(planId);
  }

  private spawnWorker(provider: Provider, name: string, color: number, task: Task | null, model?: string | null): Actor {
    const sim = this.sim;
    const worktree = task ? `${WORKTREES}/${task.key}-${name.toLowerCase()}` : null;
    const a = sim.putAgent(sim.newAgent({
      name, provider, model: model ?? WORKER_MODELS[provider], role: task ? 'worker' : 'solo', color, billing: provider === 'opencode' ? 'third_party' : 'subscription',
      taskId: task?.id ?? null, worktree,
    }));
    const act = sim.actor(a.id);
    const transport = provider === 'claude' ? 'stream-json' : provider === 'codex' ? 'app-server' : 'acp';
    act.emit({
      type: 'session_started', provider, model: a.model, cwd: worktree ?? sim.workspace?.repo_path ?? REPO,
      cli_version: provider === 'claude' ? '2.1.309' : provider === 'codex' ? '0.161.0' : provider === 'gemini' ? '0.62.0' : '1.18.31',
      session_id: a.session_id, billing: a.billing,
      tools: transport === 'stream-json' ? ['Bash', 'Read', 'Edit', 'Write', 'Grep', 'Glob', 'TodoWrite', 'mcp__codly__codly_search', 'mcp__codly__codly_post_note']
        : transport === 'app-server' ? ['shell', 'apply_patch', 'update_plan', 'codly.codly_search']
        : ['read_file', 'write_file', 'replace', 'run_shell_command', 'glob', 'search_file_content', 'codly_search'],
    });
    return act;
  }

  private assign(task: Task, act: Actor, branchRoot: string) {
    const sim = this.sim;
    const worktree = `${WORKTREES}/${task.key}-${act.a.name.toLowerCase()}`;
    act.set({ task_id: task.id, worktree, role: act.a.role === 'lead' ? 'lead' : 'worker' });
    sim.patchTask(task.id, {
      status: 'running', assignee: act.id, branch: `${branchRoot}/${task.key}`, worktree, attempt: task.attempt + 1, status_reason: null,
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
      const existing = sim.agentByName(route.name);
      act = existing && !this.running.has(existing.id) ? sim.actor(existing.id) : null;
    }
    const chosenByYou = !!(preassigned && act && act.id === preassigned.id && this.userAssigned.has(task.id));
    if (!act) act = this.spawnWorker(route.provider, route.name, route.color, task);
    const a = act.a;
    const label = `${a.name} · ${PROVIDER_NAME[a.provider]}`;
    sim.decision('route', chosenByYou ? 'human' : 'rule', chosenByYou ? `${key} → ${label}: you assigned it` : `${key} → ${label}: ${route.why}`, {
      plan_id: plan.id, task: key, chosen: a.provider, agent: a.name, alternatives: route.alternatives,
    }, { agent: a.id, task: task.id });
    this.assign(task, act, BRANCH[spec]);
    const body =
      spec === 'theme' ? { t1: () => this.trackT1(act!, task), t2: () => this.trackT2(act!, task), t3: () => this.trackT3(act!, task) }[key]
      : { t1: () => this.trackL1(act!, task), t2: () => this.trackL2(act!, task) }[key];
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
        sim.patchTask(t.id, { status: 'blocked', status_reason: 'Interrupted by you. Move it to Ready to retry.' });
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
    if (!this.running.has(act.id) || act.a.task_id === task.id) act.set({ status: 'idle', activity: null, activity_label: null, task_id: null });
  }

  /**
   * Wait for your answer to a request that is on screen. With `?autoplay=1` it is allowed after
   * `auto` ms (`answers`: what autoplay picks for a question).
   */
  private async answered(act: Actor, ap: Approval, label: string, auto: number, answers: unknown = null): Promise<Approval['status']> {
    const sim = this.sim;
    act.status('waiting_approval', 'await_approval', label);
    while (sim.approvals.get(ap.id)!.status === 'pending') {
      await act.sleep(250);
      if (sim.opts.autoplay && sim.now() - ap.created_at > auto) this.resolveApproval(ap.id, 'allow_once', null, answers);
    }
    return sim.approvals.get(ap.id)!.status;
  }

  /** A command that asks first. `output`: what it prints once it is allowed. */
  private async approval(act: Actor, command: string, reason: string, risks: string[], output: string[]): Promise<'approved' | 'denied'> {
    const sim = this.sim;
    const a = act.a;
    const call = uid('call-');
    act.emit({
      type: 'tool_started', call_id: call, name: 'commandExecution', activity: 'execute',
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
    const st = rule ? 'approved' : await this.answered(act, ap, `${command.split(' ').slice(0, 3).join(' ')}…`, 3500);
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

  /** AskUserQuestion-style question; resolves with the chosen label (or null when skipped). */
  private async question(act: Actor, question: string, header: string, options: { label: string; description: string }[]): Promise<string | null> {
    const sim = this.sim;
    const a = act.a;
    const ap: Approval = {
      id: uid('ap-'), agent_id: a.id, run_id: a.run_id ?? '', task_id: a.task_id, call_id: null,
      subject: { kind: 'question', questions: { questions: [{ question, header, options, multiSelect: false }] } },
      reason: null, choices: ['allow_once', 'deny'], risks: [], status: 'pending', decided_by: null, created_at: sim.now(),
    };
    sim.approvals.set(ap.id, ap);
    act.emit({ type: 'approval_requested', approval: ap });
    const st = await this.answered(act, ap, header, 4000, { [question]: options[0].label });
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

  /** In "ask for edits" a 1:1 Claude agent asks before an edit, with the change it wants to make. */
  private async editApproval(act: Actor, path: string, diff: string): Promise<boolean> {
    const sim = this.sim;
    const a = act.a;
    const ap: Approval = {
      id: uid('ap-'), agent_id: a.id, run_id: a.run_id ?? '', task_id: a.task_id, call_id: null,
      subject: { kind: 'file_change', paths: [path], diff }, reason: null, choices: ['allow_once', 'deny', 'cancel'], risks: [],
      status: 'pending', decided_by: null, created_at: sim.now(),
    };
    sim.approvals.set(ap.id, ap);
    act.emit({ type: 'approval_requested', approval: ap });
    const st = await this.answered(act, ap, path.split('/').pop() ?? path, 3500);
    act.status('working', 'thinking');
    if (st === 'cancelled') throw new Cancelled();
    return st === 'approved';
  }

  /**
   * A 1:1 Claude agent does what you ask in its mode (its sheet → Controls → Mode): "plan first"
   * shows its plan before it changes anything, "ask for edits" asks before the edit, "accept
   * edits" makes it without asking.
   */
  private async soloWork(act: Actor, ask: string) {
    const sim = this.sim;
    const thread = `agent:${act.id}`;
    const path = 'README.md';
    await act.think('Read how the project is set up before changing anything.', { pace: 40 });
    await act.tool({ name: 'Read', activity: 'read', target: { path }, ms: 600 });
    await act.tool({ name: 'Read', activity: 'read', target: { path: 'package.json' }, ms: 500 });
    if (act.a.permission_mode === 'plan') {
      // Like Claude Code: a plan you send back is changed and shown again; an accepted plan
      // ends plan mode, and the agent asks for edits from then on.
      for (let note = await this.showPlan(act, soloPlan(ask, null)); note !== null; note = await this.showPlan(act, soloPlan(ask, note))) {
        await act.say(note ? 'I changed the plan as you asked.' : 'The plan stays as it is until you say what to change.', { phase: 'commentary' });
      }
      act.set({ permission_mode: 'ask' });
      sim.decision('permission_mode', 'rule', `Claude Code changed its mode to "${MODE_LABEL.ask}"`, { mode: 'ask' }, { agent: act.id });
    }
    const added = `## Notes\n\n- ${ask.replace(/\s+/g, ' ').slice(0, 200)}`;
    const asks = (act.a.permission_mode ?? 'ask') === 'ask';
    if (asks && !(await this.editApproval(act, path, `@@ ${path} @@\n${added.split('\n').map((l) => `+${l}`).join('\n')}`))) {
      await act.say('Understood: I changed nothing. Tell me what to do instead.', { thread });
      return;
    }
    await act.tool({ name: 'Edit', activity: 'edit', target: { path }, ms: 1200, files: [path] });
    await act.tool({
      name: 'Bash', activity: 'test', target: { command: 'npm test' }, ms: 2400, passed: 41, failed: 0,
      lines: ['> acme-web@2.4.0 test', '> vitest run', '', ' Test Files  9 passed (9)', '      Tests  41 passed (41)'],
    });
    await act.say(`Done: I added a short note to \`${path}\` about ${quoted(ask, 80)}. 41 tests pass.`, { thread });
  }

  /**
   * A Claude Code Task-tool helper: a sub-agent with its kind, its own calls (`steps`, over `ms`
   * in all), how far it is, and its report.
   */
  private async subagent(act: Actor, description: string, steps: HelperStep[], ms: number, result: string) {
    const sim = this.sim;
    const call = uid('call-');
    const started = sim.now();
    const none: Target = { path: null, paths: [], command: null, query: null, url: null, server: null, tool: null };
    act.emit({
      type: 'tool_started', call_id: call, name: 'Task', activity: 'delegate', target: { ...none, tool: 'Task' },
      input: { description, subagent_type: 'Explore' }, confidence: 'observed',
    });
    act.status('working', 'delegate', 'Explore');
    act.emit({ type: 'sub_agent', call_id: call, description, status: 'running' });
    act.emit({ type: 'sub_agent_progress', call_id: call, name: 'Explore', step: null, tool_call_id: null, tool_uses: null, tokens: null, report: null });
    // `?helper=slow`: 20 times as long, so the row can be looked at while the helper works.
    const each = (ms * (this.opts.slowHelper ? 20 : 1)) / steps.length;
    for (const [i, s] of steps.entries()) {
      const own = uid('call-');
      act.emit({ type: 'tool_started', call_id: own, name: s.name, activity: s.activity, target: { ...none, ...s.target }, input: {}, confidence: 'observed' });
      // Its `tool_started` came just before: this says that the call is the helper's own.
      act.emit({ type: 'sub_agent_progress', call_id: call, name: null, step: s.step, tool_call_id: own, tool_uses: i + 1, tokens: 2100 * (i + 1), report: null });
      await act.sleep(each);
      act.emit({ type: 'tool_completed', call_id: own, ok: true, output: null, exit_code: null, duration_ms: Math.round(each), tests_passed: null, tests_failed: null });
    }
    act.emit({ type: 'sub_agent_progress', call_id: call, name: null, step: null, tool_call_id: null, tool_uses: null, tokens: null, report: result });
    act.emit({ type: 'sub_agent', call_id: call, description, status: 'completed' });
    act.emit({ type: 'tool_completed', call_id: call, ok: true, output: result, exit_code: null, duration_ms: sim.now() - started, tests_passed: null, tests_failed: null });
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

  resolveApproval(id: string, choice: string, message: string | null, answers: unknown = null) {
    const sim = this.sim;
    const ap = sim.approvals.get(id);
    if (!ap || ap.status !== 'pending') throw new Error('This request was already answered.');
    if (answers != null) this.answers.set(id, answers);
    if (message) this.notes.set(id, message);
    const status = choice === 'deny' || choice === 'cancel' ? (choice === 'cancel' ? 'cancelled' : 'denied') : 'approved';
    sim.approvals.set(id, { ...ap, status, decided_by: 'user' });
    sim.emit({ type: 'approval_resolved', approval_id: id, status, by: 'user' }, { agent: ap.agent_id, task: ap.task_id, run: ap.run_id });
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
      await this.subagent(ada, 'Explore: find every place that reads or switches the theme', [
        { name: 'Grep', activity: 'search', target: { query: 'theme|prefers-color-scheme' }, step: 'Searching for code that reads the theme' },
        { name: 'Read', activity: 'read', target: { path: 'src/styles/tokens.css' }, step: 'Reading src/styles/tokens.css' },
        { name: 'Read', activity: 'read', target: { path: 'src/hooks/useMediaQuery.ts' }, step: 'Reading src/hooks/useMediaQuery.ts' },
      ], 2800, 'Three places: tokens.css defines `.dark` but nothing applies it; useMediaQuery.ts already wraps matchMedia; layout.tsx has no provider.');
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
      for (const t of tasks().sort((a, b) => a.key.localeCompare(b.key, undefined, { numeric: true }))) {
        if (started.has(t.id) || t.status === 'done' || t.status === 'cancelled') continue;
        if (t.depends_on.some((d) => sim.tasks.get(d)?.status !== 'done')) continue;
        started.add(t.id);
        if (t.status === 'pending') {
          sim.patchTask(t.id, { status: 'ready', status_reason: 'Dependencies merged; starting' });
          await sim.sleep(900);
        }
        this.startPlanned(planId, t.key);
        await sim.sleep(500);
      }
      await sim.sleep(400);
    }
    sim.patchPlan(planId, { status: 'integrating' });
    await sim.sleep(1500);
    const plan = sim.plans.get(planId)!;
    const all = tasks().flatMap((t) => TASK_FILES[this.fileKey(t)] ?? []);
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

  mergePlan(planId: string): string {
    const sim = this.sim;
    const plan = sim.plans.get(planId);
    if (!plan) throw new Error('Unknown plan');
    if (plan.status !== 'done') throw new Error('The plan is still running; merge when every task is done.');
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
    const edited = done.flatMap((t) => TASK_FILES[this.fileKey(t)] ?? []).filter((f) => f.before !== null);
    return edited.slice(-1).map((f) => f.path);
  }

  /**
   * "Have an agent resolve it". Like the core: one more task of the plan resolves the conflict
   * with `target` (your branch) in the plan's own worktree, the plan runs again, and it is
   * done again when that task passed the plan's checks. Refused in the core's words.
   */
  resolveConflict(planId: string, target: string, files: string[]): Task {
    const sim = this.sim;
    const plan = sim.plans.get(planId);
    if (!plan) throw new Error('Unknown plan');
    if (plan.merged_sha) throw new Error('this plan is already merged into your branch');
    if (plan.status === 'running' || plan.status === 'integrating') throw new Error('the plan is still running; wait until it is done');
    if (plan.status === 'cancelled') throw new Error('this plan was cancelled, so no agent works on it any more; resolve the conflict yourself');
    const branch = plan.integration_branch;
    if (!branch || (plan.status !== 'done' && plan.status !== 'failed')) throw new Error('the plan never ran, so there is nothing to merge');
    const tasks = [...sim.tasks.values()].filter((t) => t.plan_id === planId);
    // "resolve", then "resolve-2"…: a plan can need this more than once.
    let key = 'resolve';
    for (let n = 2; tasks.some((t) => t.key === key); n++) key = `resolve-${n}`;
    const names = files.join(', ');
    const now = sim.now();
    const task = sim.putTask({
      id: uid('t-'), plan_id: planId, key, title: `Resolve conflicts with ${target}`,
      goal: `The plan is finished, but ${target} (your branch) changed the same lines meanwhile. Codly started merging ${target} into the plan’s worktree, and these files have conflict markers: ${names}. Resolve every conflict so that the plan’s work and the changes on ${target} are both kept, and remove all markers.`,
      acceptance: ['No conflict marker (<<<<<<<, =======, >>>>>>>) is left in any file', `The plan’s work and the changes on ${target} are both kept and work together`],
      write_scope: files, verify: [...new Set(tasks.filter((t) => t.status === 'done').flatMap((t) => t.verify))], depends_on: [], status: 'ready',
      assignee: null, agent_hint: null, account_hint: null, consultant: null, fresh_session: true, branch, worktree: plan.integration_worktree, attempt: 0,
      status_reason: 'Ready: a free agent picks it up', summary: null, created_at: now, updated_at: now,
    });
    sim.decision('merge', 'human', `you had an agent resolve the conflicts of ${branch} with ${target} in ${names}: task ${key}`, { plan_id: planId, files }, { task: task.id });
    sim.patchPlan(planId, { status: 'running', base_branch: target });
    sim.chat(plan.thread, 'codly', `${target} and this plan both changed ${names}. Codly brought ${target} into the plan’s branch, in its own worktree (your checkout is not touched), and task ${key} resolves the conflicts there. Then the plan’s checks run again and you can merge it.`);
    void this.runResolution(planId, task.id, target);
    return task;
  }

  /** A free worker takes the resolving task; then the plan is done again, resolved or not. */
  private async runResolution(planId: string, taskId: string, target: string) {
    const sim = this.sim;
    await sim.sleep(900);
    const task = sim.tasks.get(taskId)!;
    if (task.status === 'ready') {
      // First a worker that knows the plan's work, then any free one.
      const free = (a: Agent | undefined): a is Agent => !!a && a.role === 'worker' && a.status !== 'offline' && !this.running.has(a.id);
      const ofPlan = [...sim.tasks.values()].filter((t) => t.plan_id === planId && t.assignee).map((t) => sim.agents.get(t.assignee!)).reverse();
      const agent = ofPlan.find(free) ?? [...sim.agents.values()].find(free);
      const act = agent ? sim.actor(agent.id) : this.spawnWorker('claude', 'Eli', 5, task);
      act.set({ task_id: taskId, worktree: task.worktree });
      sim.patchTask(taskId, { status: 'running', assignee: act.id, attempt: 1, status_reason: null });
      sim.decision('route', 'rule', `${task.key} → ${act.a.name}: free, and it works in the plan’s own worktree`, { plan_id: planId, task: task.key, agent: act.a.name }, { agent: act.id, task: taskId });
      await this.runTrack(act, task, () => this.trackResolve(act, sim.tasks.get(taskId)!, target));
    }
    const plan = sim.plans.get(planId)!;
    const end = sim.tasks.get(taskId)!;
    if (end.status === 'done') {
      sim.decision('plan_done', 'rule', `The conflict with ${target} is resolved on ${plan.integration_branch}, and the plan’s checks passed again`, { plan_id: planId });
    } else {
      // Like the core: a resolution that does not finish leaves nothing in the plan's branch.
      if (end.status !== 'cancelled') sim.patchTask(taskId, { status: 'cancelled', status_reason: 'the conflict was not resolved' });
      sim.chat(plan.thread, 'codly', 'The conflict with your branch was not resolved. Codly took the unfinished merge back: the plan’s branch is as its tasks left it.');
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
    const unmet = t.depends_on.filter((d) => sim.tasks.get(d)?.status !== 'done');
    const reason = status === 'ready'
      ? unmet.length ? `Waiting on ${unmet.map((d) => sim.tasks.get(d)?.key).join(', ')}` : t.assignee ? 'Starting…' : 'Ready: a free agent picks it up'
      : status === 'cancelled' ? 'Cancelled by you' : stop ? 'stopped by you; Retry to resume' : null;
    const next = sim.patchTask(id, { status, status_reason: reason });
    // After the status: the worker that holds the task lets go at its next step.
    if ((stop || cancel) && t.assignee && this.running.has(t.assignee) && sim.agents.get(t.assignee)?.task_id === id) sim.cancelled.add(t.assignee);
    sim.decision('move', 'human', `You moved ${t.key} to ${status}`, { task: t.key, from, to: status }, { task: id });
    if (status === 'ready' && !unmet.length) void this.autoStart(next);
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
    this.sim.decision('gate', 'human', `You re-ran the checks of ${t.key}`, { task: t.key }, { task: id });
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
      agent = [...sim.agents.values()].find((a) => a.role !== 'lead' && !this.running.has(a.id) && a.status !== 'offline') ?? null;
      if (!agent) {
        const used = new Set([...sim.agents.values()].map((a) => a.color));
        const color = [5, 1, 6, 0].find((c) => !used.has(c)) ?? 5;
        agent = this.spawnWorker('claude', 'Eli', color, task).a;
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

  /** A new 1:1 agent. `named` is the name the hire wizard gave it (checked there to be free); without one it gets its CLI's stock name. */
  hire(provider: Provider, model: string | null = null, title: string | null = null, named: string | null = null): Agent {
    const sim = this.sim;
    const names = new Set([...sim.agents.values()].map((a) => a.name));
    let name = named ?? WORKER_NAMES[provider];
    if (!named) for (let i = 2; names.has(name); i++) name = `${WORKER_NAMES[provider]} ${i}`;
    const used = new Set([...sim.agents.values()].map((a) => a.color));
    const color = [0, 1, 3, 5, 2, 4, 7, 6].find((c) => !used.has(c)) ?? sim.agents.size % 8;
    const act = this.spawnWorker(provider, name, color, null, model);
    act.status('idle');
    // Like the core: a 1:1 agent works in a worktree of its own, called after its name (the
    // hire puts it in your project folder instead when Git isolation is off). Two names can
    // read the same in a path, so a path that is taken gets a number.
    const slug = name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'agent';
    const taken = new Set([...sim.agents.values()].map((a) => a.worktree));
    let worktree = `${WORKTREES}/${slug}`;
    for (let i = 2; taken.has(worktree); i++) worktree = `${WORKTREES}/${slug}-${i}`;
    return sim.patchAgent(act.a.id, { worktree, ...(title ? { title } : {}) });
  }

  /** `?mode=plan`: a 1:1 Claude agent in "plan first", asked for a small change, so its plan waits for your answer. */
  private seedPlanner() {
    const a = this.hire('claude', null, 'Docs');
    this.sim.patchAgent(a.id, { permission_mode: 'plan' });
    this.userChat(`agent:${a.id}`, 'Add a short “Theming” section to the README.');
  }

  /** Interrupt the agent's turn (Send now): its next queued message runs right after. */
  interrupt(agentId: string, stop = false) {
    const sim = this.sim;
    const a = sim.agents.get(agentId);
    if (!a) throw new Error('Unknown agent');
    const act = sim.actor(agentId);
    if (this.running.has(agentId) || this.busyLeads.has(agentId) || act.inTurn) sim.cancelled.add(agentId);
    else if (!stop) throw new Error(`${a.name} isn’t running anything.`);
    if (stop) {
      act.emit({ type: 'exited', code: 0 });
      act.set({ status: 'offline', activity: null, activity_label: null });
    }
  }

  // ───────────── diffs ─────────────

  taskDiff(id: string): string {
    const t = this.sim.tasks.get(id);
    if (!t) throw new Error('Unknown task');
    if (t.key === 'a3' && !t.plan_id) return diffOf(NEXT_BUMP);
    const fk = this.fileKey(t);
    const files = TASK_FILES[fk] ?? TASK_FILES.adhoc;
    const touched = this.sim.touched.get(fk) ?? new Set<string>();
    return diffOf(files.filter((f) => touched.has(f.path)));
  }

  planDiff(id: string): string {
    const tasks = [...this.sim.tasks.values()].filter((t) => t.plan_id === id && t.status === 'done');
    return diffOf(tasks.flatMap((t) => TASK_FILES[this.fileKey(t)] ?? []));
  }

  searchIndex() {
    const done = [...this.sim.tasks.values()].filter((t) => t.status === 'done').map((t) => this.fileKey(t));
    return [...INDEX, ...done.flatMap((k) => INDEX_AFTER[k] ?? [])];
  }
}
