import { describe, expect, it } from 'vitest';
import { DEFAULT_SETTINGS } from '../state/store.svelte';
import { NOTIFY_KINDS, NOTIFY_WHEN, withSwitch } from '../util/notifications';

describe('Settings → Notifications', () => {
  it('every setting has one switch, and every switch a setting', () => {
    const keys = [NOTIFY_WHEN, ...NOTIFY_KINDS].map((s) => s.key);
    expect([...keys].sort()).toEqual(Object.keys(DEFAULT_SETTINGS.notifications).sort());
    expect(new Set(keys).size).toBe(keys.length);
  });

  it('everything is on at first, in the background only', () => {
    expect(Object.values(DEFAULT_SETTINGS.notifications).every(Boolean)).toBe(true);
  });

  it('a switch changes its own setting and no other', () => {
    const all = DEFAULT_SETTINGS.notifications;
    for (const { key } of [NOTIFY_WHEN, ...NOTIFY_KINDS]) {
      const next = withSwitch(all, key, false);
      expect(next).toEqual({ ...all, [key]: false });
      expect(withSwitch(next, key, true)).toEqual(all);
    }
    expect(all.budget).toBe(true); // the settings given are not changed
  });
});
