// The toast stack lies over the page and must cover nothing you can press. The boxes below
// are the ones measured in the app (left, top, right, bottom in px): a one-line toast is
// 360 x 63, the stack starts 68 px under the window's top.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import tokens from '../../../styles/tokens.css?raw';
import toastsView from '../../../components/Toasts.svelte?raw';
import { countedBox, counts, frontToast, grown, ms, onTop, paced, reachDown, restLook, stackPlace, toastTop, visiblePart, type Box, type Seen, type Shape } from '../toastPlace';

const box = (left: number, top: number, right: number, bottom: number): Box => ({ left, top, right, bottom });

const TOP = 68;
const MOVES = 3;
/** 40 percent of a 900 px and of a 600 px high window. */
const REACH_900 = 360;
const REACH_600 = 240;

describe('toastTop', () => {
  it('stays right under the top bar when nothing is under it', () => {
    expect(toastTop(box(1066, 68, 1426, 131), [], TOP, MOVES, REACH_900)).toBe(68);
  });

  it('does not count a control beside or below the stack', () => {
    const plus = box(1033, 72, 1063, 102);
    const runPlan = box(945, 138, 1077, 170);
    expect(toastTop(box(1066, 68, 1426, 131), [plus, runPlan], TOP, MOVES, REACH_900)).toBe(68);
  });

  it('does not count a control that only touches the stack', () => {
    const above = box(1100, 40, 1200, 68);
    const left = box(1000, 72, 1066, 102);
    expect(toastTop(box(1066, 68, 1426, 131), [above, left], TOP, MOVES, REACH_900)).toBe(68);
  });

  it('goes under one row of controls: the request tabs’ "+" and "Closed requests" (Home, 1440 x 900)', () => {
    const plus = box(1033, 72, 1063, 102);
    const closed = box(1071, 72, 1101, 102);
    expect(toastTop(box(1066, 68, 1426, 131), [plus, closed], TOP, MOVES, REACH_900)).toBe(102);
  });

  it('goes under two rows: the request tabs, then "Cancel plan" (Home, 900 x 600)', () => {
    const tabClose = box(617, 77, 637, 97);
    const plus = box(763, 72, 793, 102);
    const closed = box(801, 72, 831, 102);
    const cancelPlan = box(707, 123, 795, 149);
    expect(toastTop(box(526, 68, 886, 131), [tabClose, plus, closed, cancelPlan], TOP, MOVES, REACH_600)).toBe(149);
  });

  it('goes under the 1:1 header’s two rows of buttons (900 x 600 with the sheet open)', () => {
    const details = box(394.7, 83.3, 454, 109.3);
    const back = box(305.6, 122.5, 454, 148.5);
    const send = box(360, 410, 396, 446);
    expect(toastTop(box(108, 68, 468, 131), [details, back, send], TOP, MOVES, REACH_600)).toBe(148.5);
  });

  it('goes under the lowest control that is under it, not the first', () => {
    const high = box(1100, 70, 1130, 90);
    const low = box(1200, 100, 1230, 120);
    expect(toastTop(box(1066, 68, 1426, 131), [high, low], TOP, MOVES, REACH_900)).toBe(120);
  });

  it('where it started does not count: only its height and its two sides', () => {
    const closed = box(1071, 72, 1101, 102);
    expect(toastTop(box(1066, 300, 1426, 363), [closed], TOP, MOVES, REACH_900)).toBe(102);
  });

  it('with nothing free it stays at the last place it tried', () => {
    // A list of rows that are buttons, 40 px apart: each move lands on the next one.
    const rows = [0, 1, 2, 3, 4, 5, 6, 7].map((i) => box(1000, 70 + i * 40, 1400, 100 + i * 40));
    // 68 -> 140 (under rows 0 and 1) -> 220 (rows 2 and 3) -> 300 (rows 4 and 5): three moves, and it stays.
    expect(toastTop(box(1066, 68, 1426, 131), rows, TOP, MOVES, REACH_900)).toBe(300);
  });

  it('makes no more moves than it may', () => {
    const rows = [0, 1, 2, 3].map((i) => box(1000, 70 + i * 40, 1400, 100 + i * 40));
    expect(toastTop(box(1066, 68, 1426, 131), rows, TOP, 1, REACH_900)).toBe(140);
    expect(toastTop(box(1066, 68, 1426, 131), rows, TOP, 0, REACH_900)).toBe(68);
  });

  it('never goes under a control that ends further down than it may go', () => {
    // A tall panel of controls: its bottom is more than 40 percent of the window under the top bar.
    const tabs = box(1071, 72, 1101, 102);
    const panel = box(1000, 140, 1400, 500);
    // 68 -> 102 (under the tabs). There it lies on the panel, whose bottom is too far down.
    expect(toastTop(box(1066, 68, 1426, 131), [tabs, panel], TOP, MOVES, REACH_600)).toBe(102);
    // Under the stack from the start, and nothing else: it does not move at all.
    expect(toastTop(box(1066, 68, 1426, 131), [box(1000, 110, 1400, 500)], TOP, MOVES, REACH_600)).toBe(68);
  });

  it('a control out of reach does not keep it on one that is in reach: three toasts on the tab row and a tall list', () => {
    // Three toasts are 205 px high: from the start the stack lies on the tab row and on the list.
    const tabs = box(1071, 72, 1101, 102);
    const list = box(1000, 140, 1400, 500);
    // 68 -> 102: off the tab row. The list's bottom (500) is past 68 + 240, so it stays there.
    expect(toastTop(box(1066, 68, 1426, 273), [tabs, list], TOP, MOVES, REACH_600)).toBe(102);
    // Two rows in reach under it: it goes under the lower one.
    const cancelPlan = box(1100, 123, 1188, 149);
    expect(toastTop(box(1066, 68, 1426, 273), [tabs, cancelPlan, list], TOP, MOVES, REACH_600)).toBe(149);
  });

  it('may go down as far as the limit itself', () => {
    const tall = box(1100, 70, 1200, TOP + REACH_600);
    expect(toastTop(box(1066, 68, 1426, 131), [tall], TOP, MOVES, REACH_600)).toBe(TOP + REACH_600);
  });

  it('a taller stack (three toasts) is checked with its whole height', () => {
    const runPlan = box(945, 138, 1077, 170);
    expect(toastTop(box(1066, 68, 1426, 131), [runPlan], TOP, MOVES, REACH_900)).toBe(68);
    expect(toastTop(box(1066, 68, 1426, 273), [runPlan], TOP, MOVES, REACH_900)).toBe(170);
  });
});

describe('reachDown', () => {
  /** 40 percent of the window, 12 px kept at the window's bottom, as the stack uses them. */
  const reach = (windowHeight: number, height: number) => reachDown(windowHeight, 0.4, TOP, height, 12);

  it('is 40 percent of the window for a toast that has the room', () => {
    expect(reach(900, 63)).toBe(360);
    expect(reach(600, 63)).toBe(240);
  });

  it('is less for a high stack: its bottom stays 12 px inside the window', () => {
    // Three long errors, 511 px high, in a 640 px high window: 640 - 12 - 68 - 511.
    expect(reach(640, 511)).toBe(49);
  });

  it('with that reach, three long errors go under the tab row and not on to Send (1024 x 640)', () => {
    const closed = box(863, 72, 893, 102);
    const findClose = box(849, 116, 875, 142);
    const send = box(853, 578, 889, 614);
    // 68 -> 102. Under the find bar (142) the stack would end at 653, out of the window: it stays.
    expect(toastTop(box(650, 68, 1010, 579), [closed, findClose, send], TOP, MOVES, reach(640, 511))).toBe(102);
  });

  it('is nothing for a stack that fills the window or is higher than it', () => {
    expect(reach(640, 560)).toBe(0);
    expect(reach(640, 700)).toBe(0);
    // No reach: no move, even off a control right under the top bar.
    expect(toastTop(box(650, 68, 1010, 628), [box(863, 72, 893, 102)], TOP, MOVES, 0)).toBe(68);
  });
});

describe('stackPlace: the place, and whether the stack folds', () => {
  /** As the stack measures: each control with 4 px around it. */
  const controls = (...boxes: Box[]) => boxes.map((c) => grown(c, 4));
  /** A stack of this height in a window of this height: it may go down 40 percent of the window, and stays 12 px inside it. */
  const shape = (height: number, windowHeight: number): Shape => ({ height, reach: reachDown(windowHeight, 0.4, TOP, height, 12) });
  /** Measured: a one-line toast is 45 px high, three of them with 8 px between are 151. */
  const THREE = 151;
  /** One toast, 8 px, and the 22 px line "+2 more". */
  const FOLDED = 75;

  it('a stack with a free place does not fold', () => {
    const closed = box(1071, 72, 1101, 102);
    expect(stackPlace({ left: 1066, right: 1426 }, shape(THREE, 900), shape(FOLDED, 900), controls(closed), TOP, MOVES)).toEqual({ top: 106, folded: false });
    expect(stackPlace({ left: 1066, right: 1426 }, shape(THREE, 900), shape(FOLDED, 900), [], TOP, MOVES)).toEqual({ top: 68, folded: false });
  });

  it('folds off "Deny": three toasts over an approval card (Home, 1024 x 640)', () => {
    // Measured on the page: the request tabs, the plan header, the card's "Copy command" and "Deny".
    const page = controls(
      box(453, 72, 679, 102), box(679, 77, 699, 97), box(825, 72, 855, 102), box(863, 72, 893, 102),
      box(205, 110, 713.4, 162), box(719.4, 125, 741.4, 147), box(737.4, 125, 759.4, 147), box(769.4, 123, 857, 149), box(869, 124, 893, 148),
      box(823, 160.9, 847, 184.9),
      box(596.3, 302.9, 686.2, 334.9),
    );
    const sides = { left: 650, right: 1010 };
    // The whole stack: 68 -> 188.9, under "Copy command". There it ends at 339.9, on "Deny" (298.9 to 338.9),
    // and under "Deny" is further than it may go (68 + 256).
    expect(toastTop(box(650, 68, 1010, 68 + THREE), page, TOP, MOVES, shape(THREE, 640).reach)).toBe(188.9);
    // Folded, it ends at 263.9: 35 px over "Deny".
    expect(stackPlace(sides, shape(THREE, 640), shape(FOLDED, 640), page, TOP, MOVES)).toEqual({ top: 188.9, folded: true });
    // Two toasts (98 px) end at 286.9: free, not folded.
    expect(stackPlace(sides, shape(98, 640), shape(FOLDED, 640), page, TOP, MOVES)).toEqual({ top: 188.9, folded: false });
  });

  it('with no free place for the folded stack either, it stays folded where it last looked: a list of task rows (Home, 1440 x 900, the sheet open)', () => {
    // The request tabs, the plan header, three task rows (each one button, 632 px wide: not wider
    // than twice the stack, so each counts whole) and "Plan room" under them.
    const page = controls(
      box(452, 72, 678, 102), box(678, 77, 698, 97), box(824, 72, 854, 102), box(862, 72, 892, 102),
      box(204, 110, 712.4, 162), box(718.4, 125, 740.4, 147), box(736.4, 125, 758.4, 147), box(768.4, 123, 856, 149), box(868, 124, 892, 148),
      box(228, 221.9, 860, 301.9), box(228, 303.9, 860, 365.9), box(228, 367.9, 860, 429.9),
      box(755.4, 439.9, 850, 465.9),
    );
    // Whole and folded: 68 -> under the header or the first row -> under the second row. The third
    // row ends further down than the stack may go (68 + 360).
    expect(stackPlace({ left: 648, right: 1008 }, shape(THREE, 900), shape(FOLDED, 900), page, TOP, MOVES)).toEqual({ top: 369.9, folded: true });
  });

  it('a stack of one toast cannot fold: it stays where it last looked', () => {
    const rows = [0, 1, 2, 3, 4, 5, 6, 7].map((i) => box(1000, 70 + i * 40, 1400, 100 + i * 40));
    expect(stackPlace({ left: 1066, right: 1426 }, shape(63, 900), null, rows, TOP, MOVES)).toEqual({ top: 300, folded: false });
  });

  it('the folded stack looks with its own reach: it may go where the whole stack would leave the window', () => {
    // Three long errors, 511 px high, in a 640 px high window: the whole stack can go down 49 px.
    const findClose = box(849, 116, 875, 142);
    const page = [box(863, 72, 893, 102), findClose];
    expect(shape(511, 640).reach).toBe(49);
    // The whole stack stays on the find bar's button; folded (one long error and the line), it goes under it.
    expect(stackPlace({ left: 650, right: 1010 }, shape(511, 640), shape(195, 640), page, TOP, MOVES)).toEqual({ top: 142, folded: true });
  });
});

describe('frontToast: the one toast of a folded stack', () => {
  const toast = (id: number, kind: string) => ({ id, kind });

  it('is the newest toast', () => {
    expect(frontToast([toast(3, 'info'), toast(2, 'success'), toast(1, 'progress')])?.id).toBe(3);
  });

  it('is the newest error when the stack has one: an error is never the one that is hidden', () => {
    expect(frontToast([toast(3, 'info'), toast(2, 'error'), toast(1, 'error')])?.id).toBe(2);
    expect(frontToast([toast(3, 'error'), toast(2, 'info')])?.id).toBe(3);
  });

  it('is nothing for no toast', () => {
    expect(frontToast([])).toBeUndefined();
  });
});

describe('countedBox: a control much wider than the stack counts by what it shows', () => {
  const STACK = 360;
  const never = () => {
    throw new Error('not asked');
  };

  it('a whole-width row whose label is at the left: the stack at the right stays at the top', () => {
    // Measured in the Plan room (1440 x 900): the header "Not in a plan" is one button, 860 px wide,
    // and its text ends at x 715. Here it is right under the top bar.
    const row = box(323, 70, 1183, 102);
    const label = box(337, 78, 715, 93);
    expect(countedBox(row, STACK, false, () => label)).toEqual(label);
    const stack = box(1066, 68, 1426, 113);
    expect(toastTop(stack, [grown(row, 4)], TOP, MOVES, REACH_900)).toBe(106);
    expect(toastTop(stack, [grown(countedBox(row, STACK, false, () => label)!, 4)], TOP, MOVES, REACH_900)).toBe(68);
  });

  it('a row that shows something at the right too still counts there', () => {
    // The question row of the Plan room: its text starts at x 337, its time ends at x 1169.
    const row = box(323, 70, 1183, 104);
    const shown = countedBox(row, STACK, false, () => box(337, 79, 1169, 94))!;
    expect(toastTop(box(1066, 68, 1426, 113), [grown(shown, 4)], TOP, MOVES, REACH_900)).toBe(98);
  });

  it('a press area that shows nothing of its own (it lies over a whole task row) does not count', () => {
    expect(countedBox(box(323, 378, 1183, 576), STACK, false, () => null)).toBeNull();
  });

  it('what a control shows counts only inside the part of it that shows', () => {
    // A label that is cut at the row's right edge, and one that lies outside the part that shows.
    expect(countedBox(box(100, 70, 900, 102), STACK, false, () => box(120, 78, 1040, 93))).toEqual(box(120, 78, 900, 93));
    expect(countedBox(box(100, 70, 900, 102), STACK, false, () => box(120, 20, 400, 60))).toBeNull();
  });

  it('a control up to twice as wide as the stack counts whole: what it shows is not asked for', () => {
    // A task row of Home is 632 px wide; a control of exactly twice the stack's width.
    expect(countedBox(box(228, 222, 860, 302), STACK, false, never)).toEqual(box(228, 222, 860, 302));
    expect(countedBox(box(100, 70, 820, 102), STACK, false, never)).toEqual(box(100, 70, 820, 102));
  });

  it('a field you type in counts whole, however wide: its text can be anywhere in it', () => {
    expect(countedBox(box(100, 70, 1100, 104), STACK, true, never)).toEqual(box(100, 70, 1100, 104));
  });

  it('follows the stack’s width: a narrow window has a narrower stack', () => {
    // The stack is 300 px wide: a 632 px row is wider than twice of it.
    expect(countedBox(box(10, 222, 642, 302), 300, false, () => box(24, 240, 200, 256))).toEqual(box(24, 240, 200, 256));
  });
});

describe('paced: at most one run in a time', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('runs at once, and once more at the end of the time for all the calls in it', () => {
    const run = vi.fn();
    const call = paced(run, 250);
    call();
    expect(run).toHaveBeenCalledTimes(1);
    call();
    call();
    vi.advanceTimersByTime(249);
    expect(run).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1);
    expect(run).toHaveBeenCalledTimes(2);
  });

  it('text that streams in for a second makes five runs, not one for each change', () => {
    const run = vi.fn();
    const call = paced(run, 250);
    for (let at = 0; at < 1000; at += 10) {
      call();
      vi.advanceTimersByTime(10);
    }
    // At 0, 250, 500, 750 and 1000 ms.
    expect(run).toHaveBeenCalledTimes(5);
  });

  it('with no call in the time it does not run again, and the next call runs at once', () => {
    const run = vi.fn();
    const call = paced(run, 250);
    call();
    vi.advanceTimersByTime(1000);
    expect(run).toHaveBeenCalledTimes(1);
    call();
    expect(run).toHaveBeenCalledTimes(2);
  });

  it('stop drops the run that waits', () => {
    const run = vi.fn();
    const call = paced(run, 250);
    call();
    call();
    call.stop();
    vi.advanceTimersByTime(1000);
    expect(run).toHaveBeenCalledTimes(1);
  });
});

describe('restLook: the look when the page has come to rest', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());
  /** `--d-4` and two frames. */
  const WAIT = 414;

  it('a second toast within the wait does not start the wait over: the look comes once, at the end of the first wait', () => {
    const look = vi.fn();
    const rest = restLook(() => WAIT, look);
    rest.after(false);
    vi.advanceTimersByTime(200);
    rest.after(false);
    vi.advanceTimersByTime(213);
    expect(look).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(look).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1000);
    expect(look).toHaveBeenCalledTimes(1);
  });

  it('a progress toast that changes every 100 ms still gets its look, and a change after it starts a new wait', () => {
    const look = vi.fn();
    const rest = restLook(() => WAIT, look);
    for (let at = 0; at < 1000; at += 100) {
      rest.after(false);
      vi.advanceTimersByTime(100);
    }
    // The first wait ends at 414 ms; the change at 500 ms starts the second, which ends at 914 ms.
    expect(look).toHaveBeenCalledTimes(2);
  });

  it('a change of the page’s layout starts the wait over: its boxes move from then on', () => {
    const look = vi.fn();
    const rest = restLook(() => WAIT, look);
    rest.after(false);
    vi.advanceTimersByTime(300);
    rest.after(true);
    vi.advanceTimersByTime(413);
    expect(look).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(look).toHaveBeenCalledTimes(1);
  });

  it('stop drops the look that waits (the last toast went)', () => {
    const look = vi.fn();
    const rest = restLook(() => WAIT, look);
    rest.after(false);
    rest.stop();
    vi.advanceTimersByTime(1000);
    expect(look).not.toHaveBeenCalled();
    // The next toast starts a wait of its own.
    rest.after(false);
    vi.advanceTimersByTime(WAIT);
    expect(look).toHaveBeenCalledTimes(1);
  });
});

describe('grown', () => {
  it('keeps a gap around a control: "+" is 3 px left of the stack, and counts with a 4 px gap', () => {
    const plus = box(1033, 72, 1063, 102);
    expect(grown(plus, 4)).toEqual(box(1029, 68, 1067, 106));
    expect(toastTop(box(1066, 68, 1426, 131), [grown(plus, 4)], TOP, MOVES, REACH_900)).toBe(106);
  });
});

describe('visiblePart', () => {
  it('is the whole box when nothing clips it', () => {
    expect(visiblePart(box(10, 10, 50, 30), [])).toEqual(box(10, 10, 50, 30));
    expect(visiblePart(box(10, 10, 50, 30), [box(0, 0, 100, 100)])).toEqual(box(10, 10, 50, 30));
  });

  it('is cut to the list it scrolls in', () => {
    // A button half scrolled out at the top of a conversation that starts at y 110.
    expect(visiblePart(box(700, 95, 760, 125), [box(123, 110, 843, 500)])).toEqual(box(700, 110, 760, 125));
  });

  it('is cut by every frame around it', () => {
    expect(visiblePart(box(0, 0, 100, 100), [box(10, 0, 100, 100), box(0, 20, 90, 100)])).toEqual(box(10, 20, 90, 100));
  });

  it('is nothing for a control that is scrolled out, or that only touches the edge', () => {
    expect(visiblePart(box(700, 60, 760, 90), [box(123, 110, 843, 500)])).toBeNull();
    expect(visiblePart(box(700, 80, 760, 110), [box(123, 110, 843, 500)])).toBeNull();
  });
});

describe('counts: must a toast keep off this control?', () => {
  const pressable: Seen = { inStack: false, above: false, disabled: false, visibility: 'visible', pointerEvents: 'auto', onTop: true };

  it('yes for a control of the page that shows and takes a press', () => {
    expect(counts(pressable)).toBe(true);
  });

  it('no for a control of the toast stack itself (a toast’s own button, its Dismiss)', () => {
    expect(counts({ ...pressable, inStack: true })).toBe(false);
  });

  it('no for a control in a layer above the stack (a dialog’s buttons): the stack never lies on it', () => {
    expect(counts({ ...pressable, above: true })).toBe(false);
  });

  it('no for a disabled control', () => {
    expect(counts({ ...pressable, disabled: true })).toBe(false);
  });

  it('no for a control that is not shown', () => {
    expect(counts({ ...pressable, visibility: 'hidden' })).toBe(false);
    expect(counts({ ...pressable, visibility: 'collapse' })).toBe(false);
  });

  it('no for a control that takes no press', () => {
    expect(counts({ ...pressable, pointerEvents: 'none' })).toBe(false);
  });

  it('no for a control under something else (the office’s buttons under the quest or the IDE)', () => {
    expect(counts({ ...pressable, onTop: false })).toBe(false);
  });
});

describe('onTop: does a press at a point reach this control?', () => {
  type Part = { name: string; parent?: Part; contains(other: Part): boolean };
  /** A part of the page that knows what is inside it, as an Element does. */
  const part = (name: string, parent?: Part): Part => ({
    name,
    parent,
    contains(other) {
      for (let p: Part | undefined = other; p; p = p.parent) if (p === this) return true;
      return false;
    },
  });

  const page = part('page');
  const shell = part('shell', page);
  const button = part('button', shell);
  const icon = part('icon', button);
  const stack = part('stack', page);
  const toast = part('toast', stack);
  const quest = part('quest', page);
  const dialog = part('dialog', page);
  const scrim = part('scrim', dialog);
  /** Left out: the toast stack, and the dialog, which is drawn above it. */
  const aside = (hit: Part) => stack.contains(hit) || dialog.contains(hit);

  it('yes when the control is the first thing hit', () => {
    expect(onTop(button, [button, shell, page], aside)).toBe(true);
  });

  it('yes when a part of the control is hit first (its icon)', () => {
    expect(onTop(button, [icon, button, shell, page], aside)).toBe(true);
  });

  it('the toast stack does not count: a toast that lies on the control hides nothing', () => {
    expect(onTop(button, [toast, stack, icon, button, shell, page], aside)).toBe(true);
  });

  it('a layer above the stack does not count: the button is there again when the dialog closes', () => {
    expect(onTop(button, [scrim, dialog, button, shell, page], aside)).toBe(true);
  });

  it('no when another layer of the page is hit first (the quest over the office)', () => {
    expect(onTop(button, [quest, button, shell, page], aside)).toBe(false);
    expect(onTop(button, [scrim, toast, quest, button, shell, page], aside)).toBe(false);
  });

  it('no when only what is around the control is hit (the control takes no press there)', () => {
    expect(onTop(button, [shell, page], aside)).toBe(false);
  });

  it('no when nothing is hit: the point is outside the window, or only a toast is there', () => {
    expect(onTop(button, [], aside)).toBe(false);
    expect(onTop(button, [toast, stack], aside)).toBe(false);
  });
});

describe('ms', () => {
  it('reads a CSS time in ms or in s', () => {
    expect(ms('380ms')).toBe(380);
    expect(ms('0.38s')).toBe(380);
    expect(ms('.5s')).toBe(500);
    expect(ms('0ms')).toBe(0);
  });

  it('reads a custom property’s value, which can start with a space', () => {
    expect(ms(' 380ms')).toBe(380);
  });

  it('refuses what is not a time: it does not make a wait of 0 from it', () => {
    expect(() => ms('')).toThrow('Not a CSS time');
    expect(() => ms('380')).toThrow('Not a CSS time');
    expect(() => ms('var(--d-3)')).toThrow('Not a CSS time');
  });

  it('reads every value the design tokens give --d-4, the time the stack waits for the page', () => {
    const values = [...tokens.matchAll(/--d-4:\s*([^;]+);/g)].map((m) => m[1]);
    // The usual value, and the one for reduced motion.
    expect(values.length).toBeGreaterThanOrEqual(2);
    for (const value of values) expect(ms(value), value).toBeGreaterThanOrEqual(0);
  });
});

describe('a toast’s words stay in the toast', () => {
  /** The declarations of one rule in the toast stack's style. */
  const rule = (selector: string): string => {
    const found = new RegExp(`^\\s*\\${selector} \\{([^}]*)\\}`, 'm').exec(toastsView);
    if (!found) throw new Error(`no rule ${selector}`);
    return found[1];
  };

  it('a long word with no place to break (a branch, a file name) breaks anywhere, in the title and in the text', () => {
    expect(rule('.title')).toContain('overflow-wrap: anywhere;');
    expect(rule('.body')).toContain('overflow-wrap: anywhere;');
  });

  it('the text column can get narrower than its longest word', () => {
    expect(rule('.txt')).toContain('min-width: 0;');
  });
});
