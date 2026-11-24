// Tab inside a dialog (modal.ts): it wraps at the ends so the focus stays inside, and a field
// that used the key itself keeps it (the IDE's editor indents with Tab; before, the focus then
// jumped to "Close the IDE" whenever the editor was the last stop).
import { describe, expect, it } from 'vitest';
import { tabWrap } from '../modal';

const key = (shiftKey = false, defaultPrevented = false) => ({ shiftKey, defaultPrevented });
const node = 'dialog';
const stops = ['close', 'tab', 'editor'];

describe('Tab inside a dialog', () => {
  it('wraps from the last stop to the first, and back with Shift', () => {
    expect(tabWrap(key(), 'editor', stops, node)).toBe('close');
    expect(tabWrap(key(true), 'close', stops, node)).toBe('editor');
  });

  it('leaves the move to the browser between the ends', () => {
    expect(tabWrap(key(), 'close', stops, node)).toBeNull();
    expect(tabWrap(key(true), 'editor', stops, node)).toBeNull();
  });

  it('takes the focus in from the dialog itself, or keeps it there when nothing can take it', () => {
    expect(tabWrap(key(), node, stops, node)).toBe('close');
    expect(tabWrap(key(), node, [], node)).toBe(node);
  });

  it('lets a field that used the key keep it: the focus stays where it is', () => {
    expect(tabWrap(key(false, true), 'editor', stops, node)).toBeNull();
    expect(tabWrap(key(true, true), 'close', stops, node)).toBeNull();
  });
});
