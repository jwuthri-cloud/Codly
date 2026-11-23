// Portraits never block a paint: asking for one returns at once ('' = show the monogram), the
// drawing code is loaded only after the page has painted, and each agent is drawn once.
import { describe, expect, it, vi } from 'vitest';
import { PORTRAIT_PX, PortraitCache, type Draw, type Sitter } from '../portraitCache';

const sitter = (id: string, over: Partial<Sitter> = {}): Sitter => ({ id, color: 1, look_seed: '', provider: 'claude', role: 'worker', ...over });

function harness(draw: Draw = (a, px) => `png:${a.id}@${px}`) {
  const later: (() => void)[] = [];
  let loads = 0;
  let ready = 0;
  let fail: Error | null = null;
  const drawn: string[] = [];
  const cache = new PortraitCache(
    {
      load: () => {
        loads++;
        return fail
          ? Promise.reject(fail)
          : Promise.resolve<Draw>((a, px) => {
              drawn.push(a.id);
              return draw(a, px);
            });
      },
      later: (cb) => later.push(cb),
    },
    () => ready++,
  );
  /** The page painted: run what was waiting for it (and let the import settle). */
  const paint = async () => {
    for (const cb of later.splice(0)) cb();
    await Promise.resolve();
    await Promise.resolve();
  };
  /** Paint until nothing waits. */
  const settle = async () => {
    for (let i = 0; i < 50 && later.length; i++) await paint();
  };
  return { cache, paint, settle, drawn, loads: () => loads, ready: () => ready, waiting: () => later.length, failWith: (e: Error | null) => (fail = e) };
}

describe('PortraitCache', () => {
  it('asking never draws or loads anything: the monogram shows until the page has painted', () => {
    const h = harness();
    expect(h.cache.get(sitter('a'))).toBe('');
    expect(h.cache.get(sitter('b'))).toBe('');
    expect(h.loads()).toBe(0);
    expect(h.drawn).toEqual([]);
    expect(h.waiting()).toBe(1);
  });

  it('draws one portrait per paint, in the order asked, and says when each is ready', async () => {
    const h = harness();
    for (const id of ['a', 'b', 'c']) h.cache.get(sitter(id));
    await h.paint();
    expect(h.drawn).toEqual(['a']);
    expect(h.ready()).toBe(1);
    expect(h.cache.get(sitter('a'))).toBe(`png:a@${PORTRAIT_PX}`);
    expect(h.cache.get(sitter('b'))).toBe('');
    await h.paint();
    expect(h.drawn).toEqual(['a', 'b']);
    await h.settle();
    expect(h.drawn).toEqual(['a', 'b', 'c']);
    expect(h.ready()).toBe(3);
    expect(h.loads()).toBe(1);
    expect(h.waiting()).toBe(0);
  });

  it('draws an agent once, whatever size and however often it is asked for', async () => {
    const h = harness();
    for (let i = 0; i < 20; i++) h.cache.get(sitter('a'));
    await h.settle();
    for (let i = 0; i < 20; i++) expect(h.cache.get(sitter('a'))).toBe(`png:a@${PORTRAIT_PX}`);
    await h.settle();
    expect(h.drawn).toEqual(['a']);
  });

  it('a new look is a new portrait', async () => {
    const h = harness();
    h.cache.get(sitter('a', { look_seed: 's1' }));
    await h.settle();
    expect(h.cache.get(sitter('a', { look_seed: 's2' }))).toBe('');
    await h.settle();
    expect(h.drawn).toEqual(['a', 'a']);
  });

  it('asked again later, it starts drawing again', async () => {
    const h = harness();
    h.cache.get(sitter('a'));
    await h.settle();
    h.cache.get(sitter('b'));
    await h.settle();
    expect(h.cache.get(sitter('b'))).toBe(`png:b@${PORTRAIT_PX}`);
    expect(h.loads()).toBe(1);
  });

  it('a theme change draws them again; the old portrait shows meanwhile', async () => {
    let theme = 'dark';
    const h = harness((a) => `png:${a.id}:${theme}`);
    h.cache.get(sitter('a'));
    await h.settle();
    theme = 'light';
    const ready = h.ready();
    h.cache.retheme();
    expect(h.ready()).toBe(ready + 1);
    expect(h.cache.get(sitter('a'))).toBe('png:a:dark');
    await h.settle();
    expect(h.cache.get(sitter('a'))).toBe('png:a:light');
  });

  it('no WebGL (the drawing returns nothing): the monogram stays, and it is not drawn over and over', async () => {
    const h = harness(() => '');
    h.cache.get(sitter('a'));
    await h.settle();
    expect(h.cache.get(sitter('a'))).toBe('');
    await h.settle();
    expect(h.drawn).toEqual(['a']);
  });

  it('a drawing that throws is reported, keeps the monogram and does not stop the others', async () => {
    const err = vi.spyOn(console, 'error').mockImplementation(() => {});
    const h = harness((a) => {
      if (a.id === 'a') throw new Error('context lost');
      return `png:${a.id}`;
    });
    h.cache.get(sitter('a'));
    h.cache.get(sitter('b'));
    await h.settle();
    expect(h.cache.get(sitter('a'))).toBe('');
    expect(h.cache.get(sitter('b'))).toBe('png:b');
    expect(err).toHaveBeenCalledTimes(1);
    err.mockRestore();
  });

  it('a failed load is reported and tried again the next time a portrait is asked for', async () => {
    const err = vi.spyOn(console, 'error').mockImplementation(() => {});
    const h = harness();
    h.failWith(new Error('chunk failed'));
    h.cache.get(sitter('a'));
    await h.settle();
    expect(err).toHaveBeenCalledTimes(1);
    expect(h.cache.get(sitter('a'))).toBe('');
    h.failWith(null);
    await h.settle();
    expect(h.cache.get(sitter('a'))).toBe(`png:a@${PORTRAIT_PX}`);
    expect(h.loads()).toBe(2);
    err.mockRestore();
  });
});
