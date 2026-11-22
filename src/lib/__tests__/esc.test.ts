import { afterEach, describe, expect, it } from 'vitest';
import { escStep, type EscState } from '../state/esc';
import { ui } from '../state/ui.svelte';

const home: EscState = { repeat: false, artifactOpen: false, selection: false, mode: 'blend', room: 'home', popupOpen: false, inOffice: false, findOpen: false, readingClosed: false };

describe('Esc outside a text field', () => {
  it('closes what is open first, one step per press, and only then stops the turn', () => {
    expect(escStep({ ...home, artifactOpen: true, selection: true, mode: 'office' })).toBe('close-artifact');
    expect(escStep({ ...home, selection: true, mode: 'office' })).toBe('deselect');
    expect(escStep({ ...home, mode: 'office' })).toBe('leave-office');
    expect(escStep({ ...home, mode: 'map' })).toBe('leave-office');
    expect(escStep(home)).toBe('stop');
    expect(escStep({ ...home, mode: 'work' })).toBe('stop');
  });

  it('a held key closes but never stops the turn', () => {
    expect(escStep({ ...home, repeat: true, selection: true })).toBe('deselect');
    expect(escStep({ ...home, repeat: true, findOpen: true })).toBe('close-find');
    expect(escStep({ ...home, repeat: true })).toBeNull();
  });

  it('with the find bar open it closes the bar, and the running turn goes on', () => {
    expect(escStep({ ...home, findOpen: true })).toBe('close-find');
    expect(escStep({ ...home, findOpen: true, mode: 'work' })).toBe('close-find');
    // The bar closes before a closed request is left: one step per press.
    expect(escStep({ ...home, findOpen: true, readingClosed: true })).toBe('close-find');
    // What is open on top of the conversation still closes first.
    expect(escStep({ ...home, findOpen: true, artifactOpen: true })).toBe('close-artifact');
    expect(escStep({ ...home, findOpen: true, selection: true })).toBe('deselect');
  });

  it('reading a closed request, it goes back to your requests and stops nothing', () => {
    expect(escStep({ ...home, readingClosed: true })).toBe('leave-closed');
    expect(escStep({ ...home, readingClosed: true, repeat: true })).toBe('leave-closed');
  });

  it('the find bar and a closed request are left alone where Esc belongs to something else', () => {
    // The full office hides the conversation: Esc leaves the office first.
    expect(escStep({ ...home, findOpen: true, mode: 'office' })).toBe('leave-office');
    expect(escStep({ ...home, findOpen: true, popupOpen: true })).toBeNull();
    expect(escStep({ ...home, findOpen: true, inOffice: true })).toBeNull();
    expect(escStep({ ...home, findOpen: true, room: 'plan' })).toBeNull();
    expect(escStep({ ...home, readingClosed: true, popupOpen: true })).toBeNull();
  });

  it('stops only on Home, with no popup open, and not from inside the office', () => {
    expect(escStep({ ...home, room: 'plan' })).toBeNull();
    expect(escStep({ ...home, room: 'terminal' })).toBeNull();
    expect(escStep({ ...home, popupOpen: true })).toBeNull();
    expect(escStep({ ...home, inOffice: true })).toBeNull();
  });
});

describe('⌘F', () => {
  afterEach(() => {
    ui.mode = 'blend';
    ui.go('home');
  });

  it('finds where a conversation is on screen: Home, but not the full office or the map', () => {
    ui.go('home');
    expect(ui.canFind).toBe(true);
    ui.toggleMode('work');
    expect(ui.canFind).toBe(true);
    ui.toggleMode('office');
    expect(ui.canFind).toBe(false);
    ui.toggleMode('map');
    expect(ui.canFind).toBe(false);
    ui.toggleMode('map'); // back to the blend
    expect(ui.canFind).toBe(true);
    ui.go('plan');
    expect(ui.canFind).toBe(false);
  });
});
