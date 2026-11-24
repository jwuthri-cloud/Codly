// Where the tooltip goes. The fourth browser review (defect 19): the tip of a Stop button in the
// agent sheet covered the title of its section, because a tip goes above its control whenever
// there is room. A control can ask for its tip below.
import { describe, expect, it } from 'vitest';
import { tipPlace } from '../tipPlace';

const win = { width: 1024, height: 640 };
const control = (top: number, left = 500) => ({ left, top, bottom: top + 24, width: 60 });

describe('the place of a tooltip', () => {
  it('goes above its control when there is room, as before', () => {
    expect(tipPlace(control(300), 100, 40, '', win)).toEqual({ x: 480, y: 252, below: false });
  });

  it('goes below when there is no room above', () => {
    expect(tipPlace(control(20), 100, 40, '', win)).toEqual({ x: 480, y: 52, below: true });
  });

  it('goes below when its control asks for it, also with room above', () => {
    expect(tipPlace(control(300), 100, 40, 'below', win)).toEqual({ x: 480, y: 332, below: true });
  });

  it('a control that asks for below at the bottom of the window gets it above', () => {
    expect(tipPlace(control(590), 100, 40, 'below', win)).toEqual({ x: 480, y: 542, below: false });
  });

  it('stays inside the window at its sides', () => {
    expect(tipPlace(control(300, 0), 100, 40, '', win).x).toBe(8);
    expect(tipPlace(control(300, 1000), 100, 40, '', win).x).toBe(1024 - 100 - 8);
  });
});
