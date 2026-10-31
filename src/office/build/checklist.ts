// What every office needs, so the work always has somewhere to go: a front door (a door in a wall
// on the lot's edge), the Lead's desk, a server, a bookshelf and a mailbox, each reachable on
// foot from the front door. Build mode shows the list and refuses an edit that takes away the
// last of one, or cuts one off. Read from `officeLayout`: its stations and walking grid. Pure.

import type { OfficeLayout, Slot } from '../layout';
import { cellOf } from '../layout';
import { isH } from './grid';

export type Need = 'door' | 'lead' | 'server' | 'books' | 'mail';

export interface Check {
  need: Need;
  label: string;
  /** There is one. */
  have: boolean;
  /** One can be walked to from the front door. */
  reached: boolean;
}

const LABEL: Record<Need, string> = { door: 'Front door', lead: 'Lead’s desk', server: 'Server', books: 'Bookshelf', mail: 'Mailbox' };
const THING: Record<Need, string> = { door: 'a front door in an outside wall', lead: 'a desk for the Lead', server: 'a server', books: 'a bookshelf', mail: 'a mailbox' };

/** Cells reachable on foot from just inside the front door. */
export function reachable(L: OfficeLayout): Uint8Array {
  const seen = new Uint8Array(L.cols * L.rows);
  const [sx, sz] = cellOf(L, L.door.inside.x, L.door.inside.z);
  const start = sz * L.cols + sx;
  if (L.blocked[start]) return seen;
  seen[start] = 1;
  const queue = [start];
  for (let i = 0; i < queue.length; i++) {
    const c = queue[i];
    const x = c % L.cols;
    for (const n of [c - 1, c + 1, c - L.cols, c + L.cols]) {
      if (n < 0 || n >= seen.length || seen[n] || L.blocked[n] || Math.abs((n % L.cols) - x) > 1) continue;
      seen[n] = 1;
      queue.push(n);
    }
  }
  return seen;
}

export function checklist(L: OfficeLayout): Check[] {
  const lot = L.lot;
  const W = lot.size.w;
  const D = lot.size.d;
  const edge = (id: string) => {
    const w = lot.walls.find((x) => x.id === id);
    if (!w) return false;
    return isH(w) ? Math.abs(w.z0) < 1e-6 || Math.abs(w.z0 - D) < 1e-6 : Math.abs(w.x0) < 1e-6 || Math.abs(w.x0 - W) < 1e-6;
  };
  const front = lot.doors.some((d) => edge(d.wall));
  const seen = front ? reachable(L) : new Uint8Array(L.cols * L.rows);
  const ok = (slots: Slot[]) => slots.some((s) => {
    const [x, z] = cellOf(L, s.x, s.z);
    return !!seen[z * L.cols + x];
  });
  const jobs = (job: string) => L.items.filter((i) => i.job === job && i.desk == null);
  const leads = L.desks.filter((d) => d.lead);
  const mail = L.items.filter((i) => i.job === 'mail');
  return [
    { need: 'door', label: LABEL.door, have: front, reached: front },
    { need: 'lead', label: LABEL.lead, have: leads.length > 0, reached: ok(leads.map((d) => d.seat)) },
    { need: 'server', label: LABEL.server, have: jobs('server').length > 0, reached: ok(L.stations['server.rack'].slots) },
    { need: 'books', label: LABEL.books, have: jobs('books').length > 0, reached: ok(L.stations['library.shelf'].slots) },
    { need: 'mail', label: LABEL.mail, have: mail.length > 0, reached: ok(L.stations.mail.slots) },
  ];
}

/** Why an edit from `before` to `after` is refused (null: it isn't): it takes away the last of
 *  something the office needs, or cuts it off from the front door. What was already missing
 *  doesn't block anything, so you can always build your way back. */
export function guard(before: Check[], after: Check[]): string | null {
  for (const b of before) {
    const a = after.find((x) => x.need === b.need)!;
    if (b.need === 'door' && b.have && !a.have) return `Every office needs ${THING.door}.`;
    if (b.have && !a.have) return `Every office needs ${THING[b.need]}: that would take away the last one.`;
    if (b.reached && !a.reached) return b.need === 'door' ? `Every office needs ${THING.door}.` : `That would cut the ${LABEL[b.need].toLowerCase()} off from the front door.`;
  }
  return null;
}
