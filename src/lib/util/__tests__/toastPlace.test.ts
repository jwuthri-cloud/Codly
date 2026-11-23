// The toast stack lies over the page and must cover nothing you can press. The boxes below
// are the ones measured in the app (left, top, right, bottom in px): a one-line toast is
// 360 x 63, the stack starts 68 px under the window's top.
import { describe, expect, it } from 'vitest';
import { grown, toastTop, visiblePart, widthBeside, type Box } from '../toastPlace';

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

  it('never goes further down than it may: it stays where it is', () => {
    // A tall panel of controls: its bottom is more than 40 percent of the window under the top bar.
    const tabs = box(1071, 72, 1101, 102);
    const panel = box(1000, 140, 1400, 500);
    // 68 -> 102 (under the tabs). There it lies on the panel, whose bottom is too far down.
    expect(toastTop(box(1066, 68, 1426, 131), [tabs, panel], TOP, MOVES, REACH_600)).toBe(102);
    // Under the stack from the start: it does not move at all.
    expect(toastTop(box(1066, 68, 1426, 131), [box(1000, 110, 1400, 500)], TOP, MOVES, REACH_600)).toBe(68);
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
