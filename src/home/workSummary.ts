// One-line summary of a folded work group: "Read 4 files · searched twice · ran npm test".
import type { Seg } from '../lib/state/store.svelte';

export function workSummary(segs: Seg[]): string {
  const count = (pred: (s: Seg) => boolean) => segs.filter(pred).length;
  const parts: string[] = [];
  const reads = count((s) => s.activity === 'read' || s.activity === 'fetch');
  const searches = count((s) => s.activity === 'search');
  const edits = segs.filter((s) => s.activity === 'edit' || s.activity === 'create' || s.activity === 'delete');
  const tests = segs.filter((s) => s.activity === 'test');
  const runs = segs.filter((s) => s.activity === 'execute');
  const plans = segs.filter((s) => s.activity === 'plan');
  const helpers = count((s) => s.activity === 'delegate');
  const saves = count((s) => s.activity === 'memory' || s.activity === 'note');
  const asks = count((s) => /ask_lead/.test(s.name) || s.target.tool === 'codly_ask_lead');
  const other = segs.length - reads - searches - edits.length - tests.length - runs.length - plans.length - saves - helpers - asks;
  if (reads) parts.push(`read ${reads === 1 ? segs.find((s) => s.activity === 'read' || s.activity === 'fetch')!.label : `${reads} files`}`);
  if (searches) parts.push(`searched ${searches === 1 ? 'once' : searches === 2 ? 'twice' : `${searches} times`}`);
  if (edits.length) parts.push(`changed ${edits.length === 1 ? edits[0].label : `${new Set(edits.map((e) => e.label)).size} files`}`);
  if (tests.length) {
    const last = tests[tests.length - 1];
    parts.push(last.end == null ? 'running tests' : last.testsFailed ? `${last.testsFailed} test${last.testsFailed > 1 ? 's' : ''} failing` : last.testsPassed != null ? `${last.testsPassed} tests passed` : 'ran tests');
  }
  if (runs.length) parts.push(`ran ${runs.length === 1 ? runs[0].label : `${runs.length} commands`}`);
  if (helpers) parts.push(helpers === 1 ? 'briefed a helper' : `briefed ${helpers} helpers`);
  if (plans.length) parts.push(plans.some((s) => /submit_plan/.test(s.name)) ? 'submitted the plan' : 'updated its plan');
  if (saves) parts.push(saves === 1 ? 'saved a note' : `saved ${saves} notes`);
  if (asks) parts.push(asks === 1 ? 'asked the Lead' : `asked the Lead ${asks} times`);
  if (other > 0) parts.push(`${other} other step${other > 1 ? 's' : ''}`);
  const s = parts.join(' · ');
  return s ? s[0].toUpperCase() + s.slice(1) : '';
}
