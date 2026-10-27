import { describe, expect, it } from 'vitest';
import { buildMessageGraph, classifyAct, exactSearch } from '../demo/memgraph';
import { circleLayout, edgeOffsets, edgePath } from '../util/msgLayout';
import type { Agent } from '../gen/Agent';
import type { ChatMessage } from '../gen/ChatMessage';
import type { Memory } from '../gen/Memory';
import type { Task } from '../gen/Task';

const agent = (id: string, name: string, role: Agent['role'], thread: string | null = null) => ({ id, name, role, thread, color: 1, title: null }) as unknown as Agent;
const task = (id: string, o: Partial<Task> = {}) => ({ id, key: id, title: id, goal: '', acceptance: [], summary: null, status_reason: null, assignee: null, depends_on: [], updated_at: 1, ...o }) as unknown as Task;
const mem = (id: string, text: string, o: Partial<Memory> = {}) => ({ id, text, kind: 'fact', scope: 'project', scope_id: null, author: 'user', superseded_by: null, created_at: 1, ...o }) as unknown as Memory;
const chat = (thread: string, author: string, text: string, ts: number, attachment: ChatMessage['attachment'] = null) => ({ id: `c${ts}`, thread, author, text, attachment, ts }) as ChatMessage;

describe('exactSearch', () => {
  const tasks = [task('t1', { title: 'Dark mode toggle', goal: 'Add a theme switch to settings' }), task('t2', { title: 'Billing', goal: 'Invoices' })];
  const mems = [mem('m1', 'Settings API: theme is system | light | dark', { kind: 'note', author: 'bo', created_at: 10 }), mem('m2', 'Await settings writes', { created_at: 20 })];

  it('is literal, case-insensitive and spans tasks, notes and memories', () => {
    const hits = exactSearch('SETTINGS', tasks, mems);
    expect(hits.map((h) => h.kind).sort()).toEqual(['board', 'memory', 'task']);
    const h = hits.find((x) => x.kind === 'task')!;
    expect(h.field).toBe('goal');
    expect(h.snippet.slice(h.at, h.at + h.len).toLowerCase()).toBe('settings');
    expect(exactSearch('mode dark', tasks, mems)).toEqual([]);
    expect(exactSearch('  ', tasks, mems)).toEqual([]);
  });

  it('ranks title matches first and skips superseded memories', () => {
    const hits = exactSearch('cache', [task('a', { goal: 'the cache' }), task('b', { title: 'Cache layer' })], [mem('x', 'cache', { superseded_by: 'y' })]);
    expect(hits.map((h) => h.id)).toEqual(['b', 'a']);
  });

  it('trims long text around the match', () => {
    const h = exactSearch('needle', [], [mem('m', `${'a'.repeat(100)} needle ${'b'.repeat(100)}`)])[0];
    expect(h.snippet.startsWith('…') && h.snippet.endsWith('…')).toBe(true);
    expect(h.snippet.slice(h.at, h.at + h.len)).toBe('needle');
  });
});

describe('classifyAct', () => {
  it('reads the six acts from keywords', () => {
    expect(classifyAct("I can't touch that file", 'inform')).toBe('refuse');
    expect(classifyAct('I propose we split this in two', 'inform')).toBe('propose');
    expect(classifyAct('Question from Ada (t1): which key do we use?', 'inform')).toBe('query');
    expect(classifyAct('Done. Merged into integration.', 'inform')).toBe('agree_done');
    expect(classifyAct('Please add a settings route', 'inform')).toBe('request');
    expect(classifyAct('The build takes two minutes', 'inform')).toBe('inform');
    expect(classifyAct('The build takes two minutes', 'request')).toBe('request');
    expect(classifyAct('The branch was abandoned', 'inform')).toBe('inform');
  });
});

describe('buildMessageGraph', () => {
  const agents = [agent('lead', 'Lead', 'lead', 'lead'), agent('ada', 'Ada', 'worker'), agent('bo', 'Bo', 'worker')];
  const t1 = task('t1', { assignee: 'ada' });
  const t2 = task('t2', { assignee: 'bo', depends_on: ['t1'] });
  const chats = [
    chat('lead', 'user', 'Add a dark mode toggle', 1),
    chat('lead', 'codly', 'Plan ready (2 tasks).', 2),
    chat('lead', 'ada', 'Question from Ada (t1): which theme key do we use?', 3),
    chat('lead', 'lead', 'Use the data-theme attribute.', 4, { reply_to: 'ada' }),
    chat('lead', 'ada', 'Another question: where do tokens live?', 5),
    chat('agent:bo', 'bo', 'Thanks, understood. Done with the API.', 7),
    chat('agent:ghost', 'user', 'hi', 8),
  ];
  const notes = [
    mem('n1', 'Theme tokens key on [data-theme]', { kind: 'note', author: 'ada', scope: 'task', scope_id: 't1', created_at: 9 }),
    mem('n2', 'Heads up: flaky e2e', { kind: 'note', author: 'bo', created_at: 10 }),
    mem('f', 'not a note', { author: 'bo' }),
  ];
  const g = buildMessageGraph(agents, chats, notes, [t1, t2]);
  const edge = (from: string, to: string, act: string) => g.edges.find((e) => e.from === from && e.to === to && e.act === act);

  it('draws requests, questions, replies and notes', () => {
    expect(edge('user', 'lead', 'request')?.count).toBe(1);
    expect(edge('ada', 'lead', 'query')?.count).toBe(2);
    expect(edge('ada', 'lead', 'query')?.sample).toBe('Another question: where do tokens live?');
    expect(edge('lead', 'ada', 'inform')).toBeTruthy();
    expect(edge('bo', 'user', 'agree_done')?.count).toBe(1);
    // A task note goes to the assignees of dependent tasks; a loose note to the whiteboard.
    expect(edge('ada', 'bo', 'inform')).toBeTruthy();
    expect(edge('bo', 'board', 'inform')).toBeTruthy();
  });

  it('leaves out codly lines, unknown agents and non-notes', () => {
    expect(g.edges.every((e) => e.from !== 'codly' && e.from !== 'ghost' && e.to !== 'ghost')).toBe(true);
    expect(g.edges.filter((e) => e.to === 'board')).toHaveLength(1);
    expect(g.nodes.map((n) => n.id)).toEqual(['lead', 'ada', 'bo', 'user', 'board']);
  });

  it('lists agents even with no traffic', () => {
    expect(buildMessageGraph(agents, [], [], [])).toEqual({ nodes: agents.map((a) => expect.objectContaining({ id: a.id })), edges: [] });
  });
});

describe('message graph layout', () => {
  it('puts nodes on an ellipse, first at the top, deterministically', () => {
    const a = circleLayout(['a', 'b', 'c', 'd'], 400, 300, 50);
    const b = circleLayout(['a', 'b', 'c', 'd'], 400, 300, 50);
    expect([...a]).toEqual([...b]);
    expect(a.get('a')!.x).toBeCloseTo(200);
    expect(a.get('a')!.y).toBeCloseTo(50);
    expect(a.get('c')!.y).toBeCloseTo(250);
    expect(circleLayout(['solo'], 400, 300).get('solo')).toEqual({ x: 200, y: 150 });
    expect(circleLayout([], 400, 300).size).toBe(0);
  });

  it('bends parallel and opposite edges apart, and leaves a lone edge straight', () => {
    expect(edgeOffsets([{ from: 'a', to: 'b', act: 'request' }])).toEqual([0]);
    const [ab, ba] = edgeOffsets([{ from: 'a', to: 'b', act: 'request' }, { from: 'b', to: 'a', act: 'inform' }]);
    // Drawn from their own ends, opposite signs mean the same side of the page: they must differ in space.
    const p = { x: 0, y: 0 };
    const q = { x: 100, y: 0 };
    const mid = (o: number, from: typeof p, to: typeof q) => edgePath(from, to, o, 20).mid.y;
    expect(Math.abs(mid(ab, p, q) - mid(ba, q, p))).toBeGreaterThan(5);
  });

  it('stops edges short of the node', () => {
    const { d } = edgePath({ x: 0, y: 0 }, { x: 100, y: 0 }, 0, 20);
    expect(d).toBe('M21.0,0.0 Q50.0,0.0 75.0,0.0');
  });
});
