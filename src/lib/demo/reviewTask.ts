// The browser demo's answer to `send_back_plan`. Like the core (orchestrator/review_back.rs):
// your review comments on a finished plan become one more task of the plan, on the plan's own
// branch and in its worktree, for the agent that wrote most of the files they are on. That
// agent works on them for a few seconds (when it is busy, the task waits for it), the task goes
// through a short gate with the plan's checks, and the plan is done again. A task that does not
// finish leaves the plan as it was. Refused in the core's words.
import type { Agent } from '../gen/Agent';
import type { Task } from '../gen/Task';
import { Cancelled, sha, uid, type Sim } from './sim';

/** How a refusal ends when the comments belong in a new request (review_back.rs `NEW_REQUEST`). */
const NEW_REQUEST = 'ask for the changes in a new request';
const TITLE = 'Changes you asked for in review';

/** "review", then "review-2"… (merge_conflict.rs `next_key`). */
function nextKey(tasks: Task[]): string {
  let key = 'review';
  for (let n = 2; tasks.some((t) => t.key === key); n++) key = `review-${n}`;
  return key;
}

/** The files of a task's commits, as Codly recorded them. */
const committedFiles = (sim: Sim, taskId: string): string[] =>
  sim.log.flatMap((e) => (e.task_id === taskId && e.kind.type === 'committed' ? e.kind.files : []));

/** The finished task of the plan whose commits changed most of `files`. */
function wroteMost(sim: Sim, done: Task[], files: string[]): Task | null {
  let best: { n: number; task: Task } | null = null;
  for (const task of done) {
    const changed = new Set(committedFiles(sim, task.id));
    const n = files.filter((f) => changed.has(f)).length;
    if (n > 0 && (!best || n > best.n)) best = { n, task };
  }
  return best?.task ?? null;
}

/** A worker at rest: on no task, in no turn, not stopped. */
const free = (sim: Sim, a: Agent | undefined): a is Agent => !!a && !a.archived && a.role === 'worker' && a.status !== 'offline' && !a.task_id && !sim.actor(a.id).inTurn;

/** `send_back_plan`: returns the new task. */
export function sendBackPlanDemo(sim: Sim, planId: string, comments: string, files: string[]): Task {
  const plan = sim.plans.get(planId);
  if (!plan) throw new Error('Unknown plan');
  const text = comments.trim();
  if (!text) throw new Error('there is no comment to send');
  if (plan.merged_sha) throw new Error(`this plan is merged into your branch; ${NEW_REQUEST}`);
  if (plan.kept) throw new Error(`you kept this plan on its own branch; ${NEW_REQUEST}`);
  if (plan.status === 'running' || plan.status === 'integrating') throw new Error('the plan is still running; send your comments again when it is done');
  if (plan.status !== 'done') throw new Error(`this plan is not finished; ${NEW_REQUEST}`);
  const branch = plan.integration_branch;
  const worktree = plan.integration_worktree;
  if (!branch || !worktree) throw new Error(`the plan has no branch and worktree of its own any more; ${NEW_REQUEST}`);
  const tasks = [...sim.tasks.values()].filter((t) => t.plan_id === planId);
  const done = tasks.filter((t) => t.status === 'done' && t.branch !== branch);
  const wrote = wroteMost(sim, done, files);
  const agent = wrote?.assignee ? sim.agents.get(wrote.assignee) : undefined;
  const who = agent && !agent.archived ? agent : null;
  const now = sim.now();
  const task = sim.putTask({
    id: uid('t-'), plan_id: planId, key: nextKey(tasks), title: TITLE,
    goal: `The plan is finished, and the user reviewed its work. This worktree holds the plan’s finished work, on the plan’s own branch. Make the changes the user asks for below, here, and keep the rest of the plan’s work as it is. Then make sure the verify commands pass.\n\n${text}`,
    acceptance: ['Each review comment is done, or the report says why not', 'The plan’s verify commands pass'],
    write_scope: [...new Set([...done.flatMap((t) => committedFiles(sim, t.id)), ...files])],
    verify: [...new Set(done.flatMap((t) => t.verify))], depends_on: [], status: 'ready',
    assignee: who?.id ?? null, agent_hint: null, account_hint: null, consultant: null, fresh_session: true, branch, worktree, attempt: 0,
    status_reason: who ? `Ready: ${who.name} takes it when free` : 'Ready: a free agent picks it up', summary: null, created_at: now, updated_at: now,
  });
  sim.decision('review', 'human', `you sent the plan back with review comments: task ${task.key}${who ? ` for ${who.name}` : ''}`, { plan_id: planId, files, assignee: task.assignee }, { task: task.id });
  sim.chat(plan.thread, 'user', text);
  sim.chat(plan.thread, 'codly', who
    ? `Your review comments are task ${task.key} of this plan, for ${who.name} (who wrote ${wrote!.key}): it works in the plan’s worktree, in a new conversation. Then the plan’s checks run again and you can merge it.`
    : `Your review comments are task ${task.key} of this plan: the next free agent takes it and works in the plan’s worktree. Then the plan’s checks run again and you can merge it.`);
  sim.patchPlan(planId, { status: 'running' });
  void work(sim, planId, task.id, files);
  return task;
}

/** The task's worker makes the changes; then the plan is done again, the task finished or not. */
async function work(sim: Sim, planId: string, taskId: string, files: string[]) {
  const live = () => sim.tasks.get(taskId)!;
  // Its agent when that one is free (it waits for it), else the first free worker.
  let worker: Agent | null = null;
  while (live().status === 'ready') {
    await sim.sleep(900);
    const wanted = live().assignee;
    const pick = wanted ? sim.agents.get(wanted) : [...sim.agents.values()].find((a) => free(sim, a));
    if (free(sim, pick)) {
      worker = pick;
      break;
    }
  }
  const plan = () => sim.plans.get(planId)!;
  if (worker) {
    const act = sim.actor(worker.id);
    const own = worker.worktree;
    const task = live();
    act.set({ task_id: taskId, worktree: task.worktree });
    sim.patchTask(taskId, { status: 'running', assignee: act.id, attempt: 1, status_reason: null });
    sim.decision('route', 'rule', `${task.key} → ${act.a.name}: it wrote these files, and it works in the plan’s own worktree`, { plan_id: planId, task: task.key, agent: act.a.name }, { agent: act.id, task: taskId });
    const names = files.map((f) => `\`${f}\``).join(', ');
    try {
      await act.sleep(600);
      await act.turn(`Task ${task.key} · ${task.title}\n\n${task.goal}`, async () => {
        await act.think(`The user reviewed the finished plan and left comments on ${files.join(', ')}. Change what they ask there and keep the rest of the plan’s work.`);
        for (const path of files) await act.tool({ name: 'Read', activity: 'read', target: { path }, ms: 600 });
        for (const path of files) await act.tool({ name: 'Edit', activity: 'edit', target: { path }, ms: 1300, files: [path] });
        const check = task.verify[0];
        if (check) await act.tool({ name: 'Bash', activity: 'test', target: { command: check }, ms: 2000, lines: [`$ ${check}`, 'ok'] });
        await act.say(`I changed ${names} as you asked in review. The rest of the plan’s work is as it was${check ? `, and \`${check}\` passes` : ''}.`);
      });
      // A short gate, like the core's: the commit, then the plan's checks. It ends when the task was stopped or cancelled meanwhile.
      if (live().status !== 'running') throw new Cancelled();
      act.status('done', 'done');
      sim.patchTask(taskId, { status: 'verifying', status_reason: `Committing and running ${task.verify.join(' && ') || 'the checks'}` });
      await act.sleep(900);
      if (live().status !== 'verifying') throw new Cancelled();
      sim.emit({ type: 'committed', sha: sha(), files, message: `codly(${task.key}): ${task.title}` }, { agent: act.id, task: taskId });
      for (const command of task.verify) sim.emit({ type: 'verified', command, ok: true, exit_code: 0, tail: 'ok' }, { agent: act.id, task: taskId });
      sim.patchTask(taskId, { status: 'done', status_reason: null, summary: `Made the changes you asked for in review in ${files.join(', ')}.` });
      sim.decision('plan_done', 'rule', `The changes you asked for in review are on ${plan().integration_branch}, and the plan’s checks passed again`, { plan_id: planId });
    } catch (e) {
      if (!(e instanceof Cancelled)) throw e;
      sim.cancelled.delete(act.id);
    } finally {
      // The plan's worktree was the agent's for this task only.
      act.set({ task_id: null, worktree: own, ...(act.a.status === 'offline' ? {} : { status: 'idle' as const, activity: null, activity_label: null }) });
    }
  }
  if (live().status !== 'done') {
    // Like the core: a task that does not finish leaves the plan's branch as its tasks left it.
    if (live().status !== 'cancelled') sim.patchTask(taskId, { status: 'cancelled', status_reason: 'the changes you asked for in review did not finish' });
    sim.chat(plan().thread, 'codly', 'The changes you asked for in review were not finished. Codly took them back: the plan’s branch is as its tasks left it.');
  }
  if (plan().status === 'running') sim.patchPlan(planId, { status: 'done' });
}
