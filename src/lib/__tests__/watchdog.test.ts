import { describe, expect, it } from 'vitest';
import type { WatchdogSettings } from '../gen/WatchdogSettings';
import { store } from '../state/store.svelte';
import { WATCHDOG_DETAIL, WATCHDOG_LIMITS, limitChoices, limitLabel, withLimit } from '../util/watchdog';

describe('the watchdog limits in Settings', () => {
  it('has a row for every limit the core has', () => {
    const fields: Record<keyof WatchdogSettings, true> = { quiet_secs: true, quiet_tool_secs: true, idle_secs: true };
    expect(WATCHDOG_LIMITS.map((l) => l.key).sort()).toEqual(Object.keys(fields).sort());
  });

  it('says each limit in two short sentences, with the steps the core takes', () => {
    const body = Object.fromEntries(WATCHDOG_LIMITS.map((l) => [l.key, l.body]));
    for (const l of WATCHDOG_LIMITS) expect(l.body.split(/(?<=\.)\s+/), l.key).toHaveLength(2);
    // watchdog.rs: flagged at the limit, interrupted at twice the limit, stopped one limit later.
    expect(body.quiet_secs).toBe('Codly marks a turn as stalled after this long with no output. At twice this time it interrupts the turn, and when the agent stays silent this long once more, it stops the agent.');
    expect(body.quiet_tool_secs).toContain('the same three steps');
    // An idle worker is only marked.
    expect(body.idle_secs).toContain('It does nothing else');
  });

  it('says once, for all the limits, what follows an interrupt and a stop', () => {
    expect(WATCHDOG_DETAIL).toContain('asks the agent to continue, once');
    expect(WATCHDOG_DETAIL).toContain('A Lead is only interrupted: send your message again.');
    expect(WATCHDOG_DETAIL).toContain('from its third start on it is blocked for you instead');
    for (const l of WATCHDOG_LIMITS) {
      expect(l.body, l.key).not.toContain('third start');
      expect(l.body, l.key).not.toContain('Lead');
    }
  });

  it('offers Off first, then longer and longer limits, the core’s default among them', () => {
    const defaults = store.settings.watchdog;
    for (const l of WATCHDOG_LIMITS) {
      expect(l.choices[0], l.key).toBe(0);
      expect(l.choices, l.key).toContain(defaults[l.key]);
      expect([...l.choices].sort((a, b) => a - b), l.key).toEqual(l.choices);
    }
  });

  it('shows a saved value that is not one of its choices', () => {
    expect(limitChoices([0, 300, 600], 450)).toEqual([0, 300, 450, 600]);
    expect(limitChoices([0, 300, 600], 600)).toEqual([0, 300, 600]);
    expect(limitChoices([0, 300, 600], 86400)).toEqual([0, 300, 600, 86400]);
  });

  it('names a limit in the unit a person reads', () => {
    expect([0, 45, 60, 600, 3600, 5400, 14400].map(limitLabel)).toEqual(['Off', '45 s', '1 min', '10 min', '1 h', '1 h 30 min', '4 h']);
  });

  it('changes one limit and leaves the others', () => {
    const w: WatchdogSettings = { quiet_secs: 600, quiet_tool_secs: 3600, idle_secs: 120 };
    expect(withLimit(w, 'idle_secs', 0)).toEqual({ quiet_secs: 600, quiet_tool_secs: 3600, idle_secs: 0 });
    expect(w.idle_secs).toBe(120);
  });
});
