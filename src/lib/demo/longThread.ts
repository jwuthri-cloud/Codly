// `?long=N` (docs/DEMO.md): a 1:1 agent with a long past, to measure the conversation on it. Its
// thread holds N turns (2000 when N is not a number above 0), each your message, a tool call or
// two and its answer; now and then a command it asked you before, a turn that failed (and your
// message sent again), or a change of its mode. 12 seconds after the page loads, it streams
// one long answer.
import { tick } from 'svelte';
import type { Approval } from '../gen/Approval';
import type { EventKind } from '../gen/EventKind';
import { Cancelled, uid, type Sim } from './sim';
import type { Story } from './story';

/** How many turns `?long` makes when its value is no count. */
export const LONG_DEFAULT = 2000;
/** The time between two turns of the past, in ms. */
const TURN_MS = 90_000;

const ASKS = [
  'Look at how the settings page loads its data, and tell me what is slow.',
  'Add a test for the empty state of the invoice list.',
  'Rename `fetchUser` to `loadUser` everywhere, and keep the old name as an alias.',
  'Why does the build warn about a missing key in `Sidebar.tsx`?',
  'Write a short note in the README about the new env variable.',
];

/** A long answer in markdown: paragraphs, a list, code and a table, about `chars` characters. */
export function longAnswer(chars = 9000): string {
  const parts: string[] = ['Here is what I found, step by step.'];
  for (let i = 1; parts.join('\n\n').length < chars; i++) {
    parts.push(`## Part ${i}`);
    parts.push(`The loader in \`src/app/settings/page.tsx\` asks for the user, then for the team, then for the plan, one after the other. Each waits for the one before, so the page waits for **three** round trips (part ${i}).`);
    parts.push(['- the user: 120 ms', '- the team: 180 ms', `- the plan: ${200 + i} ms`].join('\n'));
    parts.push(['```ts', `const [user, team, plan] = await Promise.all([loadUser(), loadTeam(), loadPlan(${i})]);`, '```'].join('\n'));
    parts.push(['| Call | Before | After |', '| --- | --- | --- |', `| settings ${i} | 500 ms | 200 ms |`].join('\n'));
  }
  return parts.join('\n\n');
}

/** What one batch of events cost the page, in ms: the store's part (`store`, with the listeners
 *  of each event), all of its script up to the screen's update (`script`), then the layout it
 *  asks for (`layout`). */
export type BatchCost = { events: number; store: number; script: number; layout: number };

/**
 * Each batch the Sim hands the UI is timed, into `globalThis.__codlyLong` (read it in the
 * browser's console). Timers, not frames: a page that is not drawn gets no animation frame.
 */
function timeBatches(sim: Sim) {
  const costs: BatchCost[] = [];
  (globalThis as { __codlyLong?: { batches: BatchCost[] } }).__codlyLong = { batches: costs };
  let sink = sim.sink;
  Object.defineProperty(sim, 'sink', {
    get: () => sink,
    set: (to: Sim['sink']) => {
      sink = to && ((b) => {
        const t0 = performance.now();
        to(b);
        const stored = performance.now() - t0;
        void tick().then(() => {
          const t1 = performance.now();
          void document.body.offsetHeight;
          costs.push({ events: b.events.length, store: stored, script: t1 - t0, layout: performance.now() - t1 });
        });
      });
    },
  });
}

/**
 * Hire the agent and write its past into the Sim's log, `turns` turns of it ending a minute ago.
 * Then it streams `longAnswer` into its thread.
 */
export function seedLongThread(story: Story, sim: Sim, turns: number) {
  const agent = story.hire('claude', null, 'Docs', 'Lena');
  const thread = `agent:${agent.id}`;
  const meta = { agent: agent.id, task: null, run: agent.run_id };
  let ts = sim.now() - turns * TURN_MS - 60_000;
  /** The row just written happened at `at` (the Sim stamps it with its clock's now). */
  const stamp = (at: number) => {
    sim.log[sim.log.length - 1].ts = at;
  };
  const emit = (kind: EventKind, at: number) => {
    sim.emit(kind, meta);
    stamp(at);
  };
  const chat = (author: string, text: string, at: number, itemId?: string) => {
    sim.chat(thread, author, text, null, itemId).ts = at;
    stamp(at);
  };
  let failed: string | null = null;
  for (let i = 0; i < turns; i++, ts += TURN_MS) {
    // After a turn that failed, you send the same message again.
    const text: string = failed ?? `${ASKS[i % ASKS.length]} (${i + 1})`;
    failed = null;
    chat('user', text, ts);
    emit({ type: 'turn_started', input: text }, ts + 200);
    const call = uid('call-');
    emit({ type: 'tool_started', call_id: call, name: 'Read', activity: 'read', target: { path: `src/app/page${i % 40}.tsx`, paths: [], command: null, query: null, url: null, server: null, tool: null }, input: { file_path: `src/app/page${i % 40}.tsx` }, confidence: 'observed' }, ts + 2000);
    emit({ type: 'tool_completed', call_id: call, ok: true, output: 'export default function Page() {}', exit_code: null, duration_ms: 400, tests_passed: null, tests_failed: null }, ts + 2400);
    if (i % 25 === 12) {
      const run = uid('call-');
      const ap: Approval = {
        id: uid('ap-'), agent_id: agent.id, run_id: agent.run_id ?? '', task_id: null, call_id: run, subject: { kind: 'command', command: 'npm test', cwd: null },
        reason: null, choices: ['allow_once', 'deny'], risks: [], status: 'pending', decided_by: null, created_at: ts + 3000,
      };
      emit({ type: 'tool_started', call_id: run, name: 'Bash', activity: 'execute', target: { path: null, paths: [], command: 'npm test', query: null, url: null, server: null, tool: null }, input: { command: 'npm test' }, confidence: 'observed' }, ts + 3000);
      emit({ type: 'approval_requested', approval: ap }, ts + 3000);
      sim.approvals.set(ap.id, { ...ap, status: 'approved', decided_by: 'user' });
      emit({ type: 'approval_resolved', approval_id: ap.id, status: 'approved', by: 'user' }, ts + 9000);
      emit({ type: 'tool_completed', call_id: run, ok: true, output: 'Tests: 42 passed', exit_code: 0, duration_ms: 3000, tests_passed: 42, tests_failed: 0 }, ts + 12_000);
    }
    if (i % 50 === 30) {
      emit({ type: 'error', message: 'API Error: 529 Overloaded', fatal: false }, ts + 20_000);
      emit({ type: 'turn_completed', status: 'failed', result_text: 'API Error: 529 Overloaded', duration_ms: 20_000 }, ts + 20_000);
      failed = text;
      continue;
    }
    if (i % 40 === 20) {
      sim.decision('agent_control', 'human', `Lena now works in the mode "${i % 80 === 20 ? 'Accept edits' : 'Ask for edits'}"`, { agent_id: agent.id }, meta);
      stamp(ts + 25_000);
    }
    const item = uid('msg-');
    const answer = `Done with step ${i + 1}: I read \`src/app/page${i % 40}.tsx\`, changed what was asked, and the tests pass.`;
    emit({ type: 'message', item_id: item, text: answer, phase: 'final' }, ts + 30_000);
    chat(agent.id, answer, ts + 30_000, item);
    emit({ type: 'turn_completed', status: 'completed', result_text: null, duration_ms: 30_000 }, ts + 30_100);
  }
  timeBatches(sim);
  const act = sim.actor(agent.id);
  const ask = 'Explain in detail why the settings page is slow, with the numbers.';
  void sim.sleep(12_000).then(() => {
    sim.chat(thread, 'user', ask);
    // Stop or Interrupt ends the answer: that is no error of the demo.
    return act.turn(ask, () => act.say(longAnswer(), { thread, pace: 30 })).catch((e) => {
      if (!(e instanceof Cancelled)) throw e;
    });
  });
}
