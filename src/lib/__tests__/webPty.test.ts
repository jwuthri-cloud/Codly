import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { WebTransport } from '../api';

/** The page's side of one server-sent stream: the test opens it, sends data and ends it. */
class FakeEventSource {
  static all: FakeEventSource[] = [];
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  private listeners: ((e: MessageEvent<string>) => void)[] = [];
  url: string;
  constructor(url: string) {
    this.url = url;
    FakeEventSource.all.push(this);
  }
  addEventListener(type: string, f: (e: MessageEvent<string>) => void) {
    if (type === 'data') this.listeners.push(f);
  }
  close() {
    this.closed = true;
  }
  open() {
    this.onopen?.();
  }
  send(text: string) {
    for (const f of this.listeners) f({ data: btoa(text) } as MessageEvent<string>);
  }
  fail() {
    this.onerror?.();
  }
}

const text = (chunks: Uint8Array[]) => chunks.map((c) => String.fromCharCode(...c));

describe('a terminal of `codly serve`', () => {
  beforeEach(() => {
    FakeEventSource.all = [];
    vi.useFakeTimers();
    vi.stubGlobal('EventSource', FakeEventSource);
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('attaches again when its stream ends, from a clean screen, and stops when the terminal is gone', async () => {
    const got: Uint8Array[] = [];
    const attached = new WebTransport().attachPty('p 1', (b) => got.push(b));
    const first = FakeEventSource.all[0];
    expect(first.url).toBe('/api/pty/p%201');
    first.open();
    await attached;
    first.send('$ yes\r\n');

    // Codly let the tab go (it fell behind the output): the stream ends.
    first.fail();
    expect(first.closed).toBe(true);
    vi.advanceTimersByTime(1000);
    const second = FakeEventSource.all[1];
    expect(second.url).toBe('/api/pty/p%201');
    second.open();
    second.send('scrollback');
    second.send('live');
    expect(text(got)).toEqual(['$ yes\r\n', '\x1bc', 'scrollback', 'live']);

    // The terminal closed: the next attach finds nothing (404), and the page stops there.
    second.fail();
    vi.advanceTimersByTime(1000);
    FakeEventSource.all[2].fail();
    await vi.runAllTimersAsync();
    expect(FakeEventSource.all).toHaveLength(3);
  });

  it('says so when the terminal is not there at the first attach', async () => {
    const attached = new WebTransport().attachPty('gone', () => {});
    FakeEventSource.all[0].fail();
    await expect(attached).rejects.toThrow('no such terminal');
    await vi.runAllTimersAsync();
    expect(FakeEventSource.all).toHaveLength(1);
  });
});
