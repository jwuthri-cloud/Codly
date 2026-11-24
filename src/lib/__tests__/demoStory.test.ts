// The browser demo's scripted story (demo/story.ts), run on fake timers: what it shows must be
// what the app does. Each test lets the script play and looks at the events it wrote, which
// are the ones the UI draws.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { rulePrefix } from '../demo/approvalRules';
import { ChecksDemo } from '../demo/checks';
import claudeRs from '../../../crates/codly-core/src/adapters/claude.rs?raw';
import mergeConflictRs from '../../../crates/codly-core/src/orchestrator/merge_conflict.rs?raw';
import plansRs from '../../../crates/codly-core/src/orchestrator/plans.rs?raw';
import runsRs from '../../../crates/codly-core/src/orchestrator/runs.rs?raw';
import schedulerRs from '../../../crates/codly-core/src/orchestrator/scheduler.rs?raw';
import workspaceRs from '../../../crates/codly-core/src/orchestrator/workspace.rs?raw';
import { MockControls, NOT_RUNNING } from '../demo/controls';
import { demoDraft, hireFromManifest, reviewHire } from '../demo/hire';
import { demoIntake } from '../demo/intake';
import { MergeConflictDemo } from '../demo/mergeConflict';
import { PullRequestDemo } from '../demo/pullrequest';
import { REPO, Sim, WITHOUT_FOLDER_CONFIG } from '../demo/sim';
import { demoStates, opensRepo } from '../demo/states';
import { AMEND, AUTOPLAY, MERGE_FAILURES, REQUEST, REQUEST_B, REQUEST_CARDS, Story, type StoryOptions } from '../demo/story';
import { TASK_FILES } from '../demo/repo';
import { mockAgentDiff, mockAgentGit } from '../demo/agentgit';
import { outcomeBy } from '../util/outcome';
import { parseDiff } from '../util/diff';
import { startedWithoutFolderConfig } from '../state/session';
import type { Workspace } from '../gen/Workspace';
import type { Agent } from '../gen/Agent';
import type { Approval } from '../gen/Approval';
import type { EventKind } from '../gen/EventKind';
import type { PrDraft } from '../gen/PrDraft';
import type { Settings } from '../gen/Settings';
import { waitingOf, type Turn } from '../state/queue';
import { ofLiveRun, withMcpServer, type McpServer } from '../state/session';
import { DEFAULT_SETTINGS, type Decision } from '../state/store.svelte';
import { waitReason } from '../util/plan';

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
    // Each session starts in the folder its agent works in.
    for (const a of [dot, tri, ...twins, ...stock, here]) expect(events(sim, 'session_started', a.id).map((s) => s.cwd)).toEqual([a.worktree]);
  });

  it('a message to an agent that is gone does not wait for ever', () => {
    const { story } = demo({ story: { yourRequestFirst: true } });
    story.userChat('agent:nobody', 'Are you there?');
    // The story's own list of what no agent has read yet (private: nothing on screen shows it).
    expect((story as unknown as { unread: Map<string, unknown> }).unread.has('agent:nobody')).toBe(false);
  });
});

describe('`?approval=…`: a 1:1 agent asks that kind of request once', () => {
  const asking = (ask: StoryOptions['ask']) => demo({ story: { ask, yourRequestFirst: true } });
  const of = (sim: Sim, provider: Agent['provider']) => agent(sim, (a) => a.role === 'solo' && a.provider === provider)!;
  const all = (sim: Sim) => requests(sim, () => true);
  const said = (sim: Sim, id: string) => sim.chats.filter((c) => c.thread === `agent:${id}` && c.author === id).map((c) => c.text);
  /** The end of the call a request was for (the last call the agent started). */
  const call = (sim: Sim, id: string) => {
    const started = events(sim, 'tool_started', id).pop()!;
    return { started, ended: events(sim, 'tool_completed', id).find((e) => e.call_id === started.call_id) };
  };

  it('`file`: a Claude agent asks before its edit, with the change', async () => {
    const { sim, story } = asking('file');
    await until(() => all(sim).length === 1);
    const ap = all(sim)[0];
    expect(ap).toMatchObject({ status: 'pending', agent_id: of(sim, 'claude').id, choices: ['allow_once', 'deny', 'cancel'] });
    expect(ap.subject).toMatchObject({ kind: 'file_change', paths: ['README.md'] });
    expect(ap.subject.kind === 'file_change' && ap.subject.diff).toContain('+## Notes');
    story.resolveApproval(ap.id, 'allow_once', null);
    expect(sim.approvals.get(ap.id)).toMatchObject({ status: 'approved', decided_by: 'user' });
    await until(() => said(sim, ap.agent_id).length === 1);
    expect(events(sim, 'files_changed', ap.agent_id).map((e) => e.paths)).toEqual([['README.md']]);
    expect(all(sim)).toHaveLength(1);
  });

  it('`plan`: a Claude agent shows its plan (ExitPlanMode), as `?mode=plan` does', async () => {
    const { sim, story } = asking('plan');
    await until(() => all(sim).length === 1);
    const ap = all(sim)[0];
    expect(of(sim, 'claude').permission_mode).toBe('plan');
    expect(ap.subject).toMatchObject({ kind: 'tool', name: 'ExitPlanMode' });
    expect(ap.subject.kind === 'tool' && (ap.subject.input as { plan: string }).plan).toContain('# Plan');
    story.resolveApproval(ap.id, 'deny', null);
    expect(sim.approvals.get(ap.id)!.status).toBe('denied');
  });

  it('`network`: Claude Code’s sandbox asks for a host on behalf of a command, and the step is that command', async () => {
    const { sim, story } = asking('network');
    await until(() => all(sim).length === 1);
    const claude = of(sim, 'claude');
    const sandbox = all(sim)[0];
    const subject = { kind: 'tool', name: 'SandboxNetworkAccess', input: { host: 'registry.npmjs.org' } };
    // The sandbox's request asks in the CLI's words and has an id of its own as its call: no
    // call of the agent has that id. The command it holds up is the agent's Bash call.
    expect(sandbox).toMatchObject({
      subject, reason: 'Allow network connection to registry.npmjs.org?', risks: ['network (registry.npmjs.org)'], choices: ['allow_once', 'deny', 'cancel'], status: 'pending',
    });
    expect(sandbox.call_id).toEqual(expect.any(String));
    expect(events(sim, 'tool_started', claude.id).map((e) => e.call_id)).not.toContain(sandbox.call_id);
    expect(call(sim, claude.id).started).toMatchObject({ name: 'Bash', activity: 'execute', input: { command: 'npm view react version' } });
    // No step of the thread is named after the request: it is the command's.
    expect(events(sim, 'tool_started').map((e) => e.name)).not.toContain('SandboxNetworkAccess');

    story.resolveApproval(sandbox.id, 'allow_once', null);
    await until(() => said(sim, claude.id).length === 1);
    expect(call(sim, claude.id).ended).toMatchObject({ ok: true, output: '19.2.3', exit_code: 0 });
    expect(said(sim, claude.id)).toEqual(['The newest react on npm is 19.2.3.']);
  });

  it('`network`: after a "no" the command ends with the sandbox’s network error', async () => {
    const { sim, story } = asking('network');
    await until(() => all(sim).length === 1);
    const id = of(sim, 'claude').id;
    story.resolveApproval(all(sim)[0].id, 'deny', null);
    await until(() => said(sim, id).length === 1);
    const ended = call(sim, id).ended!;
    expect(ended).toMatchObject({ ok: false, exit_code: 1 });
    expect(ended.output).toContain('npm error 403 403 user denied - GET https://registry.npmjs.org/react');
    expect(ended.output).toContain('<sandbox_violations>\ndeny network-outbound registry.npmjs.org:443 (user denied)\n</sandbox_violations>');
    expect(said(sim, id)).toEqual(['The sandbox did not let npm reach registry.npmjs.org, so I cannot tell from here.']);
    expect(agent(sim, (a) => a.provider === 'gemini')).toBeUndefined();
  });

  it('`network`: “Stop the turn” ends the command with a network error, then the turn', async () => {
    const { sim, story } = asking('network');
    await until(() => all(sim).length === 1);
    const id = of(sim, 'claude').id;
    story.resolveApproval(all(sim).find((a) => a.agent_id === id)!.id, 'cancel', null);
    await until(() => events(sim, 'turn_completed', id).length === 1);
    expect(events(sim, 'turn_completed', id)[0].status).toBe('interrupted');
    expect(call(sim, id).ended).toMatchObject({ ok: false, exit_code: 1 });
    expect(said(sim, id)).toEqual([]);
    await until(() => sim.agents.get(id)!.status === 'idle');
  });

  it('`permissions`: a Codex agent asks to write outside its worktree', async () => {
    for (const choice of ['allow_session', 'deny'] as const) {
      const { sim, story } = asking('permissions');
      await until(() => all(sim).length === 1);
      const codex = of(sim, 'codex');
      const ap = all(sim)[0];
      expect(ap).toMatchObject({ agent_id: codex.id, status: 'pending', choices: ['allow_once', 'allow_session', 'deny'], reason: 'The changelog is in ../acme-docs, outside my worktree.' });
      expect(ap.subject).toMatchObject({ kind: 'permissions', detail: { cwd: codex.worktree, permissions: { network: null, fileSystem: { read: null, write: ['/Users/demo/Code/acme-docs'] } } } });
      story.resolveApproval(ap.id, choice, null);
      await until(() => said(sim, codex.id).length === 1);
      expect(sim.approvals.get(ap.id)!.status).toBe(choice === 'deny' ? 'denied' : 'approved');
      expect(events(sim, 'files_changed', codex.id).map((e) => e.paths)).toEqual(choice === 'deny' ? [] : [['/Users/demo/Code/acme-docs/CHANGELOG.md']]);
    }
  });

  it('`question-long`: long options to pick from, and a question with a free answer', async () => {
    const { sim, story } = asking('question-long');
    await until(() => all(sim).length === 2);
    const [claude, codex] = [of(sim, 'claude'), of(sim, 'codex')];
    const pick = all(sim).find((a) => a.agent_id === claude.id)!;
    const free = all(sim).find((a) => a.agent_id === codex.id)!;
    type Asked = { questions: { questions: { question: string; options: { label: string; description: string }[] }[] } };
    const q = (pick.subject as unknown as Asked).questions.questions[0];
    expect(pick.subject.kind).toBe('question');
    expect(q.options).toHaveLength(2);
    for (const o of q.options) expect(Math.min(o.label.length, o.description.length)).toBeGreaterThanOrEqual(110);
    // No list of questions in it: the card shows the request as it is, with a field for your answer.
    expect(free.subject).toMatchObject({ kind: 'question', questions: { serverName: 'linear', mode: 'form' } });
    expect(Array.isArray((free.subject as { questions: { questions?: unknown } }).questions.questions)).toBe(false);

    story.resolveApproval(pick.id, 'allow_once', null, { [q.question]: q.options[1].label });
    const answer = 'The payments team, because the checkout test fails in the Stripe webhook and not in the page, so the web team cannot fix it.';
    expect(answer.length).toBeGreaterThanOrEqual(120);
    story.resolveApproval(free.id, 'allow_once', null, { answer });
    await until(() => said(sim, claude.id).length === 1 && said(sim, codex.id).length === 1);
    expect(said(sim, claude.id)[0]).toMatch(/^Two notes then/);
    expect(said(sim, codex.id)[0]).toContain(answer);
    expect(all(sim).every((a) => a.status === 'approved')).toBe(true);
  });
});

describe('the demo’s second office, billing-api', () => {
  it('is in the middle of its plan when you open it, with one request waiting, and plays on to the end', async () => {
    const sim = new Sim({ speed: 1, skipMs: 0, autoplay: false }, { ...DEFAULT_SETTINGS });
    const story = new Story(sim, { billing: true });
    const ws = story.openWorkspace('~/code/billing-api');
    expect(ws).toMatchObject({ id: 'demo-office-billing', name: 'billing-api', repo_path: '~/code/billing-api' });
    // At once, before any of its time passes: what its tab said (three agents, two tasks in flight, one request).
    expect([...sim.agents.values()].filter((a) => a.role !== 'lead').map((a) => a.name).sort()).toEqual(['Otto', 'Pia', 'Rex']);
    expect(sim.chats.map((c) => [c.thread, c.author])).toEqual([['lead', 'user'], ['lead', 'codly']]);
    const plan = planOf(sim, 'lead')!;
    expect(plan).toMatchObject({ status: 'running', summary: 'Retries for failed invoice payments', integration_branch: 'codly/payment-retries/integration' });
    expect([...sim.tasks.values()].map((t) => [t.key, t.status, sim.agents.get(t.assignee!)!.name])).toEqual([['t1', 'running', 'Otto'], ['t2', 'running', 'Rex']]);
    const waiting = requests(sim, (a) => a.status === 'pending');
    expect(waiting).toHaveLength(1);
    expect(waiting[0]).toMatchObject({ agent_id: named(sim, 'Otto')!.id, subject: { kind: 'command', command: 'docker compose up -d stripe-mock' } });
    expect(waiting[0].subject.kind === 'command' && waiting[0].subject.cwd).toBe(named(sim, 'Otto')!.worktree);
    expect(named(sim, 'Otto')!.worktree).toBe('/Users/demo/.codly/worktrees/billing-api/t1-otto');
    // Otto is a Codex agent: its command is Codex's.
    expect(events(sim, 'tool_started', named(sim, 'Otto')!.id).find((e) => e.call_id === waiting[0].call_id)!.name).toBe('commandExecution');

    // Rex finishes without you; Otto waits for your answer.
    await until(() => task(sim, plan.id, 't2')!.status === 'done');
    expect(task(sim, plan.id, 't1')!.status).toBe('running');
    story.resolveApproval(waiting[0].id, 'allow_once', null);
    await until(() => sim.plans.get(plan.id)!.status === 'done');
    // Each task was started once, by the worker that had it when you came: nobody new, no second request.
    const crew = [...sim.agents.values()].filter((a) => a.role !== 'lead');
    expect(crew.map((a) => a.name).sort()).toEqual(['Otto', 'Pia', 'Rex']);
    expect(crew.map((a) => events(sim, 'turn_started', a.id).length).sort()).toEqual([0, 1, 1]);
    expect(sim.approvals.size).toBe(1);
    expect(story.planDiff(plan.id)).toContain('+++ b/src/invoices/retry.ts');
    expect(story.taskDiff(task(sim, plan.id, 't2')!.id)).toContain('+++ b/src/mail/paymentFailed.ts');
    await until(() => sim.chats.some((c) => c.thread === 'lead' && c.author === lead(sim)!.id));
    expect(sim.chats.filter((c) => c.author === lead(sim)!.id).pop()!.text).toMatch(/^Both tasks are merged into `codly\/payment-retries\/integration`/);

    // What you ask there gets an answer, and no plan of acme/web.
    story.userChat('lead:next', 'Add a dark mode toggle to Settings.');
    await until(() => sim.chats.some((c) => c.thread === 'lead:next' && c.author !== 'user'));
    expect(sim.chats.find((c) => c.thread === 'lead:next' && c.author !== 'user')!.text).toContain('billing-api has the one you see');
    expect(sim.plans.size).toBe(1);
  });
});

describe('a request for a command', () => {
  it('names the shell tool of the agent’s own CLI', async () => {
    const { sim } = demo({ autoplay: true });
    const commands = () => requests(sim, (a) => a.subject.kind === 'command');
    const providers = () => new Set(commands().map((a) => sim.agents.get(a.agent_id)!.provider));
    await until(() => ['claude', 'codex', 'gemini'].every((p) => providers().has(p as Agent['provider'])));
    const tools = commands().map((a) => [sim.agents.get(a.agent_id)!.provider, events(sim, 'tool_started', a.agent_id).find((e) => e.call_id === a.call_id)!.name]);
    const NAME: Record<string, string> = { claude: 'Bash', codex: 'commandExecution', gemini: 'run_shell_command' };
    for (const [provider, name] of tools) expect(name, provider).toBe(NAME[provider]);
  });
});

describe('the MCP servers of the first Claude Lead', () => {
  it('are reported when its session starts, and a snapshot still has them when the log has moved on', async () => {
    const { sim } = demo();
    await until(() => !!lead(sim));
    const id = lead(sim)!.id;
    const rows = sim.log.filter((e) => e.agent_id === id && e.kind.type === 'mcp_server');
    expect(rows.map((e) => (e.kind.type === 'mcp_server' ? [e.kind.name, e.kind.status, e.kind.error] : []))).toEqual([
      ['codly', 'connected', null], ['linear', 'needs_sign_in', null], ['postgres', 'failed', 'MCP error -32000: Connection closed'],
    ]);
    // Each belongs to the run of the Lead's process: the agent sheet shows the servers of the live run.
    expect(rows.every((e) => e.run_id === lead(sim)!.run_id)).toBe(true);
    expect(sim.snapshot().session_events).toEqual([]);
    for (let i = 0; i < 2000; i++) sim.decision('note', 'rule', `filler ${i}`, {});
    const snap = sim.snapshot();
    expect(snap.events.some((e) => e.kind.type === 'mcp_server')).toBe(false);
    expect(snap.session_events).toEqual(rows);
  });
});

describe('the MCP servers of a Lead you stop', () => {
  it('are gone with its run, and are reported again when it works again', async () => {
    const { sim, story } = demo();
    await until(() => planOf(sim, 'lead')?.status === 'awaiting_approval' && lead(sim)!.status === 'idle');
    const id = lead(sim)!.id;
    /** The agent sheet's list: the servers of the Lead's live run (sheet/AgentMcp.svelte). */
    const servers = () => {
      const list = sim.log.reduce<McpServer[]>((held, e) => (e.agent_id === id && e.kind.type === 'mcp_server' ? withMcpServer(held, e.kind, e.run_id, e.ts) : held), []);
      return ofLiveRun(list, lead(sim)!.run_id).map((x) => [x.name, x.status]);
    };
    const all = [['codly', 'connected'], ['linear', 'needs_sign_in'], ['postgres', 'failed']];
    expect(servers()).toEqual(all);
    const first = lead(sim)!.run_id;

    story.interrupt(id, true);
    // As in the app: the run ended with the process, and the sheet shows the servers of the live run.
    expect(lead(sim)).toMatchObject({ status: 'offline', run_id: null });
    expect(servers()).toEqual([]);

    // Your next message starts its process again: a new run, whose CLI says its servers again.
    story.userChat('lead', 'Is the plan ready?');
    await until(() => !!lead(sim)!.run_id);
    expect(lead(sim)!.run_id).not.toBe(first);
    expect(servers()).toEqual(all);
    const turn = sim.log.filter((e) => e.agent_id === id && e.kind.type === 'turn_started').pop()!;
    expect(turn.run_id).toBe(lead(sim)!.run_id);
  });
});

describe('the demo states of docs/DEMO.md', () => {
  it('reads each parameter, and a value it does not know starts nothing', () => {
    const all = demoStates(new URLSearchParams('trust=ask&merge=conflict&preview=running&checks=on&helper=slow&mode=plan&intake=pr&approval=question-long&hour=21.5&resume=ask&background=on&turn=failed&pr=open&search=fail&requests=two&cards=3&long=300'));
    expect(all).toEqual({
      trustAsk: true, mergeConflict: true, mergeChecks: false, previewRunning: true, checksOn: true, slowHelper: true, planMode: true, intake: 'pull_request', approval: 'question-long', hour: 21.5,
      resumeAsk: true, background: true, turnFailed: true, prOpen: true, searchFails: true, twoRequests: true, threeCards: true, longThread: 300,
    });
    expect(demoStates(new URLSearchParams('merge=checks'))).toMatchObject({ mergeConflict: false, mergeChecks: true });
    expect(demoStates(new URLSearchParams('intake=issue')).intake).toBe('issue');
    const none = demoStates(new URLSearchParams('merge=yes&preview=1&checks=off&helper=fast&mode=ask&intake=linear&speed=2&approval=all&hour=24&resume=always&background=off&turn=ok&pr=closed&search=ok&requests=2&cards=2'));
    expect(none).toEqual({
      trustAsk: false, mergeConflict: false, mergeChecks: false, previewRunning: false, checksOn: false, slowHelper: false, planMode: false, intake: null, approval: null, hour: null,
      resumeAsk: false, background: false, turnFailed: false, prOpen: false, searchFails: false, twoRequests: false, threeCards: false, longThread: null,
    });
    // `?long` with no count is 2000 turns.
    expect(['long', 'long=', 'long=0', 'long=12'].map((q) => demoStates(new URLSearchParams(q)).longThread)).toEqual([2000, 2000, 2000, 12]);
    // Midnight is an hour; a parameter with no value, or with words, is none.
    expect(['hour=0', 'hour=', 'hour=noon', 'hour=-1'].map((q) => demoStates(new URLSearchParams(q)).hour)).toEqual([0, null, null, null]);
    // A state opens the demo repo at once; the trust question is asked when you open it, and the hour only moves the clock.
    expect(opensRepo(none)).toBe(false);
    expect(opensRepo({ ...none, trustAsk: true })).toBe(false);
    expect(opensRepo({ ...none, hour: 13 })).toBe(false);
    expect(opensRepo({ ...none, checksOn: true })).toBe(true);
    expect(opensRepo({ ...none, intake: 'issue' })).toBe(true);
    expect(opensRepo({ ...none, approval: 'file' })).toBe(true);
    expect(opensRepo({ ...none, resumeAsk: true })).toBe(true);
    expect(opensRepo({ ...none, background: true })).toBe(true);
    for (const on of ['mergeChecks', 'turnFailed', 'prOpen', 'searchFails', 'twoRequests', 'threeCards'] as const) expect(opensRepo({ ...none, [on]: true })).toBe(true);
    expect(opensRepo({ ...none, longThread: 2000 })).toBe(true);
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
    const clean = new MergeConflictDemo(null, sim, story);
    expect(clean.state(source)).toMatchObject({ conflicts: [], resolving: null });
    expect(() => clean.refuse(plan.id)).not.toThrow();
    expect(() => clean.resolve(source)).toThrow('there is nothing to resolve; merge it');

    const merge = new MergeConflictDemo('conflict', sim, story);
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
    // The worker that takes the task works in the plan's worktree for that task only.
    const worker = () => sim.agents.get(sim.tasks.get(resolving.id)!.assignee!)!;
    await until(() => sim.tasks.get(resolving.id)!.status === 'running');
    expect(worker().worktree).toBe(plan.integration_worktree);

    await until(() => sim.plans.get(plan.id)!.status === 'done');
    expect(sim.tasks.get(resolving.id)).toMatchObject({ status: 'done' });
    expect(worker().role).toBe('worker');
    // Then it has its own worktree again (the one of its last task): its Git tab is not the plan's.
    expect(worker().worktree).toMatch(/\/t[123]-[a-z]+$/);
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

describe('a plan whose request started from GitHub', () => {
  /** What the plan card reads (state/store.svelte.ts keeps each `decision` event this way). */
  const decisions = (sim: Sim): Decision[] =>
    sim.log.flatMap((e) => (e.kind.type === 'decision' ? [{ id: e.kind.id, type: e.kind.decision_type, method: e.kind.method, summary: e.kind.summary, detail: e.kind.detail, ts: e.ts, agentId: e.agent_id, taskId: e.task_id, eventId: e.id }] : []));
  const fromIssue = (story: Story) => demoIntake({ thread: 'lead', kind: 'issue', reference: '12', note: '' }, (thread, text, source) => story.userChat(thread, text, { intake: source }));

  it('waits in a request that runs its plans at once, with the reason on its card, and “Run plan” starts it', async () => {
    const { sim, story } = demo({ settings: { auto_run_plans: true }, story: { yourRequestFirst: true } });
    fromIssue(story);
    await until(() => planOf(sim, 'lead')?.status === 'awaiting_approval');
    const plan = planOf(sim, 'lead')!;
    // The core's record (`submit_plan_inner`), and what the card makes of it above Run plan.
    const said = events(sim, 'decision').filter((d) => d.decision_type === 'plan_validate');
    expect(said.map((d) => [d.method, d.summary, d.detail])).toEqual([
      ['rule', 'not auto-running: this request started from GitHub, and other people write there, so the plan waits for you', { rule: 'intake', plan_id: plan.id }],
    ]);
    expect(waitReason(decisions(sim), plan.id)).toBe('This request started from GitHub, and other people write there, so this plan did not start by itself. Read it, then run it if it is what you want.');
    expect(sim.chats.find((c) => c.author === 'codly')!.text).toBe(
      'Plan ready: Dark mode toggle in Settings (3 tasks). This request started from GitHub, and other people write there, so it waits for you: review it on the board, then click Run plan.',
    );
    // It does not start by itself, however long you wait.
    await vi.advanceTimersByTimeAsync(30_000);
    expect(sim.plans.get(plan.id)!.status).toBe('awaiting_approval');
    story.runPlan(plan.id);
    await until(() => task(sim, plan.id, 't1')!.status === 'running');
    expect(sim.plans.get(plan.id)!.status).toBe('running');
  });

  it('has no reason on its card in a request that asks you before every plan', async () => {
    // The Settings default: every plan waits for you, and the card says nothing more.
    const asks = demo({ story: { yourRequestFirst: true } });
    fromIssue(asks.story);
    await until(() => planOf(asks.sim, 'lead')?.status === 'awaiting_approval');
    expect(waitReason(decisions(asks.sim), planOf(asks.sim, 'lead')!.id)).toBeNull();
    expect(asks.sim.chats.find((c) => c.author === 'codly')!.text).toBe('Plan ready: Dark mode toggle in Settings (3 tasks). Review it on the board, then click Run plan.');
  });
});

describe('a typed request in a request that runs its plans at once', () => {
  const said = (sim: Sim) => sim.chats.filter((c) => c.author === 'codly').map((c) => c.text);
  const records = (sim: Sim) => events(sim, 'decision').filter((d) => d.decision_type === 'plan_validate').map((d) => [d.summary, d.detail]);

  it('runs a plan that asks you nothing, and Codly’s line says so', async () => {
    const { sim, story } = demo({ settings: { auto_run_plans: true }, story: { yourRequestFirst: true } });
    story.userChat('lead', REQUEST_B);
    await until(() => planOf(sim, 'lead')?.status === 'running');
    expect(said(sim)).toEqual(['Plan ready: Sign-in lockout after failed attempts (2 tasks). Running it now.']);
    expect(records(sim)).toEqual([]);
  });

  it('does not run a plan with an open question: it waits for you, and the record says why', async () => {
    const { sim, story } = demo({ settings: { auto_run_plans: true }, story: { yourRequestFirst: true } });
    story.userChat('lead', REQUEST);
    await until(() => planOf(sim, 'lead')?.status === 'awaiting_approval');
    const plan = planOf(sim, 'lead')!;
    expect(plan.open_questions).toHaveLength(1);
    // The core's record (`submit_plan_inner`, rule `open_questions`) and its line.
    expect(records(sim)).toEqual([['not auto-running: the plan has open questions for you', { rule: 'open_questions', plan_id: plan.id }]]);
    expect(said(sim)).toEqual(['Plan ready: Dark mode toggle in Settings (3 tasks). Review it on the board, then click Run plan.']);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(sim.plans.get(plan.id)!.status).toBe('awaiting_approval');
  });
});

describe('a helper and the turn you stop', () => {
  it('a helper whose call waits for it ends with the turn, and its row says “stopped” after the turn’s end', async () => {
    const { sim, story } = demo({ autoplay: true, story: { slowHelper: true } });
    await until(() => events(sim, 'sub_agent').length === 1);
    const ada = named(sim, 'Ada')!.id;
    story.interrupt(ada);
    await until(() => events(sim, 'turn_completed', ada).length === 1);
    expect(events(sim, 'turn_completed', ada)[0].status).toBe('interrupted');
    // As Claude Code's adapter says it: the row of the helper, after the end of the turn.
    const log = sim.log.filter((e) => e.agent_id === ada);
    const rows = events(sim, 'sub_agent', ada);
    expect(rows.map((s) => s.status)).toEqual(['running', 'stopped']);
    expect(rows[1]).toMatchObject({ call_id: rows[0].call_id, description: rows[0].description });
    expect(log.findIndex((e) => e.kind.type === 'sub_agent' && e.kind.status === 'stopped')).toBeGreaterThan(log.findIndex((e) => e.kind.type === 'turn_completed'));
    // Said once: the helper does not report later.
    await vi.advanceTimersByTimeAsync(120_000);
    expect(events(sim, 'sub_agent', ada).map((s) => s.status)).toEqual(['running', 'stopped']);
  });

  it('a helper that finished before the stop keeps its word', async () => {
    const { sim, story } = demo({ autoplay: true });
    await until(() => events(sim, 'sub_agent').length === 2);
    const ada = named(sim, 'Ada')!.id;
    story.interrupt(ada);
    await until(() => events(sim, 'turn_completed', ada).length === 1);
    expect(events(sim, 'sub_agent', ada).map((s) => s.status)).toEqual(['running', 'completed']);
  });
});

describe('who takes a task', () => {
  it('a task with no worker goes to a free worker: never to a 1:1 agent you hired, never to an agent in a turn', async () => {
    const { sim, story } = demo({ story: { yourRequestFirst: true } });
    // Two 1:1 agents, at rest: the first agents of the office. One has the stock name of a Claude agent.
    const solo = story.hire('codex');
    const eli = story.hire('claude');
    expect(eli.name).toBe('Eli');
    const start = async (title: string) => {
      const t = story.createTask(title, '');
      story.moveTask(t.id, 'ready');
      await until(() => !!sim.tasks.get(t.id)!.assignee);
      return sim.agents.get(sim.tasks.get(t.id)!.assignee!)!;
    };
    const first = await start('Sort the imports');
    expect([solo.id, eli.id]).not.toContain(first.id);
    // The worker started for it gets a name of its own, as a hire does: "Eli" is taken.
    expect([first.name, first.role, first.worktree]).toEqual(['Eli 2', 'worker', expect.stringMatching(/\/a4-eli-2$/)]);
    for (const a of [solo, eli]) expect(sim.agents.get(a.id)).toMatchObject({ role: 'solo', task_id: null, status: 'idle' });

    // That worker is free again after its task, and takes the next one.
    await until(() => sim.taskByKey('a4')?.status === 'done');
    await vi.advanceTimersByTimeAsync(5000);
    expect((await start('Rename the helper')).id).toBe(first.id);
    await until(() => sim.taskByKey('a5')?.status === 'done');
    await vi.advanceTimersByTimeAsync(5000);

    // While it answers you (a turn runs), a new worker is started for the task.
    sim.actor(first.id).inTurn = true;
    const third = await start('Update the changelog');
    expect([first.id, solo.id, eli.id]).not.toContain(third.id);
    expect([third.name, third.role]).toEqual(['Eli 3', 'worker']);
    // Each name still means one agent.
    expect(sim.agentByName('Eli')!.id).toBe(eli.id);
    expect(new Set([...sim.agents.values()].map((a) => a.name)).size).toBe(sim.agents.size);
  });

  it('an agent started only to resolve a conflict works in the plan’s worktree, and has none of its own afterwards', async () => {
    const { sim, story } = demo({ autoplay: true });
    await until(() => planOf(sim, 'lead')?.status === 'done');
    const plan = planOf(sim, 'lead')!;
    // No worker is free: you stopped each of them.
    for (const a of [...sim.agents.values()].filter((x) => x.role === 'worker')) story.interrupt(a.id, true);
    const resolving = story.resolveConflict(plan.id, 'main', ['src/app/settings/page.tsx']);
    await until(() => sim.tasks.get(resolving.id)!.status === 'running');
    const eli = sim.agents.get(sim.tasks.get(resolving.id)!.assignee!)!;
    expect([eli.name, eli.role, eli.worktree]).toEqual(['Eli', 'worker', plan.integration_worktree]);
    expect(events(sim, 'session_started', eli.id).map((s) => s.cwd)).toEqual([plan.integration_worktree]);

    await until(() => sim.plans.get(plan.id)!.status === 'done');
    await vi.advanceTimersByTimeAsync(5000);
    expect(sim.tasks.get(resolving.id)!.status).toBe('done');
    // The workers you stopped stay stopped (Cy's last task had only just landed).
    expect(['Ada', 'Bo', 'Cy'].map((name) => named(sim, name)!.status)).toEqual(['offline', 'offline', 'offline']);
    // No worktree that nobody worked in is left with its name.
    expect(sim.agents.get(eli.id)!.worktree).toBeNull();
  });
});

describe('your re-run of a stopped task’s checks', () => {
  it('is on record in the core’s words', () => {
    const { sim, story } = demo({ story: { yourRequestFirst: true } });
    story.recheckTask(sim.taskByKey('a2')!.id);
    // crates/codly-core/src/orchestrator/gate.rs, `Rerun::You`: "you re-ran the checks of {key}".
    expect(events(sim, 'decision').filter((d) => d.decision_type === 'gate').map((d) => [d.method, d.summary])).toEqual([['human', 'you re-ran the checks of a2']]);
  });
});

describe('a 1:1 Claude agent you said no to', () => {
  const said = (sim: Sim, id: string) => sim.chats.filter((c) => c.thread === `agent:${id}` && c.author === id).map((c) => c.text);
  async function denied(note: string | null) {
    const { sim, story } = demo({ story: { yourRequestFirst: true } });
    const id = story.hire('claude').id;
    story.userChat(`agent:${id}`, 'Add a line about the changelog.');
    const edit = () => requests(sim, (a) => a.subject.kind === 'file_change' && a.agent_id === id)[0];
    await until(() => !!edit());
    story.resolveApproval(edit().id, 'deny', note);
    await until(() => said(sim, id).length === 1);
    expect(events(sim, 'files_changed', id)).toEqual([]);
    return said(sim, id)[0];
  }

  it('takes up your note when you sent one', async () => {
    expect(await denied('Put it in CHANGELOG.md instead, under Unreleased.')).toBe('Understood: “Put it in CHANGELOG.md instead, under Unreleased.”. I will do that.');
  });

  it('asks what to do instead when you sent none', async () => {
    expect(await denied(null)).toBe('Understood: I changed nothing. Tell me what to do instead.');
  });
});

describe('the list of a 1:1 Claude agent', () => {
  it('has four steps, and the agent ticks them off as it works', async () => {
    const { sim, story } = demo({ story: { yourRequestFirst: true } });
    const id = story.hire('claude').id;
    sim.patchAgent(id, { permission_mode: 'accept_edits' });
    expect(sim.edited.has(id)).toBe(false);
    story.userChat(`agent:${id}`, 'Add a line about the changelog.');
    await until(() => events(sim, 'turn_completed', id).length === 1);
    const lists = events(sim, 'plan_updated', id).map((p) => p.steps.map((s) => s.status));
    expect(events(sim, 'plan_updated', id)[0].steps.map((s) => s.text)).toEqual(['Read README.md and package.json', 'Add the note to README.md', 'Run the tests', 'Tell you what changed']);
    expect(lists).toEqual([
      ['in_progress', 'pending', 'pending', 'pending'],
      ['completed', 'in_progress', 'pending', 'pending'],
      ['completed', 'completed', 'in_progress', 'pending'],
      ['completed', 'completed', 'completed', 'in_progress'],
      ['completed', 'completed', 'completed', 'completed'],
    ]);
    // Its work changed a file: from now on it has checkpoints (mock.ts reads `edited`).
    expect(sim.edited.has(id)).toBe(true);
  });
});

describe('`?turn=failed`: the first turn of a 1:1 agent fails', () => {
  it('ends failed with the reason, as the adapter writes it, and sending again works', async () => {
    const { sim, story } = demo({ story: { yourRequestFirst: true, failFirstTurn: true } });
    const id = story.hire('claude').id;
    sim.patchAgent(id, { permission_mode: 'accept_edits' });
    const ask = 'Add a line about the changelog.';
    story.userChat(`agent:${id}`, ask);
    await until(() => events(sim, 'turn_completed', id).length === 1);
    const reason = "Anthropic's servers are overloaded: wait a moment, then try again";
    expect(events(sim, 'turn_started', id)[0].input).toBe(ask);
    expect(events(sim, 'error', id)).toEqual([{ type: 'error', message: reason, fatal: false }]);
    expect(events(sim, 'turn_completed', id)[0]).toMatchObject({ status: 'failed', result_text: reason });
    await until(() => sim.agents.get(id)!.status === 'idle');

    // "Send again": the same message, and this time the turn does the work.
    story.userChat(`agent:${id}`, ask);
    await until(() => events(sim, 'turn_completed', id).length === 2);
    expect(events(sim, 'turn_completed', id)[1].status).toBe('completed');
    expect(events(sim, 'files_changed', id).map((e) => e.paths)).toEqual([['README.md']]);
  });

  it('fails nothing without the parameter', async () => {
    const { sim, story } = demo({ story: { yourRequestFirst: true } });
    const id = story.hire('codex').id;
    story.userChat(`agent:${id}`, 'Are you free?');
    await until(() => events(sim, 'turn_completed', id).length === 1);
    expect(events(sim, 'turn_completed', id)[0].status).toBe('completed');
  });
});

describe('a plan’s worker', () => {
  it('is never a 1:1 agent you hired with its name: a new worker starts, with a second name', async () => {
    const { sim, story } = demo({ autoplay: true, story: { yourRequestFirst: true } });
    // A Codex agent you hire for a chat gets Codex's stock name: the one the lockout plan's t1 asks for.
    const fay = story.hire('codex');
    expect(fay.name).toBe('Fay');
    story.userChat('lead', REQUEST_B);
    await until(() => !!planOf(sim, 'lead') && !!task(sim, planOf(sim, 'lead')!.id, 't1')?.assignee);
    const worker = sim.agents.get(task(sim, planOf(sim, 'lead')!.id, 't1')!.assignee!)!;
    expect(worker.id).not.toBe(fay.id);
    expect([worker.name, worker.role, worker.provider]).toEqual(['Fay 2', 'worker', 'codex']);
    expect(sim.agents.get(fay.id)).toMatchObject({ role: 'solo', task_id: null });
  });
});

describe('“Stop” on a helper whose call waits for it', () => {
  it('ends the helper, its call returns an error, and the agent’s turn goes on', async () => {
    const { sim } = demo({ autoplay: true, story: { slowHelper: true } });
    await until(() => events(sim, 'sub_agent').length === 1);
    const ada = named(sim, 'Ada')!.id;
    const row = events(sim, 'sub_agent', ada)[0];
    // The call of its own the helper is in now.
    const own = events(sim, 'sub_agent_progress', ada).filter((p) => p.tool_call_id).pop()!.tool_call_id!;
    const before = sim.log.length;
    expect(new MockControls(sim).stopTask(ada, row.call_id)).toBeNull();

    const decisions = sim.log.slice(before).flatMap((e) => (e.kind.type === 'decision' ? [e.kind.summary] : []));
    expect(decisions).toEqual(['asked Ada to stop its helper: Explore: find every place that reads or switches the theme']);
    expect(events(sim, 'sub_agent', ada).map((s) => s.status)).toEqual(['running', 'stopped']);
    const ended = (call: string) => events(sim, 'tool_completed', ada).find((c) => c.call_id === call);
    expect(ended(own)).toMatchObject({ ok: false });
    expect(ended(row.call_id)).toMatchObject({ ok: false, output: '[Request interrupted by user for tool use]' });
    // As recorded: the turn that waited for the helper says so and goes on, about a second after
    // the stop (a slow helper's step is 19 s long: the turn does not wait for its end); no turn of its own starts.
    await vi.advanceTimersByTimeAsync(1500);
    expect(events(sim, 'message', ada).some((m) => m.text.startsWith('The helper was stopped'))).toBe(true);
    expect(events(sim, 'turn_completed', ada)).toEqual([]);
    await until(() => sim.taskByKey('t1')?.status === 'done');
    expect(events(sim, 'turn_started', ada)).toHaveLength(1);
    expect(events(sim, 'turn_completed', ada).map((t) => t.status)).toEqual(['completed']);
    // A second Stop finds it over: refused in the core's words, and nothing changes.
    const said = /pub const NOT_RUNNING: &str = "([^"]+)";/.exec(claudeRs)?.[1];
    expect(NOT_RUNNING).toBe(said);
    expect(() => new MockControls(sim).stopTask(ada, row.call_id)).toThrow(NOT_RUNNING);
    expect(() => new MockControls(sim).stopTask(ada, 'tid-nobody')).toThrow(NOT_RUNNING);
    expect(events(sim, 'sub_agent', ada).map((s) => s.status)).toEqual(['running', 'stopped']);
  });
});

describe('`?merge=checks`: a check of the finished plan fails on its merge', () => {
  it('refuses the merge in the core’s words, and an agent’s fix makes it merge', async () => {
    const { sim, story } = demo({ autoplay: true });
    await until(() => planOf(sim, 'lead')?.status === 'done');
    const plan = planOf(sim, 'lead')!;
    const source = { kind: 'plan' as const, id: plan.id };
    const merge = new MergeConflictDemo('checks', sim, story);
    // Before a try nothing is known to fail; main has moved by one commit.
    expect(merge.state(source)).toMatchObject({ conflicts: [], failed_checks: [], resolving: null, behind: 1 });
    // "Merge anyway" runs no check.
    expect(() => merge.refuse(plan.id, true)).not.toThrow();
    expect(() => merge.refuse(plan.id)).toThrow('with the plan merged into main, `npx playwright test settings-theme` failed, so nothing was merged and main is unchanged');
    expect(sim.chats.filter((c) => c.thread === 'lead' && c.author === 'codly').pop()!.text).toBe(
      'Not merged: with "Dark mode toggle in Settings" merged into main, `npx playwright test settings-theme` failed. main was not changed. main has commits the plan was never checked with; have an agent fix it (on the plan\'s card), or merge anyway if you know why it fails.',
    );
    expect(merge.state(source).failed_checks).toEqual(['npx playwright test settings-theme']);

    // "Ask an agent to fix it": one more task of the plan, in its own worktree.
    const during = merge.resolve(source);
    const fix = sim.tasks.get(during.resolving!)!;
    expect(fix).toMatchObject({ key: 'fix-checks', title: 'Fix the checks with main merged in', branch: plan.integration_branch, worktree: plan.integration_worktree });
    expect(fix.acceptance[0]).toBe('`npx playwright test settings-theme` passes');
    expect(fix.write_scope).toContain('src/app/settings/page.tsx');
    expect(sim.plans.get(plan.id)!.status).toBe('running');
    expect(events(sim, 'decision').filter((d) => d.decision_type === 'merge').pop()!.summary).toBe(
      `you had an agent fix \`npx playwright test settings-theme\` on ${plan.integration_branch} with main merged in: task fix-checks`,
    );
    await until(() => sim.plans.get(plan.id)!.status === 'done');
    expect(sim.tasks.get(fix.id)!.status).toBe('done');
    expect(merge.state(source)).toMatchObject({ failed_checks: [], resolving: null, behind: 0 });
    expect(() => merge.refuse(plan.id)).not.toThrow();
  });

  it('a cancelled plan’s refusal says to fix the checks yourself', async () => {
    const { sim, story } = demo({ autoplay: true });
    await until(() => sim.taskByKey('t1')?.status === 'done' && planOf(sim, 'lead')?.status === 'running');
    const plan = planOf(sim, 'lead')!;
    story.cancelPlan(plan.id);
    const merge = new MergeConflictDemo('checks', sim, story);
    expect(() => merge.refuse(plan.id)).toThrow('failed, so nothing was merged');
    expect(() => merge.resolve({ kind: 'plan', id: plan.id })).toThrow('this plan was cancelled, so no agent works on it any more; fix the checks yourself');
  });
});

describe('a cancelled plan with finished work', () => {
  it('is merged as the core merges it: refused for its check, fixed by you, and merged anyway when you choose so', async () => {
    const { sim, story } = demo({ autoplay: true });
    await until(() => sim.taskByKey('t1')?.status === 'done' && planOf(sim, 'lead')?.status === 'running');
    const plan = planOf(sim, 'lead')!;
    // While it runs it is not merged, in the core's words (orchestrator/workspace.rs `merge_plan`).
    expect(workspaceRs).toContain('bail!("the plan is still running; wait until it is done (or cancel it)")');
    expect(() => story.mergePlan(plan.id)).toThrow('the plan is still running; wait until it is done (or cancel it)');
    story.cancelPlan(plan.id);
    const merge = new MergeConflictDemo('checks', sim, story);
    expect(() => merge.refuse(plan.id)).toThrow('failed, so nothing was merged');
    // "Have an agent fix it" is the button that says to fix the checks yourself (merge_conflict.rs `cancelled_refusal`).
    expect(mergeConflictRs).toContain('format!("this plan was cancelled, so no agent works on it any more; {left} yourself")');
    expect(() => merge.resolve({ kind: 'plan', id: plan.id })).toThrow('this plan was cancelled, so no agent works on it any more; fix the checks yourself');
    // "Merge anyway" runs no check and merges what the plan finished, as the refusal says it can.
    expect(() => merge.refuse(plan.id, true)).not.toThrow();
    expect(story.mergePlan(plan.id)).toMatch(/^[0-9a-f]{40}$/);
    expect(sim.plans.get(plan.id)).toMatchObject({ status: 'cancelled', merged_sha: expect.any(String) });
  });

  it('a plan that never ran has nothing to merge', async () => {
    const { sim, story } = demo();
    await until(() => planOf(sim, 'lead')?.status === 'awaiting_approval');
    expect(() => story.mergePlan(planOf(sim, 'lead')!.id)).toThrow('the plan never ran, so there is nothing to merge');
  });
});

describe('the fix of a check that fails on the merge', () => {
  it('knows the line it changes in each check’s test file', () => {
    for (const [check, f] of Object.entries(MERGE_FAILURES)) {
      const file = TASK_FILES[f.files].find((x) => x.path === f.path);
      expect(file?.after, check).toContain(f.from);
    }
  });

  it('prints the failing test as a sentence, and its diff is the line it changed', async () => {
    const { sim, story } = demo({ autoplay: true });
    await until(() => planOf(sim, 'lead')?.status === 'done');
    const plan = planOf(sim, 'lead')!;
    const merge = new MergeConflictDemo('checks', sim, story);
    expect(() => merge.refuse(plan.id)).toThrow('`npx playwright test settings-theme` failed');
    const fix = sim.tasks.get(merge.resolve({ kind: 'plan', id: plan.id }).resolving!)!;
    await until(() => sim.tasks.get(fix.id)!.status === 'done');
    const fixer = sim.tasks.get(fix.id)!.assignee!;
    const failed = events(sim, 'tool_completed', fixer).filter((c) => !c.ok).pop()!;
    expect(failed.output).toContain('  ✘  tests/e2e/settings-theme.spec.ts: settings · theme › dark mode persists after reload: expected “dark”, got “light”');
    // Its card counts what it changed: one line in one file, not "+0 −0 in 0 files".
    expect(parseDiff(story.taskDiff(fix.id)).map((f) => [f.path, f.added, f.removed])).toEqual([['tests/e2e/settings-theme.spec.ts', 1, 0]]);
    expect(story.taskDiff(fix.id)).toContain("+    await page.waitForResponse((res) => res.url().endsWith('/api/settings'));");
    // The plan's diff has the test file once, as the fix left it.
    const files = parseDiff(story.planDiff(plan.id));
    expect(new Set(files.map((f) => f.path)).size).toBe(files.length);
    const test = files.filter((f) => f.path === 'tests/e2e/settings-theme.spec.ts');
    expect(test).toHaveLength(1);
    expect(story.planDiff(plan.id)).toContain("+    await page.waitForResponse((res) => res.url().endsWith('/api/settings'));");
  });
});

describe('a task that waits on others', () => {
  it('says why in the core’s words, and says it again when a task it waits on is cancelled', async () => {
    expect(schedulerRs).toContain('format!("blocked: depends on {}", stuck.join(", "))');
    expect(schedulerRs).toContain('format!("waiting: depends on {}"');
    const { sim, story } = demo({ autoplay: true });
    await until(() => planOf(sim, 'lead')?.status === 'running');
    const plan = planOf(sim, 'lead')!;
    const t3 = () => task(sim, plan.id, 't3')!;
    expect(t3()).toMatchObject({ status: 'pending', status_reason: 'waiting: depends on t1, t2' });
    story.moveTask(task(sim, plan.id, 't1')!.id, 'cancelled');
    await vi.advanceTimersByTimeAsync(1000);
    expect(t3()).toMatchObject({ status: 'pending', status_reason: 'blocked: depends on t1 (cancelled)' });
    const said = sim.log.filter((e) => e.task_id === t3().id && e.kind.type === 'decision' && e.kind.decision_type === 'schedule');
    expect(said.map((e) => e.kind.type === 'decision' && e.kind.summary)).toEqual(['waiting: depends on t1, t2', 'blocked: depends on t1 (cancelled)']);
  });

  it('cannot be moved to Ready before what it waits on is done, as in the core', async () => {
    expect(plansRs).toContain('bail!("{} still waits for its dependencies", t.key)');
    const { sim, story } = demo({ autoplay: true });
    await until(() => planOf(sim, 'lead')?.status === 'running');
    const plan = planOf(sim, 'lead')!;
    const t3 = () => task(sim, plan.id, 't3')!;
    expect(() => story.moveTask(t3().id, 'ready')).toThrow('t3 still waits for its dependencies');
    expect(t3()).toMatchObject({ status: 'pending', status_reason: 'waiting: depends on t1, t2' });
    // So a cancel still says the true thing.
    story.moveTask(task(sim, plan.id, 't1')!.id, 'cancelled');
    await vi.advanceTimersByTimeAsync(1000);
    expect(t3()).toMatchObject({ status: 'pending', status_reason: 'blocked: depends on t1 (cancelled)' });
  });
});

describe('a plan’s worker you took a task away from', () => {
  it('takes the task again when you retry it, also when a 1:1 agent you hired has its name', async () => {
    const { sim, story } = demo({ autoplay: true, story: { yourRequestFirst: true } });
    const fay = story.hire('codex');
    story.userChat('lead', REQUEST_B);
    const t1 = () => task(sim, planOf(sim, 'lead')!.id, 't1');
    await until(() => !!planOf(sim, 'lead') && !!t1()?.assignee && sim.actor(t1()!.assignee!).inTurn);
    const worker = t1()!.assignee!;
    expect(sim.agents.get(worker)!.name).toBe('Fay 2');
    // Stop it, unassign it, Retry.
    story.moveTask(t1()!.id, 'blocked');
    await until(() => sim.agents.get(worker)!.status === 'idle' && !sim.actor(worker).inTurn);
    story.assignTask(t1()!.id, null);
    story.moveTask(t1()!.id, 'ready');
    await until(() => t1()!.status === 'running');
    expect(t1()!.assignee).toBe(worker);
    expect([...sim.agents.values()].map((a) => a.name)).not.toContain('Fay 3');
    expect(sim.agents.get(fay.id)).toMatchObject({ role: 'solo', task_id: null });
  });
});

describe('a request that autoplay answered', () => {
  it('says autoplay answered it, not you', async () => {
    const { sim } = demo({ autoplay: true });
    const install = () => requests(sim, (a) => isCommand(a, 'npm install -D @testing-library/react'))[0];
    await until(() => install()?.status === 'approved');
    expect(install().decided_by).toBe(AUTOPLAY);
    expect(events(sim, 'approval_resolved').find((e) => e.approval_id === install().id)).toMatchObject({ status: 'approved', by: AUTOPLAY });
    expect(outcomeBy(install().decided_by)).toBe('by autoplay');
  });
});

describe('`?resume=ask`: a worker that waited for your Resume', () => {
  it('starts its run with its session line, in its task’s worktree, before its turn', async () => {
    const { sim, story } = demo({ story: { resumeAsk: true } });
    const ada = named(sim, 'Ada')!;
    expect(events(sim, 'session_started', ada.id)).toEqual([]);
    story.resumeWork();
    await until(() => events(sim, 'turn_started', ada.id).length === 1);
    const session = sim.log.find((e) => e.agent_id === ada.id && e.kind.type === 'session_started')!;
    expect(session.run_id).toBe(sim.agents.get(ada.id)!.run_id);
    expect(session.kind).toMatchObject({ provider: 'claude', cwd: ada.worktree, session_id: ada.session_id });
    expect(sim.log.indexOf(session)).toBeLessThan(sim.log.findIndex((e) => e.agent_id === ada.id && e.kind.type === 'turn_started'));
  });
});

describe('a run that leaves out the repo’s own configuration', () => {
  it('says so once, in the core’s words, for each Claude run that starts while you have not trusted it', async () => {
    const fmt = /"(Started without this folder's \{\} configuration[^"]+)"/.exec(runsRs)?.[1];
    expect(fmt?.replace('{}', 'Claude Code')).toBe(WITHOUT_FOLDER_CONFIG);
    const sim = new Sim({ speed: 1, skipMs: 0, autoplay: false }, DEFAULT_SETTINGS);
    let without = false;
    sim.withoutFolderConfig = () => without;
    const story = new Story(sim, { yourRequestFirst: true });
    story.openWorkspace(REPO);
    const before = story.hire('claude');
    without = true;
    const eli = story.hire('claude');
    const kit = story.hire('codex');
    const notices = (id: string) => sim.log.filter((e) => e.agent_id === id && e.kind.type === 'notice');
    expect(notices(before.id)).toEqual([]);
    expect(notices(kit.id)).toEqual([]);
    expect(notices(eli.id).map((e) => [e.kind.type === 'notice' && e.kind.text, e.run_id])).toEqual([[WITHOUT_FOLDER_CONFIG, eli.run_id]]);
    expect(startedWithoutFolderConfig(sim.log, eli.id, sim.agents.get(eli.id)!.run_id)).toBe(true);
    // The agent that started before keeps its run as it was; its next run says it.
    expect(startedWithoutFolderConfig(sim.log, before.id, before.run_id)).toBe(false);
    story.interrupt(before.id, true);
    sim.patchAgent(before.id, { permission_mode: 'accept_edits' });
    story.userChat(`agent:${before.id}`, 'Add a line about the changelog.');
    await until(() => notices(before.id).length === 1);
    expect(startedWithoutFolderConfig(sim.log, before.id, sim.agents.get(before.id)!.run_id)).toBe(true);
  });
});

describe('the demo’s agents', () => {
  it('get the same ids on every page load of the same office, and ids of their own', () => {
    const office = (id: string) => {
      const sim = new Sim({ speed: 1, skipMs: 0, autoplay: false }, DEFAULT_SETTINGS);
      sim.workspace = { id } as Workspace;
      return ['Ada', 'Ada', 'Bo'].map((name) => sim.newAgent({ name, provider: 'claude', model: null, role: 'worker', color: 0, billing: 'subscription' }).id);
    };
    const first = office('ws-0198-7abc');
    expect(office('ws-0198-7abc')).toEqual(first);
    expect(new Set(first).size).toBe(3);
    expect(office('ws-0199-1234').some((id) => first.includes(id))).toBe(false);
  });
});

describe('the Git tab of a worker on a task', () => {
  it('lists the files it wrote for the task, with their text, until Codly commits them', async () => {
    const { sim, story } = demo({ autoplay: true });
    await until(() => !!named(sim, 'Bo') && events(sim, 'files_changed', named(sim, 'Bo')!.id).length === 2);
    const bo = named(sim, 'Bo')!;
    const files = mockAgentGit(bo, sim.now(), sim.log).files.map((f) => f.path);
    expect(files).toEqual(['prisma/schema.prisma', 'prisma/migrations/20261025093000_add_theme/migration.sql']);
    const t2 = sim.tasks.get(bo.task_id!)!;
    const diff = parseDiff(mockAgentDiff(bo, sim.log, { key: t2.key, files: story.taskFiles(t2) }));
    expect(diff.map((f) => f.path)).toEqual(files);
    expect(diff[0].added).toBeGreaterThan(0);
    // Committed: nothing is left in its worktree.
    await until(() => events(sim, 'committed', bo.id).length === 1);
    expect(mockAgentGit(sim.agents.get(bo.id)!, sim.now(), sim.log).files).toEqual([]);
  });

  it('a worker on a board task of its own shows the manifest it edits, not a note of a 1:1 conversation', async () => {
    const { sim, story } = demo({ story: { yourRequestFirst: true } });
    const t = story.createTask('Sort the imports', '');
    story.moveTask(t.id, 'ready');
    const worker = () => sim.agents.get(sim.tasks.get(t.id)!.assignee ?? '');
    await until(() => !!worker() && events(sim, 'files_changed', worker()!.id).length === 1);
    const now = sim.tasks.get(t.id)!;
    const diff = mockAgentDiff(worker(), sim.log, { key: now.key, files: story.taskFiles(now) });
    expect(parseDiff(diff).map((f) => f.path)).toEqual(['package.json']);
    expect(diff).not.toContain('1:1 conversation');
    const card = parseDiff(story.taskDiff(t.id)).map((f) => [f.path, f.added, f.removed]);
    expect(parseDiff(diff).map((f) => [f.path, f.added, f.removed])).toEqual(card);
  });
});

describe('`?cards=3`: three cards wait in one thread', () => {
  it('three workers of one plan ask within seconds of each other, and the fourth finishes while all three wait', async () => {
    const { sim, story } = demo({ autoplay: true, story: { threeCards: true } });
    const plan = () => planOf(sim, 'lead');
    const t = (key: string) => task(sim, plan()!.id, key)!;
    await until(() => !!plan() && task(sim, plan()!.id, 't4')?.status === 'done');
    expect(sim.chats.filter((c) => c.author === 'user').map((c) => c.text)).toEqual([REQUEST_CARDS]);
    expect([...sim.tasks.values()].filter((x) => x.plan_id === plan()!.id).map((x) => [x.key, x.depends_on])).toEqual([['t1', []], ['t2', []], ['t3', []], ['t4', []]]);
    // The three requests, one of each kind, all before the fourth task is done, and all three wait.
    const asked = sim.log.filter((e) => e.kind.type === 'approval_requested');
    const done = sim.log.findIndex((e) => e.kind.type === 'task_changed' && e.kind.task.id === t('t4').id && e.kind.task.status === 'done');
    expect(asked.map((e) => (e.kind.type === 'approval_requested' ? e.kind.approval.subject.kind : null)).sort()).toEqual(['command', 'file_change', 'question']);
    expect(asked.every((e) => sim.log.indexOf(e) < done)).toBe(true);
    expect(asked[2].ts - asked[0].ts).toBeLessThan(5000);
    const waiting = requests(sim, (a) => a.status === 'pending');
    expect(waiting.map((a) => sim.tasks.get(a.task_id!)!.key).sort()).toEqual(['t1', 't2', 't3']);
    // A worker that waits with a file written has a diff on its unfinished task.
    expect(parseDiff(story.taskDiff(t('t1').id)).map((f) => f.path)).toEqual(['.lintstagedrc.json']);
    expect(parseDiff(story.taskDiff(t('t2').id)).map((f) => f.path)).toEqual(['docs/env.md']);
    // Autoplay ran the plan, and leaves the three cards to you.
    await vi.advanceTimersByTimeAsync(20_000);
    expect(requests(sim, (a) => a.status === 'pending')).toHaveLength(3);
    for (const a of waiting) story.resolveApproval(a.id, 'allow_once', null, a.subject.kind === 'question' ? { 'The licence says 2024. Which years should it name?': '2026' } : null);
    await until(() => plan()!.status === 'done');
    expect(parseDiff(story.taskDiff(t('t2').id)).map((f) => f.path)).toEqual(['docs/env.md', 'README.md']);
    expect(story.taskDiff(t('t3').id)).toContain('+Copyright (c) 2026 Acme Inc.');
  });

  it('changes nothing in the script without the parameter', async () => {
    const { sim } = demo();
    await until(() => sim.chats.some((c) => c.author === 'user'));
    expect(sim.chats.filter((c) => c.author === 'user').map((c) => c.text)).toEqual([REQUEST]);
  });
});
