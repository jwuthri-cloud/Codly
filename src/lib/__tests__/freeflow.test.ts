import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { appendTranscript, HOLD_MS, HoldOption, pickMime, toBase64 } from '../util/freeflow';

describe('appendTranscript', () => {
  it('keeps what you typed and joins with one space', () => {
    expect(appendTranscript('fix the bug', 'in the sync worker')).toBe('fix the bug in the sync worker');
    expect(appendTranscript('fix the bug ', ' in the sync worker ')).toBe('fix the bug in the sync worker');
  });
  it('fills an empty draft and ignores an empty transcript', () => {
    expect(appendTranscript('', ' hello ')).toBe('hello');
    expect(appendTranscript('  ', 'hello')).toBe('hello');
    expect(appendTranscript('keep me', '   ')).toBe('keep me');
  });
});

describe('pickMime / toBase64', () => {
  it('takes the first supported container', () => {
    expect(pickMime((m) => m === 'audio/mp4')).toBe('audio/mp4');
    expect(pickMime(() => true)).toBe('audio/webm;codecs=opus');
    expect(pickMime(() => false)).toBeNull();
  });
  it('encodes big clips without blowing the stack', () => {
    expect(toBase64(new Uint8Array([104, 105]))).toBe('aGk=');
    expect(toBase64(new Uint8Array(200_000)).length).toBe(Math.ceil(200_000 / 3) * 4);
  });
});

describe('HoldOption', () => {
  const log: string[] = [];
  let h: HoldOption;
  beforeEach(() => {
    vi.useFakeTimers();
    log.length = 0;
    h = new HoldOption({ onStart: () => log.push('start'), onStop: () => log.push('stop'), onCancel: () => log.push('cancel') });
  });
  afterEach(() => vi.useRealTimers());

  it('records while Option is held past the threshold, and stops on release', () => {
    h.keydown({ code: 'AltLeft' });
    vi.advanceTimersByTime(HOLD_MS - 1);
    expect(log).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(log).toEqual(['start']);
    h.keydown({ code: 'AltLeft', repeat: true }); // auto-repeat changes nothing
    h.keyup({ code: 'AltLeft' });
    expect(log).toEqual(['start', 'stop']);
  });

  it('a tap, or Option plus another key, never starts', () => {
    h.keydown({ code: 'AltLeft' });
    vi.advanceTimersByTime(100);
    h.keyup({ code: 'AltLeft' });
    vi.advanceTimersByTime(1000);
    h.keydown({ code: 'AltRight' });
    h.keydown({ code: 'KeyE' }); // Option+E, an accent
    vi.advanceTimersByTime(1000);
    h.keydown({ code: 'AltLeft', metaKey: true }); // ⌘⌥ is a shortcut
    vi.advanceTimersByTime(1000);
    expect(log).toEqual([]);
  });

  it('another key while recording cancels it', () => {
    h.keydown({ code: 'AltLeft' });
    vi.advanceTimersByTime(HOLD_MS);
    h.keydown({ code: 'KeyA' });
    h.keyup({ code: 'AltLeft' });
    expect(log).toEqual(['start', 'cancel']);
  });

  it('losing focus mid-hold cancels, and a new hold can start after', () => {
    h.keydown({ code: 'AltLeft' });
    vi.advanceTimersByTime(HOLD_MS);
    h.abort();
    h.keydown({ code: 'AltLeft' });
    vi.advanceTimersByTime(HOLD_MS);
    h.keyup({ code: 'AltLeft' });
    expect(log).toEqual(['start', 'cancel', 'start', 'stop']);
  });
});
