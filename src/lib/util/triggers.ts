// Pure helpers for the Triggers panel: when a schedule next runs and how to say an interval. The core owns the real clocks
// (crates/codly-core/src/triggers.rs); this only labels them.
import type { NetworkMode } from '../gen/NetworkMode';
import type { Schedule } from '../gen/Schedule';

const MIN = 60_000;

/** The core's rule: one interval after it last fired, or was created. */
export const nextRunAt = (s: Schedule): number => (s.last_fired_at ?? s.created_at) + s.every_minutes * MIN;

/** "30 min", "1 h 30 min", "2 hours", "day". */
export function everyLabel(minutes: number): string {
  if (minutes % 1440 === 0) return minutes === 1440 ? 'day' : `${minutes / 1440} days`;
  if (minutes < 60) return `${minutes} min`;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return m ? `${h} h ${m} min` : h === 1 ? 'hour' : `${h} hours`;
}

/** "in 35 min", "in 2 h", "due now" (the loop wakes every 30 s, so a due one fires within it). */
export function inLabel(ts: number, now = Date.now()): string {
  const m = Math.round((ts - now) / MIN);
  if (m <= 0) return 'due now';
  if (m < 60) return `in ${m} min`;
  const h = Math.floor(m / 60);
  return h < 24 ? `in ${h} h${m % 60 ? ` ${m % 60} min` : ''}` : `in ${Math.floor(h / 24)} d`;
}

export const NETWORK_MODES: { id: NetworkMode; title: string; body: string }[] = [
  { id: 'off', title: 'Off', body: 'Nobody can reach your agents.' },
  { id: 'listen', title: 'Listen', body: 'Teammates can see what your team is working on. Read-only.' },
  { id: 'converse', title: 'Converse', body: 'Teammates can also message your Lead and answer its questions. They cannot start work.' },
  { id: 'open', title: 'Open', body: 'Teammates can also hand your agents work.' },
];

/** Interval choices in the picker (minutes). A saved custom value is added by the caller. */
export const INTERVALS = [15, 30, 60, 120, 240, 480, 1440];
