// The room library: rooms you saved (walls, doors and windows, floor, paint and furniture,
// relative to the room's corner) to put down again, in this office or another. Player-wide, and
// for now kept in this machine's localStorage (later, shared through the web app).

import type { Blueprint } from './tools';

export interface SavedRoom {
  id: string;
  name: string;
  at: number;
  bp: Blueprint;
}

const KEY = 'codly.build.rooms';

export interface Shelf {
  getItem(k: string): string | null;
  setItem(k: string, v: string): void;
}

const local = (): Shelf | null => {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
};

export function savedRooms(s: Shelf | null = local()): SavedRoom[] {
  try {
    const list = JSON.parse(s?.getItem(KEY) ?? '[]');
    return Array.isArray(list) ? list.filter((r) => r && r.bp && Array.isArray(r.bp.tiles)) : [];
  } catch {
    return [];
  }
}

function write(list: SavedRoom[], s: Shelf | null) {
  try {
    s?.setItem(KEY, JSON.stringify(list));
  } catch {
    // Full or blocked: the library just doesn't keep it.
  }
}

/** Save a room, newest first (placing it later gives everything new ids). */
export function saveRoom(name: string, bp: Blueprint, s: Shelf | null = local()): SavedRoom {
  const room: SavedRoom = { id: `room-${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`, name: name.trim().slice(0, 40) || 'Room', at: Date.now(), bp: structuredClone(bp) };
  write([room, ...savedRooms(s)].slice(0, 50), s);
  return room;
}

export function forgetRoom(id: string, s: Shelf | null = local()) {
  write(savedRooms(s).filter((r) => r.id !== id), s);
}
