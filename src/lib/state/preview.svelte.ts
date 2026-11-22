// Try the work before you merge it, from the buttons' side. A command runs in a plan's or an
// agent's worktree only when you press Run on the text you see; the core keeps that text for
// the repo, and ends the command on Stop, at a merge and when Codly quits. The output comes
// by asking again while the command runs: each answer brings the lines after the ones shown.
import { api } from '../api';
import type { Preview } from '../gen/Preview';
import type { PreviewAction } from '../gen/PreviewAction';
import type { PreviewSource } from '../gen/PreviewSource';
import { toast } from './toasts.svelte';

/** Lines kept on the page for one run (the core keeps as many). */
export const PREVIEW_LINES = 2000;

/** What the page shows for one worktree: the core's last answer, and the run's output so far. */
export type PreviewShown = { view: Preview; lines: string[] };

class PreviewUi {
  /** By `previewKey` (missing: not read yet, or it could not be read). */
  shown = $state<Record<string, PreviewShown>>({});
}
export const previewUi = new PreviewUi();

export const previewKey = (source: PreviewSource) => `${source.kind}:${source.id}`;

const msg = (e: unknown) => (e instanceof Error ? e.message : typeof e === 'string' ? e : 'Unknown error');

/** Put an answer on the page: the lines of the same run are added to the ones shown. */
function show(key: string, view: Preview) {
  const before = previewUi.shown[key];
  const run = view.run;
  let lines: string[] = [];
  if (run) {
    // What the page has of this run (nothing of another run's).
    const had = before?.view.run?.id === run.id ? { run: before.view.run, lines: before.lines } : null;
    // An answer older than the one shown (two requests crossed): the page keeps what it has.
    if (had && (run.printed < had.run.printed || (had.run.ended !== null && run.ended === null))) return;
    // The answer's lines are the run's lines `first`..`run.printed`: take only what is new,
    // and after a gap (the core dropped old lines) start again.
    const first = run.printed - run.lines.length;
    lines = had && first <= had.run.printed ? [...had.lines, ...run.lines.slice(had.run.printed - first)] : run.lines;
    if (lines.length > PREVIEW_LINES) lines = lines.slice(-PREVIEW_LINES);
  }
  previewUi.shown[key] = { view, lines };
}

async function ask(source: PreviewSource, action: PreviewAction | null): Promise<void> {
  const key = previewKey(source);
  const run = previewUi.shown[key]?.view.run;
  show(key, await api.preview(source, action, run ? { run: run.id, lines: run.printed } : null));
}

/** Read the preview of a worktree: the repo's command, and the run there with its new output. */
export async function loadPreview(source: PreviewSource) {
  try {
    await ask(source, null);
  } catch {
    // Not read: the page keeps what it shows. Run says what is wrong.
  }
}

/** Run `command` in the worktree. It is your text: Codly adds nothing to it. */
export async function startPreview(source: PreviewSource, command: string): Promise<boolean> {
  try {
    await ask(source, { do: 'start', command });
    return true;
  } catch (e) {
    toast({ kind: 'error', title: 'Couldn’t run the command', body: msg(e) });
    return false;
  }
}

/** End the command that runs in the worktree, with everything it started. */
export async function stopPreview(source: PreviewSource): Promise<boolean> {
  try {
    await ask(source, { do: 'stop' });
    return true;
  } catch (e) {
    toast({ kind: 'error', title: 'Couldn’t stop the command', body: msg(e) });
    return false;
  }
}
