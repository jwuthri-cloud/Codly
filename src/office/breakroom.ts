// The break room: who plays what when agents are idle. Pool and foosball need two players, the
// PS5 takes one or two, the armchair is for one napper. Agents are matched in the order they became free:
//   1. someone waits alone at a table → join them;
//   2. another free agent has no game yet → pair up at a free table (pool or foosball);
//   3. otherwise a PS5 seat: solo against the CPU, or the second controller;
//   4. everything taken → stay at the desk.
// A tired agent (wish 'nap') takes the armchair first, when it is free.
// Furniture you bought (use.ts spots) joins in: a bookcase for a reader when the library is full,
// a coffee machine, a sofa to nap on; and half the time an idle agent picks a sofa, a TV or a
// radio over the PS5.
// Pairs are stable: a newcomer never splits a game. When real work calls a player back, the
// partner waits a few seconds (someone may join), then moves to the PS5 or back to its desk.
//
// Stateful but pure (no engine, no clock of its own): the engine calls `update` on each sync.

import { hash } from './text';
import type { Activity } from './traits';
import type { Use } from './use';

/** Two-player tables, the PS5, the solo spots (a library chair, the coffee counter, the nap
 *  armchair), and pieces of furniture. */
export type Game = 'pool' | 'foosball' | 'console' | 'library' | 'coffee' | 'nap' | 'piece';
export interface Seat {
  game: Game;
  /** Slot index at the game's station (games.pool:0 / :1 …), or the piece's spot. */
  slot: number;
  /** For a piece: its uid and what it is used for. */
  piece?: string;
  use?: Use;
}

/** A usable piece: its uid, a use and how many people it takes for that use. */
export interface Spot {
  uid: string;
  use: Use;
  n: number;
}

const TABLES: Game[] = ['pool', 'foosball'];
const SEATS: Record<Game, number> = { pool: 2, foosball: 2, console: 2, library: 2, coffee: 2, nap: 1, piece: 0 };
const WISH_USE: Partial<Record<Activity, Use>> = { library: 'read', coffee: 'coffee', nap: 'nap' };
const LOUNGING: Use[] = ['sit', 'watch', 'dance'];
/** How long a player left alone at a table waits for someone before giving up. */
export const LONELY_MS = 6000;

export interface BreakRoomOptions {
  /** Testing: send everyone to this game when it has room. */
  prefer?: Game | null;
}

export class BreakRoom {
  private seats = new Map<string, Seat>();
  /** Alone at a two-player table since (ms). */
  private alone = new Map<string, number>();

  constructor(private opts: BreakRoomOptions = {}) {}

  /** When a lonely player gives up (the engine wakes then), or Infinity. */
  nextAt(): number {
    let t = Infinity;
    for (const since of this.alone.values()) t = Math.min(t, since + LONELY_MS);
    return t;
  }

  get(agentId: string): Seat | null {
    return this.seats.get(agentId) ?? null;
  }

  private occupants(game: Game): (string | null)[] {
    const out: (string | null)[] = Array(SEATS[game]).fill(null);
    for (const [id, s] of this.seats) if (s.game === game) out[s.slot] = id;
    return out;
  }

  private free(game: Game): number {
    return this.occupants(game).indexOf(null);
  }

  /** A free spot on a piece for one of `uses` (one person per spot; sit and nap share seats). */
  private piece(spots: Spot[], uses: Use[], id: string): Seat | null {
    const taken = new Set([...this.seats.values()].filter((s) => s.game === 'piece').map((s) => `${s.piece}:${s.slot}`));
    const fit = spots.filter((s) => uses.includes(s.use));
    for (let k = 0; k < fit.length; k++) {
      const s = fit[(k + hash(id)) % fit.length];
      for (let i = 0; i < s.n; i++) if (!taken.has(`${s.uid}:${i}`)) return { game: 'piece', slot: i, piece: s.uid, use: s.use };
    }
    return null;
  }

  /** `eligible`: idle agents wanting a break, earliest-free first, each with what it would
   *  like to do (traits, needs). Returns everyone's seat. */
  update(list: (string | { id: string; prefer?: Activity })[], now: number, spots: Spot[] = []): Map<string, Seat> {
    const wants = new Map(list.map((x) => (typeof x === 'string' ? [x, 'any' as Activity] : [x.id, x.prefer ?? 'any'])));
    const eligible = [...wants.keys()];
    const want = new Set(eligible);
    const there = new Set(spots.map((s) => `${s.uid}:${s.use}`));
    for (const [id, s] of [...this.seats]) {
      // Gone (back to work), or its piece was sold or moved out of use.
      if (!want.has(id) || (s.game === 'piece' && !there.has(`${s.piece}:${s.use}`))) {
        this.seats.delete(id);
        this.alone.delete(id);
      }
    }
    // Players left alone at a table: wait a little, then give up.
    for (const game of TABLES) {
      const occ = this.occupants(game);
      const here = occ.filter((x): x is string => x != null);
      if (here.length !== 1) {
        for (const id of here) this.alone.delete(id);
        continue;
      }
      const id = here[0];
      const since = this.alone.get(id) ?? now;
      this.alone.set(id, since);
      if (now - since < LONELY_MS) continue;
      this.alone.delete(id);
      this.seats.delete(id);
      const seat = this.free('console');
      if (seat >= 0) this.seats.set(id, { game: 'console', slot: seat });
    }
    // Someone who gave up and found no PS5 seat is simply matched again later (a newcomer).
    const pending = eligible.filter((id) => !this.seats.has(id));
    const prefer = this.opts.prefer ?? null;
    const tables = prefer && prefer !== 'console' ? [prefer, ...TABLES.filter((t) => t !== prefer)] : TABLES;
    for (let i = 0; i < pending.length; i++) {
      const id = pending[i];
      if (this.seats.has(id)) continue;
      // Its own wish first when nothing forces a game (a solo spot, or straight to the couch).
      const wish = prefer ? 'any' : (wants.get(id) ?? 'any');
      const solo: Game | null = wish === 'library' ? 'library' : wish === 'coffee' ? 'coffee' : wish === 'console' ? 'console' : wish === 'nap' ? 'nap' : null;
      if (solo && this.free(solo) >= 0) {
        this.seats.set(id, { game: solo, slot: this.free(solo) });
        continue;
      }
      const wishUse = WISH_USE[wish];
      const own = wishUse ? this.piece(spots, [wishUse], id) : null;
      if (own) {
        this.seats.set(id, own);
        continue;
      }
      // Half the agents would rather lounge on furniture you bought than play.
      const lounge = !prefer && wish === 'any' && hash(id) % 2 === 0 ? this.piece(spots, LOUNGING, id) : null;
      if (lounge) {
        this.seats.set(id, lounge);
        continue;
      }
      if (prefer !== 'console') {
        // 1. Join someone waiting alone.
        const waiting = tables.find((g) => this.occupants(g).filter(Boolean).length === 1);
        if (waiting) {
          this.seats.set(id, { game: waiting, slot: this.free(waiting) });
          continue;
        }
        // 2. Pair with the next free agent at a free table.
        const partner = pending.slice(i + 1).find((p) => !this.seats.has(p) && (prefer || ['any', 'table'].includes(wants.get(p) ?? 'any')));
        const open = tables.filter((g) => this.occupants(g).every((x) => x == null));
        if (partner && open.length) {
          const game = prefer && open.includes(prefer) ? prefer : open[hash(id + partner) % open.length];
          this.seats.set(id, { game, slot: 0 });
          this.seats.set(partner, { game, slot: 1 });
          continue;
        }
      }
      // 3. A PS5 controller, else any free piece of furniture.
      const seat = this.free('console');
      const any = seat < 0 && !prefer ? this.piece(spots, LOUNGING, id) : null;
      if (seat >= 0) this.seats.set(id, { game: 'console', slot: seat });
      else if (any) this.seats.set(id, any);
      // 4. Otherwise: the desk.
    }
    return new Map(this.seats);
  }
}
