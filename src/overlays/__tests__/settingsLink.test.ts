import { afterEach, describe, expect, it } from 'vitest';
import { ui } from '../../lib/state/ui.svelte';
import { followLinks, openSettings } from '../settingsLink';
import { safetyLink, type SettingsTarget } from '../settingsPages';

afterEach(() => {
  ui.settingsOpen = false;
  ui.settingsTab = null;
});

describe('opening Settings at a place', () => {
  it('opens a closed sheet there', () => {
    openSettings(safetyLink('budgets'));
    expect([ui.settingsOpen, ui.settingsTab]).toEqual([true, 'safety/budgets']);
  });

  it('moves the open sheet there, and leaves nothing for the next time Settings opens', () => {
    ui.settingsOpen = true;
    const went: SettingsTarget[] = [];
    const stop = followLinks((to) => went.push(to));
    openSettings(safetyLink('watchdog'));
    openSettings('team');
    stop();
    expect(went).toEqual([
      { tab: 'safety', page: 'limits', section: 'watchdog' },
      { tab: 'team', page: null, section: null },
    ]);
    expect([ui.settingsOpen, ui.settingsTab]).toEqual([true, null]);
  });

  it('opens on the link again once the sheet is gone', () => {
    const stop = followLinks(() => {
      throw new Error('the closed sheet was asked to move');
    });
    stop();
    openSettings(safetyLink('trust'));
    expect(ui.settingsTab).toBe('safety/trust');
  });

  it('a sheet that is gone does not end the newer sheet’s links', () => {
    const went: SettingsTarget[] = [];
    const old = followLinks(() => {});
    const now = followLinks((to) => went.push(to));
    old();
    openSettings('safety');
    now();
    expect(went).toEqual([{ tab: 'safety', page: 'approvals', section: null }]);
  });
});
