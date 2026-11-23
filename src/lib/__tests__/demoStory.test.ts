// The browser demo's scripted story (demo/story.ts), run on fake timers: what it shows must be
// what the app does. Each test lets the script play and looks at the events it wrote, which
// are the ones the UI draws.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { rulePrefix } from '../demo/approvalRules';
import { ChecksDemo } from '../demo/checks';
import { demoDraft, hireFromManifest, reviewHire } from '../demo/hire';
import { demoIntake } from '../demo/intake';
import { MergeConflictDemo } from '../demo/mergeConflict';
import { PullRequestDemo } from '../demo/pullrequest';
import { REPO, Sim } from '../demo/sim';
import { demoStates, opensRepo } from '../demo/states';
import { AMEND, REQUEST, Story, type StoryOptions } from '../demo/story';
import type { Agent } from '../gen/Agent';
import type { Approval } from '../gen/Approval';
import type { EventKind } from '../gen/EventKind';
import type { PrDraft } from '../gen/PrDraft';
import type { Settings } from '../gen/Settings';
import { waitingOf, type Turn } from '../state/queue';
import { DEFAULT_SETTINGS } from '../state/store.svelte';

function demo(o: { settings?: Partial<Settings>; story?: StoryOptions; autoplay?: boolean } = {}) {
  const sim = new Sim({ speed: 1, skipMs: 0, autoplay: o.autoplay ?? false }, { ...DEFAULT_SETTINGS, ...o.settings });
  const story = new Story(sim, o.story);
  story.openWorkspace(REPO);
  return { sim, story };
}

/** Let the script play until `done`, for `seconds` of its time at most. `each` runs every half second (it answers what waits for you). */
async function until(done: () => boolean, each: () => void = () => {}, seconds = 400) {
  for (let i = 0; i < seconds * 2 && !done(); i++) {
    each();
    await vi.advanceTimersByTimeAsync(500);
  }
  expect(done(), 'the script got there in time').toBe(true);
}

/** The events of one kind, in order (of one agent when given). */
const events = <T extends EventKind['type']>(sim: Sim, type: T, agentId?: string) =>
  sim.log.filter((e) => e.kind.type === type && (!agentId || e.agent_id === agentId)).map((e) => e.kind as Extract<EventKind, { type: T }>);
const agent = (sim: Sim, test: (a: Agent) => boolean) => [...sim.agents.values()].find(test);
const lead = (sim: Sim) => agent(sim, (a) => a.role === 'lead' && a.thread === 'lead');
const named = (sim: Sim, name: string) => agent(sim, (a) => a.name === name);
const planOf = (sim: Sim, thread: string) => [...sim.plans.values()].find((p) => p.thread === thread);
const task = (sim: Sim, planId: string, key: string) => [...sim.tasks.values()].find((t) => t.plan_id === planId && t.key === key);
const requests = (sim: Sim, test: (a: Approval) => boolean) => [...sim.approvals.values()].filter(test);
const isCommand = (a: Approval, start: string) => a.subject.kind === 'command' && a.subject.command.startsWith(start);

/** You, saying yes to everything: plans run, and each request is allowed once. */
function sayYes(sim: Sim, story: Story) {
  for (const p of sim.plans.values()) if (p.status === 'awaiting_approval') story.runPlan(p.id);
  for (const a of requests(sim, (x) => x.status === 'pending')) story.resolveApproval(a.id, 'allow_once', null);
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date', 'performance'] });
  // Every pause and every length of a step is the same in each run.
  vi.spyOn(Math, 'random').mockReturnValue(0.5);
  // The story draws one picture on a canvas; a test has none.
  vi.stubGlobal('OffscreenCanvas', class { getContext() { return null; } });
});
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('a message sent while the agent’s turn runs', () => {
  it('is read by a Claude Lead after the step it is in, and gets no turn of its own', async () => {
    const { sim } = demo();
    await until(() => planOf(sim, 'lead')?.status === 'awaiting_approval');
    const id = lead(sim)!.id;
    expect(events(sim, 'prompt_steered', id).map((e) => e.input)).toEqual([AMEND]);
    // What the badge reads (state/queue.ts): the turn took the message in, so nothing waits.
    const turns: Turn[] = [];
    for (const e of sim.log.filter((x) => x.agent_id === id)) {
      if (e.kind.type === 'turn_started') turns.push({ start: e.ts, end: null, input: e.kind.input, steers: [] });
      if (e.kind.type === 'prompt_steered') turns[turns.length - 1].steers!.push({ ts: e.ts, input: e.kind.input });
      if (e.kind.type === 'turn_completed') turns[turns.length - 1].end = e.ts;
    }
    const sent = sim.chats.filter((c) => c.thread === 'lead' && c.author === 'user').map((c) => ({ key: c.id, ts: c.ts, text: c.text }));
    expect(sent.map((m) => m.text)).toEqual([REQUEST, AMEND]);
    expect(waitingOf(turns, sent, sim.now()).waiting).toEqual([]);
    // The plan the Lead submits has what the message asked for.
    const plan = planOf(sim, 'lead')!;
    expect(plan.request).toBe(`${REQUEST}\n\n${AMEND}`);
    expect(task(sim, plan.id, 't3')!.acceptance).toContain('Arrow keys move between the options (a radiogroup)');
    // Later, with the Lead at rest: still one turn, the one your first message started.
    await until(() => !sim.actor(id).inTurn);
    await vi.advanceTimersByTimeAsync(5000);
    expect(events(sim, 'turn_started', id).map((e) => e.input)).toEqual([REQUEST]);
  });

  it('waits for the turn’s end with a Lead whose CLI does not read mid-turn', async () => {
    const { sim } = demo({ settings: { lead_provider: 'gemini' } });
    await until(() => !!lead(sim) && events(sim, 'turn_completed', lead(sim)!.id).length === 2);
    const id = lead(sim)!.id;
    expect(events(sim, 'prompt_steered', id)).toEqual([]);
    expect(events(sim, 'turn_started', id).map((e) => e.input)).toEqual([REQUEST, AMEND]);
    expect(task(sim, planOf(sim, 'lead')!.id, 't3')!.acceptance).toContain('Arrow keys move between the options (a radiogroup)');
  });

  it('is read by a Claude worker during its task, which says so in your chat with it', async () => {
    const { sim, story } = demo({ autoplay: true });
    await until(() => !!named(sim, 'Ada') && sim.actor(named(sim, 'Ada')!.id).inTurn);
    const ada = named(sim, 'Ada')!.id;
    story.userChat(`agent:${ada}`, 'Use CSS variables only.');
    await until(() => events(sim, 'prompt_steered', ada).length === 1);
    expect(events(sim, 'prompt_steered', ada)[0].input).toBe('Use CSS variables only.');
    await until(() => sim.chats.some((c) => c.thread === `agent:${ada}` && c.author === ada));
    expect(sim.chats.find((c) => c.thread === `agent:${ada}` && c.author === ada)!.text).toMatch(/^Noted: “Use CSS variables only\.”/);
    await until(() => !!sim.taskByKey('t1') && sim.taskByKey('t1')!.status === 'done');
    await vi.advanceTimersByTimeAsync(5000);
    expect(events(sim, 'turn_started', ada).map((e) => e.input)).not.toContain('Use CSS variables only.');
  });
});

describe('“Always allow in this repo” in the demo', () => {
  it('offers the prefix of one plain command only, as the core does', () => {
    expect(rulePrefix('npm install -D zod')).toBe('npm install');
    expect(rulePrefix('npm run build -- --watch')).toBe('npm run build');
    expect(rulePrefix('cargo test -p core')).toBe('cargo test');
    expect(rulePrefix('git add -A')).toBe('git add');
    // A chain, a pipe, a substitution, a redirect, a program with no subcommands: no prefix.
    for (const c of ['npm install && npm test', 'pnpm install 2>&1 | tail -5', 'npm install $(cat x)', 'npm test > out.log', 'redis-cli -n 9 FLUSHDB', 'npm']) {
      expect(rulePrefix(c), c).toBeNull();
    }
    // What a prefix must never cover.
    for (const c of ['git push origin main', 'git checkout -B main', 'git switch --discard-changes x', 'make docs deploy', 'mix do a + b']) {
      expect(rulePrefix(c), c).toBeNull();
    }
  });

  it('a rule you save answers the waiting request and the next one it covers; a delete always asks', async () => {
    const { sim, story } = demo();
    const install = () => requests(sim, (a) => isCommand(a, 'npm install'));
    await until(() => install().length === 1, () => {
      // A delete can never be saved: the card offers nothing.
      for (const a of requests(sim, (x) => x.status === 'pending' && isCommand(x, 'rm -rf'))) expect(story.rules.offer(a.id)).toEqual({ exact: null, prefix: null });
      sayYes(sim, story);
    });
    expect(requests(sim, (a) => isCommand(a, 'rm -rf'))).toHaveLength(1);
    const first = install()[0];
    expect(first).toMatchObject({ status: 'pending', risks: ['network (package install)'] });
    expect(story.rules.offer(first.id)).toEqual({ exact: 'npm install -D @testing-library/react', prefix: 'npm install' });
    const rule = story.rules.add(first.id, 'prefix');
    expect(rule).toMatchObject({ kind: 'prefix', command: 'npm install' });
    expect(sim.approvals.get(first.id)).toMatchObject({ status: 'approved', decided_by: 'user' });

    // Another agent, another package: Codly does not ask you.
    await until(() => install().length === 2, () => sayYes(sim, story));
    const second = sim.approvals.get(install()[1].id)!;
    expect(second.subject).toMatchObject({ command: 'npm install -D @axe-core/playwright' });
    expect(second).toMatchObject({ status: 'approved', decided_by: 'policy' });
    expect(events(sim, 'approval_resolved').find((e) => e.approval_id === second.id)).toMatchObject({ status: 'approved', by: 'policy' });
    expect(events(sim, 'decision').filter((d) => d.decision_type === 'approval_policy').map((d) => d.summary)).toEqual([
      'you saved a rule: always allow commands that start with `npm install` in this repo',
      'auto-allowed your rule for this repo: commands that start with `npm install`',
    ]);

    // Removed: Codly asks again.
    story.rules.remove(rule.id);
    expect(story.rules.list()).toEqual([]);
    expect(() => story.rules.remove(rule.id)).toThrow('already removed');
  });

  it('offers nothing with “Ask first” permissions', async () => {
    const { sim, story } = demo({ settings: { approval_mode: 'strict' } });
    const install = () => requests(sim, (a) => isCommand(a, 'npm install'));
    await until(() => install().length === 1, () => sayYes(sim, story));
    expect(story.rules.offer(install()[0].id)).toEqual({ exact: null, prefix: null });
    expect(() => story.rules.add(install()[0].id, 'exact')).toThrow('Codly always asks about this kind of request');
  });
});

describe('a 1:1 Claude agent works in its mode', () => {
  it('“plan first” shows a plan, changes it when you send it back, and asks for edits once you accept', async () => {
    const { sim, story } = demo({ story: { planMode: true, yourRequestFirst: true } });
    const eli = () => agent(sim, (a) => a.role === 'solo')!;
    const plans = () => requests(sim, (a) => a.subject.kind === 'tool' && a.subject.name === 'ExitPlanMode');
    const text = (a: Approval) => (a.subject.kind === 'tool' ? (a.subject.input as { plan: string }).plan : '');
    await until(() => plans().length === 1);
    expect(eli()).toMatchObject({ provider: 'claude', permission_mode: 'plan' });
    expect(eli().worktree).toBeTruthy();
    expect(plans()[0]).toMatchObject({ status: 'pending', choices: ['allow_once', 'deny', 'cancel'], agent_id: eli().id });
    expect(text(plans()[0])).toContain('You asked: “Add a short “Theming” section to the README.”');

    // Sent back with a note: the plan comes again with the change, and the agent still plans.
    story.resolveApproval(plans()[0].id, 'deny', 'Say which file changes.');
    await until(() => plans().length === 2);
    expect(text(plans()[1])).toContain('Changed as you asked: Say which file changes.');
    expect(eli().permission_mode).toBe('plan');
    expect(requests(sim, (a) => a.subject.kind === 'file_change')).toEqual([]);

    // Accepted: Claude Code leaves plan mode by itself, and the edit asks first.
    story.resolveApproval(plans()[1].id, 'allow_once', null);
    const edits = () => requests(sim, (a) => a.subject.kind === 'file_change');
    await until(() => edits().length === 1);
    expect(eli().permission_mode).toBe('ask');
    expect(events(sim, 'decision').filter((d) => d.decision_type === 'permission_mode').map((d) => d.summary)).toEqual(['Claude Code changed its mode to "ask for edits"']);
    expect(edits()[0].subject).toMatchObject({ paths: ['README.md'] });
    story.resolveApproval(edits()[0].id, 'allow_once', null);
    const done = () => sim.chats.filter((c) => c.thread === `agent:${eli().id}` && c.author === eli().id && c.text.startsWith('Done:'));
    await until(() => done().length === 1);

    // In "accept edits" the same work asks nothing.
    sim.patchAgent(eli().id, { permission_mode: 'accept_edits' });
    story.userChat(`agent:${eli().id}`, 'Add a line about the changelog.');
    await until(() => done().length === 2);
    expect(sim.approvals.size).toBe(3);
  });

  it('“Stop the turn” on a plan ends the turn, and nothing changes', async () => {
    const { sim, story } = demo({ story: { planMode: true, yourRequestFirst: true } });
    const plans = () => requests(sim, (a) => a.subject.kind === 'tool');
    await until(() => plans().length === 1);
    const id = plans()[0].agent_id;
    story.resolveApproval(plans()[0].id, 'cancel', null);
    await until(() => events(sim, 'turn_completed', id).length === 1);
    expect(events(sim, 'turn_completed', id)[0].status).toBe('interrupted');
    expect(sim.agents.get(id)!.permission_mode).toBe('plan');
    expect(events(sim, 'files_changed', id)).toEqual([]);
    // No turn runs any more: the agent waits for you, it does not look busy.
    await until(() => sim.agents.get(id)!.status === 'idle');
    expect(sim.agents.get(id)!.activity).toBeNull();
  });

  it('“Stop the turn” on an edit ends the turn, and the agent waits for you again', async () => {
    const { sim, story } = demo({ story: { yourRequestFirst: true } });
    const id = story.hire('claude').id;
    story.userChat(`agent:${id}`, 'Add a line about the changelog.');
    const edits = () => requests(sim, (a) => a.subject.kind === 'file_change' && a.agent_id === id);
    await until(() => edits().length === 1);
    story.resolveApproval(edits()[0].id, 'cancel', null);
    await until(() => events(sim, 'turn_completed', id).length === 1);
    expect(events(sim, 'turn_completed', id)[0].status).toBe('interrupted');
    expect(events(sim, 'files_changed', id)).toEqual([]);
    await until(() => sim.agents.get(id)!.status === 'idle');
    expect(sim.agents.get(id)!.activity).toBeNull();
  });
});

describe('agents you hire in the demo', () => {
  it('each get a worktree of their own, called after their name', () => {
    const { sim, story } = demo({ story: { yourRequestFirst: true } });
    const wizard = (brief: string) => hireFromManifest(sim, story, reviewHire(demoDraft(brief)).manifest!, false);
    const dot = wizard('Keep the docs up to date');
    const tri = wizard('Triage new bugs');
    expect([dot.name, tri.name]).toEqual(['Dot', 'Tri']);
    expect(dot.worktree).toMatch(/\/dot$/);
    expect(tri.worktree).toMatch(/\/tri$/);
    // Two names that read the same in a path, and two hires without a name of their own.
    const twins = ['Dot 2', 'dot-2'].map((name) => hireFromManifest(sim, story, { ...reviewHire(demoDraft('docs')).manifest!, name }, false));
    const stock = [story.hire('claude'), story.hire('claude')];
    expect(stock.map((a) => a.name)).toEqual(['Eli', 'Eli 2']);
    const worktrees = [dot, tri, ...twins, ...stock].map((a) => a.worktree);
    expect(new Set(worktrees).size).toBe(worktrees.length);
    // With Git isolation off the agent works in your project folder.
    const here = hireFromManifest(sim, story, { ...reviewHire(demoDraft('Cut the release')).manifest!, isolate: false }, false);
    expect(here.worktree).toBe(REPO);
  });
});

describe('the demo states of docs/DEMO.md', () => {
  it('reads each parameter, and a value it does not know starts nothing', () => {
    const all = demoStates(new URLSearchParams('trust=ask&merge=conflict&preview=running&checks=on&helper=slow&mode=plan&intake=pr'));
    expect(all).toEqual({ trustAsk: true, mergeConflict: true, previewRunning: true, checksOn: true, slowHelper: true, planMode: true, intake: 'pull_request' });
    expect(demoStates(new URLSearchParams('intake=issue')).intake).toBe('issue');
    const none = demoStates(new URLSearchParams('merge=yes&preview=1&checks=off&helper=fast&mode=ask&intake=linear&speed=2'));
    expect(none).toEqual({ trustAsk: false, mergeConflict: false, previewRunning: false, checksOn: false, slowHelper: false, planMode: false, intake: null });
    // A state opens the demo repo at once; the trust question is asked when you open it.
    expect(opensRepo(none)).toBe(false);
    expect(opensRepo({ ...none, trustAsk: true })).toBe(false);
    expect(opensRepo({ ...none, checksOn: true })).toBe(true);
    expect(opensRepo({ ...none, intake: 'issue' })).toBe(true);
  });

  it('a helper shows its kind, its own calls and its report; `?checks=on` runs your checks for each task', async () => {
    const { sim } = demo({ autoplay: true, story: { checks: new ChecksDemo(true) } });
    await until(() => sim.taskByKey('t1')?.status === 'done');
    const ada = named(sim, 'Ada')!.id;
    const progress = events(sim, 'sub_agent_progress', ada);
    const helper = progress[0].call_id;
    expect(progress[0]).toMatchObject({ name: 'Explore', tool_call_id: null });
    // Three calls of the helper's own, each started just before the line that says whose it is.
    const own = progress.filter((p) => p.tool_call_id !== null);
    expect(own.map((p) => [p.step, p.tool_uses])).toEqual([
      ['Searching for code that reads the theme', 1],
      ['Reading src/styles/tokens.css', 2],
      ['Reading src/hooks/useMediaQuery.ts', 3],
    ]);
    const log = sim.log.filter((e) => e.agent_id === ada);
    for (const p of own) {
      const at = log.findIndex((e) => e.kind.type === 'sub_agent_progress' && e.kind.tool_call_id === p.tool_call_id);
      expect(log[at - 1].kind).toMatchObject({ type: 'tool_started', call_id: p.tool_call_id });
    }
    expect(progress[progress.length - 1].report).toMatch(/^Three places:/);
    expect(events(sim, 'sub_agent', ada).filter((s) => s.call_id === helper).map((s) => s.status)).toEqual(['running', 'completed']);

    // The task's own check first, then the repo's required checks.
    const t1 = sim.taskByKey('t1')!.id;
    const verified = sim.log.filter((e) => e.task_id === t1 && e.kind.type === 'verified').map((e) => (e.kind.type === 'verified' ? [e.kind.command, e.kind.ok] : []));
    expect(verified).toEqual([['npm test -- theme', true], ['pnpm check', true], ['pnpm test', true]]);
  });

  it('`?helper=slow` keeps the helper at work long enough to look at', async () => {
    const { sim } = demo({ autoplay: true, story: { slowHelper: true } });
    await until(() => events(sim, 'sub_agent').length === 1);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(events(sim, 'sub_agent').map((s) => s.status)).toEqual(['running']);
    await until(() => events(sim, 'sub_agent').length === 2);
  });

  it('`?merge=conflict`: the plan’s merge is refused until one more task of the plan resolved the conflict', async () => {
    const { sim, story } = demo({ autoplay: true });
    await until(() => planOf(sim, 'lead')?.status === 'done');
    const plan = planOf(sim, 'lead')!;
    const source = { kind: 'plan' as const, id: plan.id };
    // Without the parameter nothing conflicts, and there is nothing to resolve.
    const clean = new MergeConflictDemo(false, sim, story);
    expect(clean.state(source)).toMatchObject({ conflicts: [], resolving: null });
    expect(() => clean.refuse(plan.id)).not.toThrow();
    expect(() => clean.resolve(source)).toThrow('there is nothing to resolve; merge it');

    const merge = new MergeConflictDemo(true, sim, story);
    expect(merge.state(source)).toEqual({
      target: 'main', branch: 'codly/dark-mode/integration', repo: REPO, conflicts: ['src/app/settings/page.tsx'], resolving: null, failed_checks: [], checks_not_run: null, behind: 1,
    });
    expect(() => merge.refuse(plan.id)).toThrow('merging into main conflicts in src/app/settings/page.tsx; the merge was aborted and your branch is unchanged.');

    // "Have an agent resolve it": one more task, in the plan's own worktree, and the plan runs again.
    const during = merge.resolve(source);
    const resolving = sim.tasks.get(during.resolving!)!;
    expect(resolving).toMatchObject({ key: 'resolve', title: 'Resolve conflicts with main', write_scope: ['src/app/settings/page.tsx'], branch: plan.integration_branch, worktree: plan.integration_worktree });
    expect(resolving.verify).toEqual(['npm test -- theme', 'npm run build', 'npm test -- settings', 'npx playwright test settings-theme']);
    expect(during.conflicts).toEqual(['src/app/settings/page.tsx']);
    expect(sim.plans.get(plan.id)!.status).toBe('running');
    expect(sim.chats.filter((c) => c.thread === 'lead' && c.author === 'codly').pop()!.text).toMatch(/^main and this plan both changed src\/app\/settings\/page\.tsx\./);
    expect(() => merge.resolve(source)).toThrow('the plan is still running; wait until it is done');

    await until(() => sim.plans.get(plan.id)!.status === 'done');
    expect(sim.tasks.get(resolving.id)).toMatchObject({ status: 'done' });
    expect(sim.agents.get(sim.tasks.get(resolving.id)!.assignee!)!.role).toBe('worker');
    expect(merge.state(source)).toMatchObject({ conflicts: [], resolving: null });
    expect(() => merge.refuse(plan.id)).not.toThrow();
  });

  it('`?intake=issue`: your request is the first, and its pull request ends with “Closes #N”', async () => {
    const { sim, story } = demo({ story: { yourRequestFirst: true } });
    await vi.advanceTimersByTimeAsync(30_000);
    expect(sim.chats).toEqual([]);
    expect(sim.plans.size).toBe(0);

    const intake = demoIntake({ thread: 'lead', kind: 'issue', reference: '12', note: '' }, (thread, text, source) => story.userChat(thread, text, { intake: source }));
    expect(sim.chats[0].attachment).toEqual({ intake: intake.source });
    await until(() => planOf(sim, 'lead')?.status === 'awaiting_approval');
    const plan = planOf(sim, 'lead')!;
    story.runPlan(plan.id);
    const draft = new PullRequestDemo().handle('draft_pull_request', { source: { kind: 'plan', id: plan.id } }, sim) as PrDraft;
    expect(draft.body.endsWith('\n\nCloses #12\n')).toBe(true);
  });
});
