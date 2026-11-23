// A timed run of one fixed stream: a Lead with a long thread writes an 8 KB reply in 300 batches
// while three workers stream and call tools. It prints what a batch costs (a `[stream-perf]`
// line, medians of three runs) and checks what must hold on any machine: a batch that is not
// about the open thread builds nothing, and a new word parses only the block being written.
// "Before" is what every batch did until this was changed: build the whole thread again, and
// parse the whole message again.
import { Marked } from 'marked';
import { describe, expect, it } from 'vitest';
import { buildSpine, keepRows, openStreams, streamText, type Item } from '../../home/spine';
import { mkAgent } from '../../office/demo/script';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import type { Snapshot } from '../gen/Snapshot';
import { store } from '../state/store.svelte';
import { markdownBlocks } from '../util/markdown';
import { streamRenderer } from '../util/mdstream';
import { clean } from '../util/sanitize';

const TURNS = 150;
const CALLS = 6;
const FILES = 3000;
const BATCHES = 300;
const REPS = 3;
const CARET = '';

const agents = [mkAgent('l1', 'Mira', 'claude', 'lead', 0, 'working', 'lead'), mkAgent('w1', 'Ada', 'claude', 'worker', 1, 'working'), mkAgent('w2', 'Bo', 'codex', 'worker', 2, 'working'), mkAgent('w3', 'Cy', 'gemini', 'worker', 3, 'working')];
const apply = (evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);
const reset = () => {
  const snap = { as_of: 0, clis: [], accounts: [], workspace: null, settings: null, workspace_team: null, request_prefs: {}, agents, plans: [], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events: [] };
  (store as unknown as { applySnapshot(s: Snapshot): void }).applySnapshot(snap as unknown as Snapshot);
};

let id = 0;
let clock = 0;
const ev = (agent: string | null, kind: unknown, live = false): EventRow => ({ id: live ? 0 : ++id, ts: (clock += 10), agent_id: agent, task_id: null, run_id: null, kind: kind as EventKind });
const read = (agent: string, call: string, path: string): EventRow[] => [
  ev(agent, { type: 'tool_started', call_id: call, name: 'Read', activity: 'read', target: { path, paths: [], command: null, query: null, url: null, server: null, tool: null }, input: {}, confidence: 'observed' }),
  ev(agent, { type: 'tool_completed', call_id: call, ok: true, output: null, exit_code: null, duration_ms: null, tests_passed: null, tests_failed: null }),
];
const batches = (evs: EventRow[], n: number): EventRow[][] => {
  const out: EventRow[][] = [];
  for (let i = 0; i < evs.length; i += n) out.push(evs.slice(i, i + n));
  return out;
};

/** The Lead's long thread: 150 turns of your message, six tool calls, a thought and a reply. */
function history(): EventRow[] {
  const out: EventRow[] = [];
  for (let t = 0; t < TURNS; t++) {
    const ask = `Request ${t}: please change the handler of module ${t} and keep the tests green`;
    const reply = `Reply ${t}: the handler of **module ${t}** now validates its input.\n\n- changed \`src/mod${t}/file0.ts\`\n- the tests pass`;
    out.push(ev(null, { type: 'chat', message: { id: `u${t}`, thread: 'lead', author: 'user', text: ask, attachment: null, ts: clock + 10 } }));
    out.push(ev('l1', { type: 'turn_started', input: ask }));
    for (let i = 0; i < CALLS; i++) out.push(...read('l1', `c${t}_${i}`, `src/mod${t}/file${i}.ts`));
    out.push(ev('l1', { type: 'reasoning', item_id: `r${t}`, text: 'The handler needs a guard before it parses.' }));
    out.push(ev('l1', { type: 'message', item_id: `m${t}`, text: reply, phase: 'final' }));
    out.push(ev(null, { type: 'chat', message: { id: `a${t}`, thread: 'lead', author: 'l1', text: reply, attachment: null, ts: clock + 10 } }));
    out.push(ev('l1', { type: 'turn_completed', status: 'completed', result_text: null, duration_ms: null }));
  }
  return out;
}

/** A worker that reads 3000 different files. */
function manyFiles(): EventRow[] {
  const out: EventRow[] = [];
  for (let i = 0; i < FILES; i++) out.push(...read('w1', `f${i}`, `src/pkg${i % 50}/file${i}.ts`));
  return out;
}

/** The reply the Lead streams: about 8 KB of paragraphs, lists, code and a table. */
function longReply(): string {
  const parts: string[] = [];
  for (let s = 0; s < 12; s++) {
    parts.push(`## Step ${s + 1}: the parser\n`);
    parts.push(`The parser reads **one token** at a time and keeps a small stack, so a nested block costs no more than a flat one. See \`parse_block\` in \`src/parser/block${s}.ts\` for the loop.\n`);
    parts.push(`- it reads the next line\n- it closes the blocks that ended\n- it opens the new block\n`);
    parts.push('```ts\n' + `export function step${s}(line: string): Block {\n  const depth = indentOf(line);\n  while (stack.length > depth) stack.pop();\n  return open(line.trim());\n}\n` + '```\n');
    parts.push(`| case | before | after |\n|---|---|---|\n| flat | ${s + 3} ms | 1 ms |\n| nested | ${s + 9} ms | 2 ms |\n`);
    parts.push(`After this step the tests of module ${s} pass, and the next step can build on it without another change to the lexer or to the callers of the parser.\n`);
  }
  return parts.join('\n');
}

const median = (xs: number[]) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];
const ms = (n: number) => Math.round(n * 100) / 100;
const replaced = (was: Item[], now: Item[]) => now.filter((row, i) => row !== was[i]).length;

describe('the cost of a streamed batch', () => {
  it('builds nothing for other agents, and parses only the block being written', { timeout: 120_000 }, () => {
    // The tests run without a browser, so without the sanitizer: both ways are timed with the parser alone.
    const md = new Marked({ gfm: true, breaks: false, async: false });
    const render = (src: string) => md.parse(clean(src)) as string;
    const reply = longReply();
    const size = Math.ceil(reply.length / BATCHES);
    const t = { history: [] as number[], files: [] as number[], apply: [] as number[], spineBefore: [] as number[], spineAfter: [] as number[], mdBefore: [] as number[], mdAfter: [] as number[], toolCall: [] as number[] };
    let rowCount = 0;
    let builds = 0;
    let rowsReplaced = 0;
    let toolCallReplaced = 0;
    let parsedLate = 0;

    for (let rep = 0; rep < REPS; rep++) {
      id = 0;
      clock = 1_000;
      reset();
      let at = performance.now();
      for (const b of batches(history(), 20)) apply(b);
      t.history.push(performance.now() - at);
      at = performance.now();
      for (const b of batches(manyFiles(), 20)) apply(b);
      t.files.push(performance.now() - at);

      // The open thread, as the conversation holds it.
      let rows = keepRows([], buildSpine('lead'));
      let seen = { rev: store.revBy.get('l1'), streams: openStreams('l1') };
      const parsed: number[] = [];
      const write = streamRenderer((src) => (parsed.push(src.length), render(src)), markdownBlocks);
      let parts = write('');
      let text = '';
      let tApply = 0;
      let tSpineBefore = 0;
      let tSpineAfter = 0;
      let tMdBefore = 0;
      let tMdAfter = 0;
      builds = 0;
      rowsReplaced = 0;
      for (let b = 0; b < BATCHES; b++) {
        const chunk = reply.slice(b * size, (b + 1) * size);
        text += chunk;
        const batch = [
          ev('l1', { type: 'text_delta', item_id: 'live', text: chunk }, true),
          ev('w1', { type: 'text_delta', item_id: 'wl1', text: 'word ' }, true),
          ev('w2', { type: 'text_delta', item_id: 'wl2', text: 'word ' }, true),
          ev('w3', { type: 'text_delta', item_id: 'wl3', text: 'word ' }, true),
          ...(b % 3 === 0 ? read('w2', `live${b}`, `src/live/file${b}.ts`) : []),
        ];
        at = performance.now();
        apply(batch);
        tApply += performance.now() - at;

        // Before: every batch built the whole thread again, and every row was a new object.
        at = performance.now();
        rowCount = buildSpine('lead').length;
        tSpineBefore += performance.now() - at;
        // After: the thread is built again only when its agent's stored events, or which
        // messages it is writing, changed; the row being written reads its words.
        at = performance.now();
        const now = { rev: store.revBy.get('l1'), streams: openStreams('l1') };
        if (now.rev !== seen.rev || now.streams !== seen.streams) {
          const next = keepRows(rows, buildSpine('lead'));
          if (b > 0) rowsReplaced += replaced(rows, next);
          rows = next;
          seen = now;
          builds++;
        }
        const row = rows[rows.length - 1];
        const words = row.kind === 'say' ? streamText(row) : '';
        tSpineAfter += performance.now() - at;
        expect(words).toBe(text);

        // Before: the message was parsed whole on every batch.
        at = performance.now();
        render(text + CARET);
        tMdBefore += performance.now() - at;
        // After: the finished blocks are kept, the rest is parsed.
        at = performance.now();
        parts = write(words, CARET);
        tMdAfter += performance.now() - at;
      }
      expect(parts.done.join('') + parts.tail).toBe(render(reply + CARET));
      parsedLate = Math.max(...parsed.slice(-100));

      // The Lead's own tool call does build its thread again, and replaces one row.
      const call = read('l1', 'own', 'src/own.ts');
      apply([call[0]]);
      at = performance.now();
      const next = keepRows(rows, buildSpine('lead'));
      t.toolCall.push(performance.now() - at);
      toolCallReplaced = replaced(rows, next);

      t.apply.push(tApply);
      t.spineBefore.push(tSpineBefore);
      t.spineAfter.push(tSpineAfter);
      t.mdBefore.push(tMdBefore);
      t.mdAfter.push(tMdAfter);
    }

    console.log(
      `[stream-perf] ${JSON.stringify({
        store_history_2700_events_ms: ms(median(t.history)),
        store_3000_files_6000_events_ms: ms(median(t.files)),
        store_300_live_batches_ms: ms(median(t.apply)),
        spine_rows: rowCount,
        spine_300_batches_before_ms: ms(median(t.spineBefore)),
        spine_300_batches_after_ms: ms(median(t.spineAfter)),
        spine_builds_in_300_batches: builds,
        spine_rows_replaced_after_the_first_batch: rowsReplaced,
        spine_own_tool_call_build_ms: ms(median(t.toolCall)),
        spine_own_tool_call_rows_replaced: toolCallReplaced,
        markdown_reply_chars: reply.length,
        markdown_300_batches_before_ms: ms(median(t.mdBefore)),
        markdown_300_batches_after_ms: ms(median(t.mdAfter)),
        markdown_longest_parse_in_last_100_batches_chars: parsedLate,
      })}`,
    );

    expect(rowCount).toBeGreaterThan(400);
    // One build, when the reply started: 299 batches of words and of other agents built nothing.
    expect(builds).toBe(1);
    expect(rowsReplaced).toBe(0);
    expect(toolCallReplaced).toBe(1);
    // Near its end the 8 KB reply is parsed a block at a time.
    expect(parsedLate).toBeLessThan(reply.length / 4);
    expect(median(t.spineAfter)).toBeLessThan(median(t.spineBefore));
    expect(median(t.mdAfter)).toBeLessThan(median(t.mdBefore));
  });
});
