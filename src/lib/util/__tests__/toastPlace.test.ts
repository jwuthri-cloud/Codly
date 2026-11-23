// The toast stack lies over the page and must cover nothing you can press. The boxes below
// are the ones measured in the app (left, top, right, bottom in px): a one-line toast is
// 360 x 63, the stack starts 68 px under the window's top.
import { describe, expect, it } from 'vitest';
import tokens from '../../../styles/tokens.css?raw';
import { BESIDE, counts, grown, ms, onTop, reachDown, toastTop, visiblePart, widthBeside, type Box, type Seen } from '../toastPlace';

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

describe('widthBeside', () => {
  /** The gap to the panel, the least and the most width, as the stack uses them. */
  const beside = (panelRight: number, stackRight: number) => widthBeside(panelRight, stackRight, 12, 220, 360);

  it('takes the free room beside the conversation column (Home, 1440 x 900: column 393..1113)', () => {
    expect(beside(1113, 1426)).toBe(301);
  });

  it('is still there in a 1280 px window (column 313..1033)', () => {
    expect(beside(1033, 1266)).toBe(221);
  });

  it('is never wider than the stack’s own width', () => {
    expect(beside(1353, 1906)).toBe(360);
  });

  it('is nothing in a narrow window (900 x 600: column 123..843)', () => {
    expect(beside(843, 886)).toBeNull();
  });

  it('is nothing with the sheet open (1440 x 900: column 184..904, the stack ends at 1008)', () => {
    expect(beside(904, 1008)).toBeNull();
  });

  it('is nothing beside a room, which is as wide as the window', () => {
    expect(beside(1426, 1426)).toBeNull();
  });

  it('needs the least width in full', () => {
    expect(widthBeside(1000, 1231, 12, 220, 360)).toBeNull();
    expect(widthBeside(1000, 1232, 12, 220, 360)).toBe(220);
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

  it('the same asks whether a panel shows: the quest’s level over the office’s column', () => {
    const column = part('column', shell);
    const message = part('message', column);
    expect(onTop(column, [message, column, shell, page], aside)).toBe(true);
    expect(onTop(column, [quest, message, column, shell, page], aside)).toBe(false);
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

describe('the panels the stack stands beside', () => {
  const views = import.meta.glob<string>(['/src/**/*.svelte'], { query: '?raw', import: 'default', eager: true });
  const source = (file: string): string => {
    const found = Object.entries(views).find(([path]) => path.endsWith(`/${file}`));
    if (!found) throw new Error(`no view ${file}`);
    return found[1];
  };

  it('the mark is an attribute the stack selects on', () => {
    expect(BESIDE).toBe('data-toast-beside');
    expect(source('Toasts.svelte')).toContain('querySelectorAll(`[${BESIDE}]`)');
  });

  it('Home’s conversation column and the first-run quest’s level carry it, and nothing else', () => {
    const marked = Object.entries(views)
      .filter(([, text]) => /<[a-z][^>]*\sdata-toast-beside[\s>]/.test(text))
      .map(([path]) => path.split('/').pop())
      .sort();
    expect(marked).toEqual(['Home.svelte', 'Onboarding.svelte']);
    expect(source('Home.svelte')).toMatch(/<section class="column glass" data-toast-beside[\s>]/);
    expect(source('Onboarding.svelte')).toMatch(/<section class="panel glass" data-toast-beside[\s>]/);
  });
});
