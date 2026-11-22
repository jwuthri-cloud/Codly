import { describe, expect, it } from 'vitest';
import { escStep, type EscState } from '../state/esc';

const home: EscState = { repeat: false, artifactOpen: false, selection: false, mode: 'blend', room: 'home', popupOpen: false, inOffice: false };

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
    expect(escStep({ ...home, repeat: true })).toBeNull();
  });

  it('stops only on Home, with no popup open, and not from inside the office', () => {
    expect(escStep({ ...home, room: 'plan' })).toBeNull();
    expect(escStep({ ...home, room: 'terminal' })).toBeNull();
    expect(escStep({ ...home, popupOpen: true })).toBeNull();
    expect(escStep({ ...home, inOffice: true })).toBeNull();
  });
});
