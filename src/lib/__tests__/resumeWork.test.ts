import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import resumeRs from '../../../crates/codly-core/src/orchestrator/resume.rs?raw';
import { api } from '../api';
import type { ResumeOffer } from '../gen/ResumeOffer';
import { REREAD_MS, resumeWork } from '../state/resumeWork.svelte';
import { toasts } from '../state/toasts.svelte';
import { BOTH_LINES, asksNow, resumeQuestion, showsBothLines } from '../util/resumeWork';

describe('the launch question, in words', () => {
  it('says how many tasks were running', () => {
    expect(resumeQuestion({ tasks: 1 })).toBe('1 task was running when Codly closed. Resume it?');
    expect(resumeQuestion({ tasks: 3 })).toBe('3 tasks were running when Codly closed. Resume them?');
  });

  it('says so when work only waited to go on', () => {
    expect(resumeQuestion({ tasks: 0 })).toBe('Work was waiting to go on when Codly closed. Resume it?');
  });

  it('is on screen with Ask, until you say Not now', () => {
    expect(asksNow('ask', false)).toBe(true);
    expect(asksNow('ask', true)).toBe(false);
    // Never: no question, only the Resume work button. (Always holds nothing.)
    expect(asksNow('never', false)).toBe(false);
  });
});

describe('the line under the bar’s text', () => {
  it('shows where the bar has room for both lines: 1440 px with a sheet open', () => {
    // 1440 less the labelled rail (184) and the sheet (404), each with its two gutters (14).
    expect(showsBothLines(1440 - (184 + 28) - (404 + 28))).toBe(true);
    expect(showsBothLines(BOTH_LINES)).toBe(true);
  });

  it('goes in less room, where the question takes two lines: 1024 px with a sheet open', () => {
    expect(showsBothLines(1024 - (184 + 28) - (404 + 28))).toBe(false);
    expect(showsBothLines(BOTH_LINES - 1)).toBe(false);
  });
});

const bar = import.meta.glob<string>('/src/components/ResumeWork.svelte', { query: '?raw', import: 'default', eager: true });

/** The text of each button in `source` whose tag has `handler`, whatever attributes or spaces
 *  stand around it. A tag ends at the first ">" outside braces ("=>" is inside them). */
function buttonTexts(source: string, handler: RegExp): string[] {
  const texts: string[] = [];
  for (const start of source.matchAll(/<button\b/g)) {
    let end = start.index! + start[0].length;
    for (let depth = 0; end < source.length; end++) {
      const c = source[end];
      if (c === '{') depth++;
      else if (c === '}') depth--;
      else if (c === '>' && depth === 0) break;
    }
    if (!handler.test(source.slice(start.index!, end))) continue;
    const text = source.slice(end + 1, source.indexOf('</button>', end));
    texts.push(text.replace(/<[^>]*>/g, '').replace(/\s+/g, ' ').trim());
  }
  return texts;
}

describe('the button a waiting task names', () => {
  it('is on the bar under that label, with the question and without it', () => {
    // Each held task shows the core's reason, and the reason names the button by its label.
    const reason = /pub const HELD: &str = "([^"]+)";/.exec(resumeRs)?.[1];
    const named = /press '([^']+)'/.exec(reason ?? '')?.[1];
    expect(named).toBe('Resume work');
    const [barSvelte] = Object.values(bar);
    const labels = buttonTexts(barSvelte, /onclick=\{\s*\(\)\s*=>\s*resumeWork\.resume\(\)\s*\}/);
    // One button with the question (Ask), one without it (after Not now, and with Never).
    expect(labels).toEqual([named, named]);
    expect(buttonTexts(barSvelte, /onclick=\{\s*\(\)\s*=>\s*resumeWork\.notNow\(\)\s*\}/)).toEqual(['Not now']);
  });

  it('reads a button’s text whatever stands around it', () => {
    const source = '<button\n  class="btn"\n  onclick={() => resumeWork.resume()}\n  disabled={a > b}\n>\n  Resume work\n</button>';
    expect(buttonTexts(source, /resumeWork\.resume\(\)/)).toEqual(['Resume work']);
  });
});

describe('the launch question, on screen', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    resumeWork.offer = null;
    resumeWork.later = false;
    for (const t of toasts.list) toasts.dismiss(t.id);
  });

  it('shows what the core holds, and is gone once you resume', async () => {
    const core = vi.spyOn(api, 'resumeWork').mockResolvedValueOnce({ tasks: 2 }).mockResolvedValueOnce(null);
    await resumeWork.check();
    expect(resumeWork.offer).toEqual({ tasks: 2 });
    expect(core).toHaveBeenLastCalledWith();

    await resumeWork.resume();
    expect(core).toHaveBeenLastCalledWith(true);
    expect(resumeWork.offer).toBeNull();
    expect(resumeWork.busy).toBe(false);
  });

  it('keeps the work on hold after Not now', async () => {
    vi.spyOn(api, 'resumeWork').mockResolvedValue({ tasks: 1 });
    await resumeWork.check();
    resumeWork.notNow();
    // Nothing was sent to the core: the Resume work button stays.
    expect(api.resumeWork).toHaveBeenCalledTimes(1);
    expect(resumeWork.offer).toEqual({ tasks: 1 });
    expect(resumeWork.later).toBe(true);
  });

  it('keeps the button when the resume fails, and says why', async () => {
    vi.spyOn(api, 'resumeWork').mockResolvedValueOnce({ tasks: 1 }).mockRejectedValueOnce(new Error('the store is locked'));
    await resumeWork.check();
    await resumeWork.resume();
    expect(resumeWork.offer).toEqual({ tasks: 1 });
    expect(resumeWork.busy).toBe(false);
    expect(toasts.list.map((t) => [t.title, t.body])).toEqual([['Couldn’t resume the work', 'the store is locked']]);
  });

  it('offers the read again when it fails', async () => {
    const core = vi.spyOn(api, 'resumeWork').mockRejectedValueOnce(new Error('no answer')).mockResolvedValueOnce({ tasks: 4 });
    await resumeWork.check();
    expect(resumeWork.offer).toBeNull();
    const [failed] = toasts.list;
    expect(failed.title).toBe('Couldn’t check for work to resume');
    failed.action!.run();
    await vi.waitFor(() => expect(resumeWork.offer).toEqual({ tasks: 4 }));
    expect(core).toHaveBeenCalledTimes(2);
  });
});

describe('the launch question, while tasks and plans change', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(async () => {
    await vi.runAllTimersAsync(); // no read of one test waits into the next
    vi.useRealTimers();
    vi.restoreAllMocks();
    resumeWork.offer = null;
    resumeWork.later = false;
    for (const t of toasts.list) toasts.dismiss(t.id);
  });

  it('reads the count again, once for all the changes of a second', async () => {
    const core = vi.spyOn(api, 'resumeWork').mockResolvedValueOnce({ tasks: 3 }).mockResolvedValueOnce({ tasks: 1 });
    await resumeWork.check();
    resumeWork.changed();
    resumeWork.changed();
    await vi.advanceTimersByTimeAsync(REREAD_MS - 1);
    resumeWork.changed();
    expect(core).toHaveBeenCalledTimes(1); // nothing asked yet
    expect(resumeWork.offer).toEqual({ tasks: 3 });

    await vi.advanceTimersByTimeAsync(1);
    expect(core).toHaveBeenCalledTimes(2);
    expect(core).toHaveBeenLastCalledWith(); // a read: nothing is resumed
    expect(resumeWork.offer).toEqual({ tasks: 1 });

    // No change since: nothing more is asked.
    await vi.advanceTimersByTimeAsync(10 * REREAD_MS);
    expect(core).toHaveBeenCalledTimes(2);
  });

  it('takes the bar away when the core says that nothing waits any more', async () => {
    const core = vi.spyOn(api, 'resumeWork').mockResolvedValueOnce({ tasks: 2 }).mockResolvedValueOnce(null);
    await resumeWork.check();
    resumeWork.changed(); // you cancelled the plan of the two tasks
    await vi.advanceTimersByTimeAsync(REREAD_MS);
    expect(resumeWork.offer).toBeNull();

    // No question on screen: later changes ask nothing.
    resumeWork.changed();
    await vi.advanceTimersByTimeAsync(REREAD_MS);
    expect(core).toHaveBeenCalledTimes(2);
  });

  it('keeps the bar when the read fails, and asks again at the next change', async () => {
    const said = vi.spyOn(console, 'error').mockImplementation(() => {});
    const core = vi.spyOn(api, 'resumeWork').mockResolvedValueOnce({ tasks: 2 }).mockRejectedValueOnce(new Error('no answer')).mockResolvedValueOnce({ tasks: 1 });
    await resumeWork.check();
    resumeWork.changed();
    await vi.advanceTimersByTimeAsync(REREAD_MS);
    expect(resumeWork.offer).toEqual({ tasks: 2 });
    expect(said).toHaveBeenCalledTimes(1);
    expect(toasts.list).toEqual([]); // the bar and its button are still there: no toast for every change

    resumeWork.changed();
    await vi.advanceTimersByTimeAsync(REREAD_MS);
    expect(core).toHaveBeenCalledTimes(3);
    expect(resumeWork.offer).toEqual({ tasks: 1 });
  });

  it('does not put the bar back when an older read is answered after your Resume', async () => {
    let answer!: (offer: ResumeOffer | null) => void;
    const core = vi
      .spyOn(api, 'resumeWork')
      .mockResolvedValueOnce({ tasks: 2 })
      .mockImplementationOnce(() => new Promise<ResumeOffer | null>((r) => (answer = r)))
      .mockResolvedValueOnce(null);
    await resumeWork.check();
    resumeWork.changed();
    await vi.advanceTimersByTimeAsync(REREAD_MS); // the read is on its way
    expect(core).toHaveBeenCalledTimes(2);

    await resumeWork.resume();
    expect(core).toHaveBeenLastCalledWith(true);
    expect(resumeWork.offer).toBeNull();

    answer({ tasks: 2 }); // what waited before the Resume
    for (let i = 0; i < 5; i++) await Promise.resolve(); // the read takes its answer
    expect(resumeWork.offer).toBeNull();
  });

  it('asks nothing beside a Resume that is on its way', async () => {
    let answer!: (offer: ResumeOffer | null) => void;
    const core = vi
      .spyOn(api, 'resumeWork')
      .mockResolvedValueOnce({ tasks: 2 })
      .mockImplementationOnce(() => new Promise<ResumeOffer | null>((r) => (answer = r)));
    await resumeWork.check();
    resumeWork.changed();
    const going = resumeWork.resume();
    await vi.advanceTimersByTimeAsync(REREAD_MS);
    expect(core).toHaveBeenCalledTimes(2); // the first read, and the Resume

    answer(null);
    await going;
    expect(resumeWork.offer).toBeNull();
    expect(resumeWork.busy).toBe(false);
  });
});
