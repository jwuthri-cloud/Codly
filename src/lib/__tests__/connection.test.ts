// A browser page on `codly serve` and its link: the waits between tries (connection.ts), and
// the store with the browser's transport (api.ts, WebTransport) when the server stops, comes
// back, or answers that the page is signed out. The app's window has no such link.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mkAgent } from '../../office/demo/script';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import type { Snapshot } from '../gen/Snapshot';
import type { Workspace } from '../gen/Workspace';
import { BACK_MS, Reconnect, waitAfter, type Clock, type Link } from '../state/connection';

/** The app's window: its transport is a Tauri Channel (mocked here; only that path imports it). */
const tauri = vi.hoisted(() => ({ calls: 0, sinks: [] as ((b: unknown) => void)[], answer: null as (() => Promise<unknown>) | null }));
vi.mock('@tauri-apps/api/core', () => ({
  Channel: class {
    constructor(sink: (b: unknown) => void) {
      tauri.sinks.push(sink);
    }
  },
  invoke: async () => {
    tauri.calls++;
    return tauri.answer!();
  },
}));

/** A clock the test moves by hand. */
function manualClock() {
  let now = 0;
  let next = 1;
  const due = new Map<number, { at: number; f: () => void }>();
  const clock: Clock = {
    now: () => now,
    setTimeout: (f, ms) => {
      due.set(next, { at: now + ms, f });
      return next++;
    },
    clearTimeout: (h) => void due.delete(h as number),
  };
  /** Move to the next timer and run it; its wait, in ms. */
  const fire = () => {
    const [id, t] = [...due.entries()].sort((a, b) => a[1].at - b[1].at)[0];
    due.delete(id);
    const wait = t.at - now;
    now = t.at;
    t.f();
    return wait;
  };
  return { clock, fire, pending: () => due.size };
}

/** Let the promises that wait settle (the test fakes the timers). */
const flush = async () => {
  for (let i = 0; i < 20; i++) await Promise.resolve();
};

describe('the waits between tries', () => {
  it('are 1, 2, 5 and 10 s, then 10 s with no end', () => {
    expect([1, 2, 3, 4, 5, 6, 50].map(waitAfter)).toEqual([1000, 2000, 5000, 10_000, 10_000, 10_000, 10_000]);
  });

  it('run by themselves while Codly stays down, and say when the last try was', async () => {
    const { clock, fire } = manualClock();
    const seen: Link[] = [];
    let tries = 0;
    const r = new Reconnect(async () => {
      tries++;
      r.lost(false);
    }, (l) => seen.push(l), clock);
    r.lost(false); // the stream dropped and the first try at once failed
    expect(r.link).toEqual({ kind: 'down', lastTry: 0, trying: false });
    const waits: number[] = [];
    for (let i = 0; i < 6; i++) {
      waits.push(fire());
      await flush();
    }
    expect(waits).toEqual([1000, 2000, 5000, 10_000, 10_000, 10_000]);
    expect(tries).toBe(6);
    expect(r.link).toEqual({ kind: 'down', lastTry: 38_000, trying: false });
    expect(seen).toContainEqual({ kind: 'down', lastTry: 18_000, trying: true }); // the bar knows a try runs
  });

  it('say "Connected again" for 2 s after a success in the middle, and start again from 1 s', async () => {
    const { clock, fire, pending } = manualClock();
    let up = false;
    const r: Reconnect = new Reconnect(async () => (up ? r.reached() : r.lost(false)), () => {}, clock);
    r.lost(false);
    fire(); // 1 s: still down
    await flush();
    up = true;
    expect(fire()).toBe(2000);
    await flush();
    expect(r.link).toEqual({ kind: 'back' });
    r.reached(); // another connect (a resync) does not make it stay longer
    expect(fire()).toBe(BACK_MS);
    expect(r.link).toEqual({ kind: 'connected' });
    expect(pending()).toBe(0);
    r.lost(false);
    expect(fire()).toBe(1000);
  });

  it('try at once on "Try now", in place of the wait', async () => {
    const { clock, fire, pending } = manualClock();
    let tries = 0;
    const r = new Reconnect(async () => {
      tries++;
      r.lost(false);
    }, () => {}, clock);
    r.lost(false);
    r.lost(false);
    r.lost(false); // the next wait is 5 s
    await r.tryNow();
    expect(tries).toBe(1);
    expect(pending()).toBe(1); // one wait, not the old one and a new one
    expect(fire()).toBe(10_000);
  });

  it('never run two tries at once', async () => {
    const { clock, fire } = manualClock();
    let running = 0;
    let most = 0;
    let finish = () => {};
    const r = new Reconnect(
      () =>
        new Promise<void>((done) => {
          most = Math.max(most, ++running);
          finish = () => {
            running--;
            done();
          };
        }),
      () => {},
      clock,
    );
    r.lost(false);
    const first = r.tryNow();
    void r.tryNow(); // "Try now" pressed again while it runs
    r.lost(false); // a connect that started elsewhere failed meanwhile: its wait ends while this try runs
    fire();
    await flush();
    expect(most).toBe(1);
    finish(); // this try was taken over by that newer connect: it reports nothing
    await first;
    expect(running).toBe(0);
    expect(r.link.kind).toBe('down');
    fire(); // the waits went on
    await flush();
    expect(running).toBe(1);
  });

  it('stop when Codly says the page is signed out', async () => {
    const { clock, pending } = manualClock();
    const r = new Reconnect(async () => {}, () => {}, clock);
    r.lost(false);
    r.lost(true);
    expect(r.link).toEqual({ kind: 'signedOut' });
    expect(pending()).toBe(0);
  });

  it('do nothing while the link is up', async () => {
    const { clock, pending } = manualClock();
    const attempt = vi.fn(async () => {});
    const r = new Reconnect(attempt, () => {}, clock);
    r.reached();
    await r.tryNow();
    expect([r.link, attempt.mock.calls.length, pending()]).toEqual([{ kind: 'connected' }, 0, 0]);
  });
});

// ───────────── the store, with the real transports ─────────────

/** The browser's event stream, driven by the test. */
class FakeEventSource {
  static all: FakeEventSource[] = [];
  onerror: (() => void) | null = null;
  closed = false;
  private on = new Map<string, (e: Event) => void>();
  constructor(readonly url: string) {
    FakeEventSource.all.push(this);
  }
  addEventListener(type: string, f: (e: Event) => void) {
    this.on.set(type, f);
  }
  close() {
    this.closed = true;
  }
  send(type: string, data: unknown) {
    this.on.get(type)?.({ data: JSON.stringify(data) } as MessageEvent<string>);
  }
  fail() {
    this.onerror?.();
  }
}
const streams = () => FakeEventSource.all.filter((s) => s.url === '/api/events');
const last = () => streams().at(-1)!;

const lead = mkAgent('lead1', 'Lead', 'claude', 'lead', 0, 'working', 'lead');
const office = { id: 'w1', name: 'w', repo_path: '/code/w' } as Workspace;
const said = (id: number, item: string): EventRow => ({ id, ts: 1000 + id, agent_id: 'lead1', task_id: null, run_id: null, kind: { type: 'message', item_id: item, text: item, phase: 'final' } as EventKind });
const snap = (events: EventRow[]): Snapshot =>
  ({
    as_of: Math.max(0, ...events.map((e) => e.id)), clis: [], accounts: [], workspace: office, settings: null, workspace_team: null, request_prefs: {},
    agents: [lead], plans: [], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events,
  }) as unknown as Snapshot;

/** What `codly serve` answers to a command: the cheap one the page asks after a failed
 *  stream, and the others (`url`). */
let probe: (url: string) => Promise<Response>;
const json = (value: unknown) => new Response(JSON.stringify(value), { headers: { 'content-type': 'application/json' } });

/** A page (the store's modules listen to its size and ask whether it shows), with `flag`. */
function page(flag: Record<string, unknown>) {
  vi.stubGlobal('window', { ...flag, innerWidth: 1440, addEventListener: () => {}, removeEventListener: () => {} });
  vi.stubGlobal('document', { visibilityState: 'visible' });
}

async function webStore() {
  vi.resetModules();
  page({ __CODLY_WEB__: true });
  vi.stubGlobal('EventSource', FakeEventSource);
  vi.stubGlobal('fetch', vi.fn((url: string) => probe(url)));
  return (await import('../state/store.svelte')).store;
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date'] });
  FakeEventSource.all = [];
  probe = () => Promise.reject(new TypeError('Failed to fetch'));
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('a browser page that loses codly serve', () => {
  it('keeps the thread, says it is down, tries again by itself and catches up', async () => {
    const store = await webStore();
    const done = store.connect();
    await flush();
    last().send('snapshot', snap([said(1, 'm1')]));
    await done;
    expect(store.link).toEqual({ kind: 'connected' });

    // Codly stops: the stream drops, the try at once finds nothing.
    last().fail();
    await flush();
    expect(streams()).toHaveLength(2);
    last().fail();
    await flush();
    expect(store.link).toMatchObject({ kind: 'down', trying: false });
    expect([store.error, store.ready, store.workspace?.id]).toEqual([null, true, 'w1']);
    expect(store.live.lead1.messages.map((m) => m.itemId)).toEqual(['m1']); // still readable

    // Nothing before 1 s; then one try, which fails too.
    await vi.advanceTimersByTimeAsync(999);
    await flush();
    expect(streams()).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(1);
    await flush();
    expect(streams()).toHaveLength(3);
    last().fail();
    await flush();
    await vi.advanceTimersByTimeAsync(2000);
    await flush();
    expect(streams()).toHaveLength(4);

    // Codly is back: what was said meanwhile comes in, and the bar says so for 2 s.
    last().send('snapshot', snap([said(1, 'm1'), said(2, 'm2')]));
    await flush();
    expect(store.link).toEqual({ kind: 'back' });
    expect(store.live.lead1.messages.map((m) => m.itemId)).toEqual(['m1', 'm2']);
    await vi.advanceTimersByTimeAsync(BACK_MS);
    expect(store.link).toEqual({ kind: 'connected' });
    expect(streams()).toHaveLength(4);
  });

  it('says it is signed out when Codly answers 401, and does not loop', async () => {
    const store = await webStore();
    const done = store.connect();
    await flush();
    last().send('snapshot', snap([]));
    await done;
    probe = async () => new Response('Open the link `codly serve` printed: it carries the token.', { status: 401 });
    last().fail();
    await flush();
    last().fail();
    await flush();
    expect(store.link).toEqual({ kind: 'signedOut' });
    expect(fetch).toHaveBeenCalledWith('/api/cmd/start_problem', expect.objectContaining({ method: 'POST' }));
    await vi.advanceTimersByTimeAsync(120_000);
    await flush();
    expect(streams()).toHaveLength(2);
  });

  it('takes any other answer for "down"', async () => {
    const store = await webStore();
    probe = async () => new Response('Codly is still starting', { status: 503 });
    const done = store.connect();
    await flush();
    last().fail();
    await done;
    expect(store.link).toMatchObject({ kind: 'down' });
  });

  it('at the first connect: the start-up screen says why, tries again by itself, and "Try again" opens the stream', async () => {
    const store = await webStore();
    const done = store.connect();
    await flush();
    last().fail();
    await done;
    expect(store.error).toBe('Can’t reach Codly. Is `codly serve` still running?');
    expect(store.link).toMatchObject({ kind: 'down' });
    await vi.advanceTimersByTimeAsync(1000);
    await flush();
    expect(streams()).toHaveLength(2); // by itself
    last().fail();
    await flush();

    const again = store.retry(); // "Try again": no retry_core, which needs the server too
    await flush();
    expect(streams()).toHaveLength(3);
    last().send('snapshot', snap([]));
    await again;
    expect([store.error, store.link.kind, store.workspace?.id]).toEqual([null, 'back', 'w1']);
    expect(fetch).not.toHaveBeenCalledWith('/api/cmd/retry_core', expect.anything());
  });

  it('goes on trying when a newer connect takes over a try that never ended', async () => {
    const store = await webStore();
    const done = store.connect();
    await flush();
    last().send('snapshot', snap([]));
    await done;
    last().fail();
    await flush();
    last().fail();
    await flush();
    await vi.advanceTimersByTimeAsync(1000);
    await flush();
    expect(streams()).toHaveLength(3); // a try whose stream neither opens nor fails (a dead tunnel)
    expect(store.link).toMatchObject({ kind: 'down', trying: true });

    void store.resync(); // the tab came back after a long time hidden
    await flush();
    expect(streams()[2].closed).toBe(true);
    last().fail();
    await flush();
    await vi.advanceTimersByTimeAsync(10_000);
    await flush();
    expect(streams().length).toBeGreaterThan(4); // the waits went on
    expect(store.link).toMatchObject({ kind: 'down' });
    last().send('snapshot', snap([]));
    await flush();
    expect(store.link).toEqual({ kind: 'back' });
  });

  it('asks again what waits for Resume when it is back, and not on a plain resync', async () => {
    const store = await webStore();
    // App.svelte asks the core what waits for Resume each time this number changes to one >= 0.
    const sessions: number[] = [];
    const seen = () => sessions.push(store.coreSession);
    seen();
    const done = store.connect();
    await flush();
    last().send('snapshot', snap([]));
    await done;
    seen();

    void store.resync(); // the tab came back: the link was never lost
    await flush();
    last().send('snapshot', snap([]));
    await flush();
    seen();

    last().fail(); // `codly serve` restarts (it holds the work again)
    await flush();
    last().fail();
    await flush();
    seen();
    await vi.advanceTimersByTimeAsync(1000);
    await flush();
    last().send('snapshot', snap([]));
    await flush();
    expect(store.link).toEqual({ kind: 'back' });
    seen();
    await vi.advanceTimersByTimeAsync(BACK_MS);
    seen(); // "Connected again" going is no reason to ask
    expect(sessions).toEqual([-1, 0, 0, 0, 1, 1]);
  });

  it('says so, and keeps the office, when the core fails to start after a restart', async () => {
    const store = await webStore();
    const { connectionBarRoom } = await import('../../shell/ConnectionBar.svelte');
    probe = (url) => (url === '/api/cmd/retry_core' ? Promise.resolve(json(null)) : Promise.reject(new TypeError('Failed to fetch')));
    const done = store.connect();
    await flush();
    last().send('snapshot', snap([said(1, 'm1')]));
    await done;
    expect(connectionBarRoom()).toBe(0);

    last().fail(); // the server restarted, and its core could not open the database
    await flush();
    last().send('failed', 'unable to open database file');
    await flush();
    expect([store.error, store.workspace?.id, store.offline]).toEqual(['unable to open database file', 'w1', true]);
    expect(store.live.lead1.messages.map((m) => m.itemId)).toEqual(['m1']);
    expect(connectionBarRoom()).toBeGreaterThan(0); // the bar says it, and the toasts make room

    const again = store.retry(); // "Try again" on the bar
    await flush();
    expect(store.ready).toBe(true); // the office stays on screen while the core starts
    expect(fetch).toHaveBeenCalledWith('/api/cmd/retry_core', expect.anything());
    last().send('snapshot', snap([said(1, 'm1')]));
    await again;
    expect([store.error, store.offline, connectionBarRoom()]).toEqual([null, false, 0]);
  });

  it('does not let the polls call Codly while it is down', async () => {
    const store = await webStore();
    const done = store.connect();
    await flush();
    last().fail();
    await done;
    expect(store.offline).toBe(true);
  });
});

describe('the app’s window', () => {
  it('never leaves "connected": a failed start and a resync are not a lost link', async () => {
    vi.resetModules();
    page({ __TAURI_INTERNALS__: {} });
    const { store } = await import('../state/store.svelte');
    const links: string[] = [];
    const watch = () => links.push(store.link.kind);

    tauri.answer = () => Promise.reject('unable to open database file');
    await store.connect();
    watch();
    expect(store.error).toBe('unable to open database file');

    tauri.answer = async () => snap([said(1, 'm1')]);
    await store.connect();
    watch();
    tauri.answer = () => Promise.reject('the core stopped');
    tauri.sinks.at(-1)!({ events: [], resync: true }); // the bus lagged
    await flush();
    watch();
    const calls = tauri.calls;
    await vi.advanceTimersByTimeAsync(120_000);
    await flush();
    watch();
    expect(links).toEqual(['connected', 'connected', 'connected', 'connected']);
    // No try runs by itself; the core is not up, so the polls skip (the bar offers "Try again").
    expect([tauri.calls, store.error, store.offline]).toEqual([calls, 'the core stopped', true]);
  });
});
