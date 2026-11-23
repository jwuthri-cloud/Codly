// The budget mark opens Settings at the budgets, the last section of a tab whose other sections
// load their rows a moment later. The section stays in place until the user scrolls.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { holdAtTop } from '../holdAtTop';

/** The browser's two observers, as far as holdAtTop uses them: the test says when one fires. */
class Sizes {
  static last: Sizes;
  seen = new Set<unknown>();
  constructor(readonly fire: () => void) {
    Sizes.last = this;
  }
  observe(target: unknown) {
    this.seen.add(target);
  }
  disconnect() {
    this.seen.clear();
  }
  /** `target` changed height: the browser tells the observer only if it watches that element. */
  resized(target: unknown) {
    if (this.seen.has(target)) this.fire();
  }
}
class Sections {
  static last: Sections;
  on = false;
  constructor(readonly fire: () => void) {
    Sections.last = this;
  }
  observe() {
    this.on = true;
  }
  disconnect() {
    this.on = false;
  }
  /** A section was added or removed. */
  changed() {
    if (this.on) this.fire();
  }
}

/** A tab with two sections above the budgets. */
function page() {
  const scrolled = vi.fn();
  const [rules, checks] = [{}, {}];
  const content = { children: [rules, checks] as unknown[] };
  const el = { scrollIntoView: scrolled, parentElement: content };
  content.children.push(el);
  return { el: el as unknown as HTMLElement, content, rules, checks, scrolled };
}

beforeEach(() => {
  vi.stubGlobal('ResizeObserver', Sizes);
  vi.stubGlobal('MutationObserver', Sections);
  vi.stubGlobal('window', new EventTarget());
});
afterEach(() => vi.unstubAllGlobals());

describe('holdAtTop', () => {
  it('scrolls the section to the top at once', () => {
    const p = page();
    holdAtTop(p.el);
    expect(p.scrolled.mock.calls).toEqual([[{ block: 'start' }]]);
  });

  it('puts it back when a section above shows its rows, or a new section appears above', () => {
    const p = page();
    holdAtTop(p.el);
    Sizes.last.resized(p.checks);
    expect(p.scrolled).toHaveBeenCalledTimes(2);

    const late = {}; // a section that had nothing to show until its data came
    p.content.children.unshift(late);
    Sections.last.changed();
    expect(p.scrolled).toHaveBeenCalledTimes(3);
    Sizes.last.resized(late); // and it is watched from then on
    expect(p.scrolled).toHaveBeenCalledTimes(4);
  });

  it.each(['wheel', 'keydown', 'pointerdown'])('lets go at the first %s: the scroll is the user’s', (type) => {
    const p = page();
    holdAtTop(p.el);
    window.dispatchEvent(new Event(type));
    Sizes.last.resized(p.rules);
    Sections.last.changed();
    expect(p.scrolled).toHaveBeenCalledTimes(1);
  });

  it('lets go when asked (the section went away), and no longer listens', () => {
    const p = page();
    const letGo = holdAtTop(p.el);
    const heard = vi.spyOn(Sizes.last, 'disconnect');
    letGo();
    Sizes.last.resized(p.rules);
    Sections.last.changed();
    expect(p.scrolled).toHaveBeenCalledTimes(1);
    window.dispatchEvent(new Event('wheel')); // the listener is gone: nothing lets go twice
    expect(heard).toHaveBeenCalledTimes(1);
  });
});
