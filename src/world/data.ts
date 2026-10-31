// The neighbourhood's data: the lots commands (api.lots, lotOf, buyLot, sellLot, moveOffice).
//
// TEMPORARY: the core's lots registry and its browser mock are being written in parallel. Until
// they land, the browser demo (and only it: never inside the Tauri app) answers here with a
// small local neighbourhood: the open repo's Studio, a Garage for docs-site, an empty lot of
// yours and a few for sale. Delete `Local` once `lots` answers in mock.ts.

import { api, isTauriApp } from '../lib/api';
import type { Lot } from '../lib/gen/Lot';
import type { LotSlot } from '../lib/gen/LotSlot';
import type { Office } from '../lib/gen/Office';
import type { Wallet } from '../lib/gen/Wallet';
import { landPrice, readyMadePrice, templateLot } from './price';

export interface Here {
  /** The open repo. */
  workspace: { id: string; name: string } | null;
  /** Its crew, the Lead aside. */
  crew: number;
  offices: Office[];
}

const missing = (e: unknown) => !isTauriApp() && /unknown command/i.test(String(e));

class Local {
  slots: LotSlot[];
  spent = 0;

  constructor(here: Here) {
    const ws = here.workspace;
    const docs = here.offices.find((o) => /docs/.test(o.name)) ?? here.offices[0];
    const slot = (id: string, x: number, z: number, size: LotSlot['size'], extra: Partial<LotSlot> = {}): LotSlot => ({
      id, x, z, size, owned: false, workspace_id: null, name: null, crew: 0, price: landPrice(size), template: null, gift: false, sells_for: 0, ...extra,
    });
    this.slots = [
      slot('lot-1', 0, 0, 'large', ws ? { owned: true, workspace_id: ws.id, name: ws.name, crew: here.crew, template: 'studio', gift: true } : {}),
      slot('lot-2', 1, 0, 'small', docs ? { owned: true, workspace_id: docs.id, name: docs.name, crew: docs.crew, template: 'garage', gift: true } : {}),
      slot('lot-3', 2, 0, 'small', { owned: true, sells_for: Math.floor(landPrice('small') * 0.8) }),
      slot('lot-4', 3, 0, 'medium'),
      slot('lot-5', 0, 1, 'small'),
      slot('lot-6', 1, 1, 'large'),
      slot('lot-7', 2, 1, 'medium'),
      slot('lot-8', 3, 1, 'small'),
    ];
  }

  private get(id: string): LotSlot {
    const s = this.slots.find((x) => x.id === id);
    if (!s) throw new Error('no such lot');
    return s;
  }

  private put(s: LotSlot) {
    this.slots = this.slots.map((x) => (x.id === s.id ? s : x));
  }

  buy(id: string, template: string | null, balance: number) {
    const s = this.get(id);
    if (s.owned) throw new Error('that lot is already yours');
    const price = readyMadePrice(s.size, template);
    if (price == null) throw new Error(`a ${template} doesn't fit on that lot`);
    if (price > balance) throw new Error(`not enough coins yet (${balance} of ${price})`);
    this.spent += price;
    // What it sells back for: 80% of the land, half of what is built (a rough guess; the core knows).
    const sells = Math.floor(s.price * 0.8 + (price - s.price) * 0.5);
    this.put({ ...s, owned: true, template, gift: false, sells_for: sells });
  }

  sell(id: string) {
    const s = this.get(id);
    if (!s.owned) throw new Error('not yours');
    if (s.workspace_id) throw new Error('a repo lives there: move its office first');
    this.spent -= s.sells_for;
    this.put({ ...s, owned: false, template: null, gift: false, sells_for: 0 });
  }

  move(ws: string, to: string, here: Here) {
    const target = this.get(to);
    if (!target.owned) throw new Error('not yours');
    if (target.workspace_id) throw new Error('another office is there');
    const from = this.slots.find((x) => x.workspace_id === ws);
    const office = here.offices.find((o) => o.id === ws);
    const name = from?.name ?? (ws === here.workspace?.id ? here.workspace.name : office?.name) ?? 'office';
    const crew = from?.crew ?? (ws === here.workspace?.id ? here.crew : (office?.crew ?? 0));
    // The office (its walls and furniture) moves with the repo; the old lot is left empty.
    if (from) this.put({ ...from, workspace_id: null, name: null, crew: 0, template: null, sells_for: from.gift ? 0 : Math.floor(from.price * 0.8) });
    this.put({ ...this.get(to), workspace_id: ws, name, crew, template: from?.template ?? target.template ?? 'garage' });
  }
}

let local: Local | null = null;
const fallback = (here: Here) => (local ??= new Local(here));

export const neighbourhood = {
  async lots(here: Here): Promise<LotSlot[]> {
    try {
      return await api.lots();
    } catch (e) {
      if (!missing(e)) throw e;
      return fallback(here).slots;
    }
  },
  async lotOf(slot: LotSlot, here: Here): Promise<Lot | null> {
    try {
      return await api.lotOf(slot.id);
    } catch (e) {
      if (!missing(e)) throw e;
      if (slot.workspace_id && slot.workspace_id === here.workspace?.id) return api.officeLot();
      return slot.template ? templateLot(slot.template) : null;
    }
  },
  async wallet(): Promise<Wallet> {
    const w = await api.wallet();
    if (!local) return w;
    return { ...w, spent: w.spent + local.spent, balance: w.balance - local.spent };
  },
  async buy(slot: LotSlot, template: string | null, here: Here): Promise<Wallet> {
    try {
      return await api.buyLot(slot.id, template);
    } catch (e) {
      if (!missing(e)) throw e;
      fallback(here).buy(slot.id, template, (await this.wallet()).balance);
      return this.wallet();
    }
  },
  async sell(slot: LotSlot, here: Here): Promise<Wallet> {
    try {
      return await api.sellLot(slot.id);
    } catch (e) {
      if (!missing(e)) throw e;
      fallback(here).sell(slot.id);
      return this.wallet();
    }
  },
  async move(workspaceId: string, slot: LotSlot, here: Here): Promise<LotSlot[]> {
    try {
      return await api.moveOffice(workspaceId, slot.id);
    } catch (e) {
      if (!missing(e)) throw e;
      fallback(here).move(workspaceId, slot.id, here);
      return fallback(here).slots;
    }
  },
};
