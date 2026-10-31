<!--
  Build mode: walls, doors and windows, floors and paint, as in a life sim (PROGRESSION.md §4).
  A toolbar of tools (tools.ts edits the lot), each previewing what it would change and its price
  live (cost.ts, the shared table); an edit shows at once and is saved through the core a moment
  later (when the core refuses it, the office goes back to what was saved). The checklist keeps
  what the work needs within reach of the front door, storage holds what no longer fit, and the
  room library (this machine's localStorage for now) keeps rooms to put down again.
-->
<script lang="ts" module>
  import type { Lot } from '../../lib/gen/Lot';
  import type { Wallet } from '../../lib/gen/Wallet';

  /** What saving a lot gives back: the lot as stored, the wallet after and what it cost (the demo
   *  harness, with no core, gives back just the lot). */
  export type LotSave = { lot: Lot; wallet?: Wallet | null; cost?: number | null };
</script>

<script lang="ts">
  import { untrack } from 'svelte';
  import type { DoorKind } from '../../lib/gen/DoorKind';
  import type { Placed } from '../../lib/gen/Placed';
  import type { Shop } from '../../lib/gen/Shop';
  import type { WallKind } from '../../lib/gen/WallKind';
  import type { BuildPointer, BuildTool, OfficeEngine } from '../engine';
  import { BY_ID, FOR_SALE, withBought } from '../buy';
  import { officeLayout, type OfficeLayout } from '../layout';
  import { sound } from '../sound';
  import { checklist, guard, type Check } from './checklist';
  import { boughtCost, costLabel, costOf } from './cost';
  import { BuildLayer, type Shape, type Tone } from './gfx';
  import { tidy, type BLot } from './grid';
  import { History } from './history';
  import { forgetRoom, saveRoom, savedRooms, type SavedRoom } from './library';
  import { COLOURS, FLOORS, PAPERS, floorLabel, paintLook } from './paints';
  import { openingOn, tileUnder, wallUnder, type WallHit } from './pick';
  import { diffShapes } from './preview';
  import {
    blueprint,
    drawRoom,
    drawWall,
    failed,
    fromStorage,
    knockDown,
    moveRoom,
    paintFloor,
    paintRoom,
    paintSide,
    pieceTiles,
    placeBlueprint,
    placeOpening,
    pushPull,
    removeOpening,
    removeRoom,
    roomBox,
    roomTiles,
    snapPiece,
    snapRect,
    snapWall,
    storageBlock,
    stretchAt,
    floorGrid,
    rectsOf,
    WINDOW_WIDTHS,
    type Blueprint,
    type Outcome,
    type Side,
  } from './tools';
  import { coins } from '../../lib/util/format';

  interface Props {
    engine: OfficeEngine;
    lot: Lot;
    shop: Shop | null;
    insets: { top: number; right: number; bottom: number; left: number };
    onsave: (lot: Lot) => Promise<LotSave>;
    onplace?: (piece: Placed) => void;
    onsell?: (uid: string) => void;
    /** The eyedropper picked a piece Buy sells: carry a new one of it there. */
    oncopy?: (item: string) => void;
    /** Go to the Buy tab (none: no Buy here). */
    onswitch?: () => void;
    open?: boolean;
  }
  let { engine, lot, shop, insets, onsave, onplace, onsell, oncopy, onswitch, open = $bindable(false) }: Props = $props();

  type ToolId = 'room' | 'wall' | 'door' | 'window' | 'floor' | 'paint' | 'push' | 'move' | 'hammer' | 'dropper' | 'library' | 'storage';
  const TOOLS: { id: ToolId; icon: string; label: string; hint: string }[] = [
    { id: 'room', icon: '▭', label: 'Room', hint: 'Drag a rectangle: four walls and a floor' },
    { id: 'wall', icon: '▬', label: 'Wall', hint: 'Drag along the grid: 1 m steps, 0.5 m with Alt' },
    { id: 'door', icon: '🚪', label: 'Door', hint: 'Click a wall to put a door in it (or change one)' },
    { id: 'window', icon: '🪟', label: 'Window', hint: 'Click an outside wall' },
    { id: 'floor', icon: '▦', label: 'Floor', hint: 'Click a room, or drag over tiles' },
    { id: 'paint', icon: '🎨', label: 'Paint', hint: 'Click a wall’s side, or a room for all its walls' },
    { id: 'push', icon: '⇔', label: 'Push/pull', hint: 'Drag a room’s arrow to grow or shrink it' },
    { id: 'move', icon: '✥', label: 'Move room', hint: 'Click a room to pick it up, R turns it, click to put it down' },
    { id: 'hammer', icon: '🔨', label: 'Sledgehammer', hint: 'Click a wall, door, window or room to knock it down' },
    { id: 'dropper', icon: '💧', label: 'Eyedropper', hint: 'Click a floor, wall, door or piece to copy it' },
    { id: 'library', icon: '📚', label: 'Library', hint: 'Save a room, or put a saved one down' },
    { id: 'storage', icon: '📦', label: 'Storage', hint: 'What didn’t fit: put it back in a room' },
  ];
  const WALL_KINDS: { id: WallKind; label: string }[] = [
    { id: 'solid', label: 'Solid' },
    { id: 'glass', label: 'Glass' },
    { id: 'half', label: 'Half' },
    { id: 'exterior', label: 'Outside' },
  ];
  const DOORS: { id: DoorKind; label: string }[] = [
    { id: 'single', label: 'Single' },
    { id: 'double', label: 'Double' },
    { id: 'glass', label: 'Glass' },
    { id: 'rollup', label: 'Roll-up' },
    { id: 'open', label: 'Doorway' },
  ];

  let tool = $state<ToolId>('room');
  let wallKind = $state<WallKind>('solid');
  let roomFloor = $state<string | null>('oak');
  let doorKind = $state<DoorKind>('single');
  let windowW = $state(1.6);
  let floorM = $state('oak');
  let floorWhole = $state(true);
  let paint = $state(COLOURS[2].id);
  let paintWhole = $state(false);
  let wallMode = $state<'up' | 'cutaway' | 'down'>('cutaway');
  /** The tool's preview: what it would do, its price, why not. */
  let pv = $state<{ text: string; cost: number | null; bad: string | null } | null>(null);
  let toast = $state<{ text: string; tone: 'ok' | 'bad' | 'info' } | null>(null);
  let checks = $state<Check[]>([]);
  let stored = $state<Placed[]>([]);
  let rooms = $state<SavedRoom[]>([]);
  let placing = $state<SavedRoom | null>(null);
  let pickSave = $state(false);
  let naming = $state<{ bp: Blueprint; name: string } | null>(null);
  let carrying = $state<Placed | null>(null);
  let moving = $state<{ k: number; c: number; r: number } | null>(null);
  let canUndo = $state(false);
  let canRedo = $state(false);
  let pending = $state(0);
  let saving = $state(false);

  /** Committed edits (undo/redo), and what the core has. */
  interface Entry {
    lot: BLot;
    bought: Placed[];
  }
  let history: History<Entry> | null = null;
  let saved: Entry = { lot: tidy(lot), bought: [] };
  const layer = new BuildLayer();
  let turns = 0;
  let last: BuildPointer | null = null;
  let drag: { at: [number, number]; side?: Side; k?: number } | null = null;
  let brush = new Set<number>();
  let pushRoom = -1;
  let outcome: Outcome | null = null;
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  let toastTimer: ReturnType<typeof setTimeout> | undefined;

  const L = (): OfficeLayout => engine.layout;
  const now = (): Entry => history?.now ?? saved;
  const balance = $derived(shop?.wallet.balance ?? null);
  const T = 0.5;

  // ───────────── open and close ─────────────

  $effect(() => {
    if (!open) return;
    untrack(start);
    return () => untrack(stop);
  });

  function start() {
    saved = { lot: tidy(lot), bought: shop?.placed ?? [] };
    history = new History(saved);
    turns = 0;
    engine.setWallMode(wallMode);
    engine.buildLayer.add(layer.group);
    engine.setBuildTool(handler);
    rooms = savedRooms();
    refresh();
  }

  function stop() {
    void flush();
    engine.setBuildTool(null);
    engine.buildLayer.remove(layer.group);
    engine.setWallMode('cutaway');
    cancel();
  }

  /** The office shows the current step: rebuilt, with its grid, checklist and storage. */
  function refresh() {
    const e = now();
    // The office's own kind of piece bought through Buy (a foosball) stands in the lot.
    engine.setLot(withBought(e.lot, e.bought));
    layer.setLot(L());
    layer.show([]);
    checks = checklist(officeLayout(withBought(e.lot, e.bought)));
    stored = e.lot.storage ?? [];
    canUndo = !!history?.canUndo;
    canRedo = !!history?.canRedo;
    pending = costOf(saved.lot, e.lot).net + boughtCost(saved.bought, e.bought);
    engine.redraw();
  }

  function say(text: string, tone: 'ok' | 'bad' | 'info' = 'info', ms = 3200) {
    toast = { text, tone };
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), ms);
  }

  // ───────────── committing and saving ─────────────

  /** Can the wallet pay for this edit, on top of what isn't saved yet? */
  function afford(o: Exclude<Outcome, { error: string }>): string | null {
    if (balance == null) return null;
    const total = costOf(saved.lot, o.lot).net + boughtCost(saved.bought, o.bought);
    return total > balance ? `Not enough coins: ${coins(total)} needed, ${coins(balance)} to spend` : null;
  }

  /** Make an edit a step (and the office show it); false when it can't be (and why is said). */
  function commit(o: Outcome | null): boolean {
    if (!o) return false;
    if (failed(o)) {
      sound.play('ui-error', { gain: 0.5 });
      say(o.error, 'bad');
      return false;
    }
    const cur = now();
    const why = guard(checklist(officeLayout(withBought(cur.lot, cur.bought))), checklist(officeLayout(withBought(o.lot, o.bought)))) ?? afford(o);
    if (why) {
      sound.play('ui-error', { gain: 0.5 });
      say(why, 'bad', 4500);
      return false;
    }
    sound.play('ui-place', { gain: 0.8 });
    history!.push({ lot: o.lot, bought: o.bought });
    if (o.note) say(o.note, 'info', 4500);
    refresh();
    schedule(JSON.stringify(o.bought) !== JSON.stringify(cur.bought));
    return true;
  }

  function undo() {
    if (!history?.undo()) return;
    sound.play('ui-rotate', { gain: 0.5 });
    refresh();
    schedule(JSON.stringify(now().bought) !== JSON.stringify(saved.bought));
  }

  function redo() {
    if (!history?.redo()) return;
    sound.play('ui-rotate', { gain: 0.5 });
    refresh();
    schedule(JSON.stringify(now().bought) !== JSON.stringify(saved.bought));
  }

  /** Save a moment after the last edit (an undo inside it costs nothing); at once when it moves
   *  or sells pieces bought through Buy, which the shop keeps. */
  function schedule(soon = false) {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => void flush(), soon ? 0 : 1500);
  }

  async function flush() {
    clearTimeout(saveTimer);
    if (saving) return schedule();
    const target = now();
    if (JSON.stringify(target.lot) === JSON.stringify(saved.lot) && JSON.stringify(target.bought) === JSON.stringify(saved.bought)) return;
    const estimate = costOf(saved.lot, target.lot).net + boughtCost(saved.bought, target.bought);
    saving = true;
    try {
      const res = await onsave(target.lot);
      const was = saved.bought;
      saved = { lot: tidy(res.lot), bought: target.bought };
      // The shop's own pieces follow: moved, sold back, or bought (a saved room's).
      const had = new Map(was.map((p) => [p.uid, p]));
      for (const p of target.bought) if (JSON.stringify(had.get(p.uid)) !== JSON.stringify(p)) onplace?.(p);
      for (const p of was) if (!target.bought.some((q) => q.uid === p.uid)) onsell?.(p.uid);
      const cost = res.cost ?? estimate;
      if (cost) say(cost > 0 ? `Built · ${coins(cost)}` : `Sold back · +${coins(-cost)}`, 'ok', 2200);
    } catch (e) {
      // Refused (the wallet can't pay, a rule the core keeps): back to what it has.
      sound.play('ui-error', { gain: 0.6 });
      history?.reset(saved);
      say(`Couldn’t build that: ${String(e).replace(/^Error: /, '')}. The office is back as it was.`, 'bad', 6000);
    } finally {
      saving = false;
      if (open) refresh();
    }
  }

  // ───────────── the tools ─────────────

  function cancel() {
    drag = null;
    brush = new Set();
    moving = null;
    placing = null;
    pickSave = false;
    carrying = null;
    outcome = null;
    pv = null;
    layer.show([]);
    engine.redraw();
  }

  function pick(id: ToolId) {
    cancel();
    tool = id;
    turns = 0;
    pushRoom = -1;
    sound.play('ui-click', { gain: 0.5 });
  }

  const heightOf = (id: string) => engine.wallHeight(id);
  const wallShape = (h: WallHit, tone: Tone): Shape => {
    const w = h.wall;
    return { kind: 'wall', x0: w.x0, z0: w.z0, x1: w.x1, z1: w.z1, h: heightOf(w.id) + 0.03, tone };
  };
  const roomShapes = (k: number, tone: 'ok' | 'bad' | 'pick'): Shape[] => rectsOf(roomTiles(L(), k), L().tiles.cols).map((r) => ({ kind: 'floor', ...r, tone }));

  /** Show an outcome: its difference, price and why not. */
  function show(o: Outcome, text: string, extra: Shape[] = []) {
    outcome = o;
    const cur = now();
    if (failed(o)) {
      pv = { text, cost: null, bad: o.error };
      layer.show(extra);
    } else {
      const cost = costOf(cur.lot, o.lot).net + boughtCost(cur.bought, o.bought);
      const why = afford(o);
      pv = { text, cost, bad: why };
      layer.show([...diffShapes(cur.lot, o.lot, !!why), ...extra]);
    }
    engine.redraw();
  }

  function hint(text: string, shapes: Shape[] = []) {
    outcome = null;
    pv = { text, cost: null, bad: null };
    layer.show(shapes);
    engine.redraw();
  }

  /** Push/pull's arrows around a rectangular room. */
  function arrows(k: number): { side: Side; x: number; z: number; nx: number; nz: number }[] {
    const b = roomBox(L(), k);
    if (!b.rect) return [];
    const [mx, mz] = [((b.c0 + b.c1) / 2) * T, ((b.r0 + b.r1) / 2) * T];
    return [
      { side: 'n', x: mx, z: b.r0 * T - 0.45, nx: 0, nz: -1 },
      { side: 's', x: mx, z: b.r1 * T + 0.45, nx: 0, nz: 1 },
      { side: 'w', x: b.c0 * T - 0.45, z: mz, nx: -1, nz: 0 },
      { side: 'e', x: b.c1 * T + 0.45, z: mz, nx: 1, nz: 0 },
    ];
  }
  const arrowShapes = (k: number, hot?: Side): Shape[] => arrows(k).map((a) => ({ kind: 'arrow', x: a.x, z: a.z, nx: a.nx, nz: a.nz, tone: a.side === hot ? 'hot' : 'pick' }));

  function preview(p: BuildPointer, dragging: boolean) {
    const e = now();
    const lotNow = e.lot;
    const bought = e.bought;
    const here = tileUnder(L(), p.x, p.z);
    switch (tool) {
      case 'room': {
        if (!dragging || !drag) return hint(TOOLS[0].hint, [{ kind: 'floor', x: Math.round(p.x / T) * T - 0.1, z: Math.round(p.z / T) * T - 0.1, w: 0.2, d: 0.2, tone: 'pick' }]);
        const [x0, z0, x1, z1] = snapRect(drag.at, [p.x, p.z]);
        const o = drawRoom(lotNow, bought, x0, z0, x1, z1, wallKind, roomFloor);
        const box: Shape[] = failed(o) ? [{ kind: 'floor', x: x0, z: z0, w: x1 - x0, d: z1 - z0, tone: 'bad' }] : [];
        return show(o, `Room ${x1 - x0} × ${z1 - z0} m`, box);
      }
      case 'wall': {
        if (!dragging || !drag) return hint(TOOLS[1].hint);
        const s = snapWall(drag.at, [p.x, p.z], p.alt ? 0.5 : 1);
        if (!s) return hint(TOOLS[1].hint);
        const o = drawWall(lotNow, bought, ...s, wallKind);
        return show(o, `Wall ${Math.abs(s[2] - s[0]) + Math.abs(s[3] - s[1])} m`, failed(o) ? [{ kind: 'wall', x0: s[0], z0: s[1], x1: s[2], z1: s[3], h: 0.9, tone: 'bad' }] : []);
      }
      case 'door':
      case 'window': {
        const hit = wallUnder(L(), p, heightOf);
        if (!hit) return hint(tool === 'door' ? TOOLS[2].hint : TOOLS[3].hint);
        const o = placeOpening(lotNow, bought, hit.wall.id, hit.x, hit.z, tool === 'door' ? { door: doorKind } : { window: windowW });
        const label = tool === 'door' ? `${DOORS.find((d) => d.id === doorKind)!.label} door` : `Window ${windowW} m`;
        return show(o, label, failed(o) ? [wallShape(hit, 'bad')] : []);
      }
      case 'floor': {
        if (here.tile < 0) return hint(TOOLS[4].hint);
        if (dragging || !floorWhole) {
          if (dragging) brush.add(here.tile);
          const tiles = dragging ? [...brush] : [here.tile];
          return show(paintFloor(lotNow, bought, tiles, floorM), `${floorLabel(floorM)} · ${tiles.length * 0.25} m²`);
        }
        if (here.room < 0) return hint('Click inside a room (or turn off Whole room to paint tiles)');
        const tiles = roomTiles(L(), here.room);
        return show(paintFloor(lotNow, bought, tiles, floorM), `${floorLabel(floorM)} in the ${L().rooms[here.room].name.toLowerCase()}`);
      }
      case 'paint': {
        if (paintWhole) {
          if (here.room < 0) return hint('Click inside a room');
          return show(paintRoom(lotNow, bought, L(), here.room, paint), `${paintLook(paint).label} around the ${L().rooms[here.room].name.toLowerCase()}`, roomShapes(here.room, 'pick'));
        }
        const hit = wallUnder(L(), p, heightOf);
        if (!hit) return hint(TOOLS[5].hint);
        return show(paintSide(lotNow, bought, hit.wall.id, hit.side, paint), `${paintLook(paint).label} on this side`, [wallShape(hit, 'hot')]);
      }
      case 'push': {
        if (drag?.side && drag.k != null) {
          const a = arrows(drag.k).find((x) => x.side === drag!.side)!;
          const steps = Math.round(((p.x - drag.at[0]) * a.nx + (p.z - drag.at[1]) * a.nz) / T);
          if (!steps) return hint('Drag out to grow it, in to shrink it', arrowShapes(drag.k, drag.side));
          return show(pushPull(lotNow, bought, L(), drag.k, drag.side, steps), `${steps > 0 ? 'Grow' : 'Shrink'} it ${Math.abs(steps) * T} m`, arrowShapes(drag.k, drag.side));
        }
        // The room under the cursor, unless it is on one of the last room's arrows.
        const onArrow = pushRoom >= 0 && pushRoom < L().rooms.length && arrows(pushRoom).some((a) => Math.hypot(a.x - p.x, a.z - p.z) < 0.6);
        if (here.room >= 0 && !onArrow) pushRoom = here.room;
        if (pushRoom < 0 || pushRoom >= L().rooms.length) return hint(TOOLS[6].hint);
        const near = arrows(pushRoom).find((a) => Math.hypot(a.x - p.x, a.z - p.z) < 0.6);
        if (!roomBox(L(), pushRoom).rect) return hint('Push/pull works on rectangular rooms', roomShapes(pushRoom, 'bad'));
        return hint(near ? 'Drag the arrow' : TOOLS[6].hint, arrowShapes(pushRoom, near?.side));
      }
      case 'move': {
        if (!moving) {
          if (here.room < 0) return hint(TOOLS[7].hint);
          return hint(`Pick up the ${L().rooms[here.room].name.toLowerCase()}`, roomShapes(here.room, 'pick'));
        }
        if (here.tile < 0) return hint('Over the lot');
        const c = here.tile % L().tiles.cols;
        const r = Math.floor(here.tile / L().tiles.cols);
        const o = moveRoom(lotNow, bought, L(), moving.k, c - moving.c, r - moving.r, turns);
        // Where it would land, in red when it can't (unturned: near enough to see why).
        const cols = L().tiles.cols;
        const ghost = failed(o) ? rectsOf(roomTiles(L(), moving.k).map((t) => t + (r - moving.r) * cols + (c - moving.c)), cols).map((x): Shape => ({ kind: 'floor', ...x, tone: 'bad' })) : [];
        return show(o, 'Put it down here · R turns it', ghost);
      }
      case 'hammer': {
        const hit = wallUnder(L(), p, heightOf);
        if (hit) {
          const op = openingOn([...lotNow.doors, ...lotNow.windows], hit.wall.id, hit.at);
          if (op) return show(removeOpening(lotNow, bought, op.id), lotNow.doors.some((d) => d.id === op.id) ? 'Knock out this door' : 'Knock out this window');
          const s = stretchAt(lotNow, hit.wall.id, hit.x, hit.z);
          return show(knockDown(lotNow, bought, hit.wall.id, hit.x, hit.z), `Knock down ${s ? (s[1] - s[0]) * T : 0} m of wall`);
        }
        if (here.room < 0) return hint(TOOLS[8].hint);
        return show(removeRoom(lotNow, bought, L(), here.room), `Knock down the ${L().rooms[here.room].name.toLowerCase()}`, roomShapes(here.room, 'bad'));
      }
      case 'dropper': {
        const s = sample(p, false);
        return hint(s ? `Copy: ${s}` : TOOLS[9].hint);
      }
      case 'library': {
        if (pickSave) {
          if (here.room < 0) return hint('Click the room to save');
          return hint(`Save the ${L().rooms[here.room].name.toLowerCase()}`, roomShapes(here.room, 'pick'));
        }
        if (!placing) return hint(TOOLS[10].hint);
        if (here.tile < 0) return hint('Over the lot');
        const bp = placing.bp;
        const [w, d] = turns % 2 ? [bp.d, bp.w] : [bp.w, bp.d];
        const c = (here.tile % L().tiles.cols) - Math.floor(w / 2);
        const r = Math.floor(here.tile / L().tiles.cols) - Math.floor(d / 2);
        return show(placeBlueprint(lotNow, bought, L(), bp, c, r, turns), `${placing.name} · R turns it`);
      }
      case 'storage': {
        if (!carrying) return hint(stored.length ? 'Pick a piece below to put it back' : 'Nothing in storage');
        const q = { ...carrying, ...snapPiece(carrying.item, p.x, p.z, carrying.rot + turns * (Math.PI / 2)), y: 0 };
        const why = storageBlock(L(), lotNow, q);
        const cols = L().tiles.cols;
        const foot: Shape[] = rectsOf(pieceTiles(lotNow, q).filter((t) => t >= 0), cols).map((r) => ({ kind: 'floor', ...r, tone: why ? 'bad' : 'ok' }));
        const label = `${BY_ID.get(q.item)?.label ?? 'Piece'} · R turns it`;
        if (why) return show({ error: why }, label, foot);
        return show(fromStorage(lotNow, bought, q.uid, q.x, q.z, q.rot), label, foot);
      }
    }
  }

  /** The eyedropper: what's under the cursor, and (`take`) switch to the tool that makes it. */
  function sample(p: BuildPointer, take: boolean): string | null {
    const e = now();
    const uid = engine.pieceAt(p.clientX, p.clientY);
    const piece = uid && [...e.lot.furniture, ...e.bought].find((x) => x.uid === uid);
    if (piece) {
      const f = BY_ID.get(piece.item);
      const forSale = FOR_SALE.some((x) => x.id === piece.item);
      if (take) {
        if (forSale && oncopy) oncopy(piece.item);
        else say(`The ${f?.label.toLowerCase() ?? 'piece'} is one of the office’s own: it isn’t sold`, 'info');
      }
      return f?.label ?? 'a piece';
    }
    const hit = wallUnder(L(), p, heightOf);
    if (hit) {
      const op = openingOn([...e.lot.doors, ...e.lot.windows], hit.wall.id, hit.at);
      const door = op && e.lot.doors.find((d) => d.id === op.id);
      if (door) {
        if (take) (pick('door'), (doorKind = door.kind));
        return `${DOORS.find((d) => d.id === door.kind)?.label ?? door.kind} door`;
      }
      if (op) {
        if (take) (pick('window'), (windowW = WINDOW_WIDTHS.reduce((a, b) => (Math.abs(b - op.width) < Math.abs(a - op.width) ? b : a))));
        return 'a window';
      }
      const pt = e.lot.paint.find((x) => x.wall === hit.wall.id && x.side === hit.side);
      if (pt) {
        if (take) (pick('paint'), (paint = pt.paint), (paintWhole = false));
        return paintLook(pt.paint).label;
      }
      if (take) (pick('wall'), (wallKind = hit.wall.kind));
      return `${WALL_KINDS.find((k) => k.id === hit.wall.kind)?.label ?? ''} wall`;
    }
    const here = tileUnder(L(), p.x, p.z);
    if (here.tile < 0) return null;
    const m = floorGrid(e.lot)[here.tile];
    if (take) (pick('floor'), (floorM = m));
    return floorLabel(m);
  }

  function click(p: BuildPointer) {
    const here = tileUnder(L(), p.x, p.z);
    switch (tool) {
      case 'move':
        if (!moving) {
          if (here.room < 0) return;
          sound.play('ui-click', { gain: 0.6 });
          moving = { k: here.room, c: here.tile % L().tiles.cols, r: Math.floor(here.tile / L().tiles.cols) };
          turns = 0;
          return preview(p, false);
        }
        if (commit(outcome)) moving = null;
        return;
      case 'dropper':
        sample(p, true);
        return;
      case 'library':
        if (pickSave) {
          if (here.room < 0) return;
          const e = now();
          naming = { bp: blueprint(e.lot, e.bought, L(), here.room), name: L().rooms[here.room].name };
          pickSave = false;
          return hint('Name it and save');
        }
        if (placing) commit(outcome);
        return;
      case 'storage':
        if (commit(outcome)) (carrying = null), (turns = 0);
        return;
      default:
        commit(outcome);
    }
  }

  // Pointer events from the office, one preview per frame.
  let queued: { p: BuildPointer; dragging: boolean } | null = null;
  const handler: BuildTool = {
    down(p) {
      last = p;
      brush = new Set();
      drag = { at: [p.x, p.z] };
      if (tool === 'push' && pushRoom >= 0) {
        const a = arrows(pushRoom).find((x) => Math.hypot(x.x - p.x, x.z - p.z) < 0.6);
        if (a) drag = { at: [p.x, p.z], side: a.side, k: pushRoom };
      }
      preview(p, true);
    },
    move(p, dragging) {
      last = p;
      if (!queued) requestAnimationFrame(() => {
        const q = queued;
        queued = null;
        if (q && open) preview(q.p, q.dragging);
      });
      queued = { p, dragging };
    },
    up(p) {
      last = p;
      queued = null;
      const moved = drag ? Math.hypot(p.x - drag.at[0], p.z - drag.at[1]) > 0.2 : false;
      // Drag tools commit what was dragged; the others what was clicked.
      if (tool === 'room' || tool === 'wall' || (tool === 'push' && drag?.side) || (tool === 'floor' && (moved || !floorWhole))) {
        preview(p, true);
        commit(outcome);
      } else {
        preview(p, false);
        click(p);
      }
      drag = null;
      brush = new Set();
      if (open && last) preview(last, false);
    },
    leave() {
      if (drag) return;
      pv = null;
      layer.show([]);
      engine.redraw();
    },
  };

  function turn() {
    turns = (turns + 1) % 4;
    sound.play('ui-rotate', { gain: 0.5 });
    if (last) preview(last, false);
  }

  function setWalls(m: typeof wallMode) {
    wallMode = m;
    engine.setWallMode(m);
  }

  function onkey(e: KeyboardEvent) {
    if (!open || e.defaultPrevented || (e.target as HTMLElement)?.closest?.('input, textarea, [contenteditable]')) return;
    if ((e.metaKey || e.ctrlKey) && (e.key === 'z' || e.key === 'Z')) {
      e.preventDefault();
      if (e.shiftKey) redo();
      else undo();
      return;
    }
    if ((e.metaKey || e.ctrlKey) && (e.key === 'y' || e.key === 'Y')) return void (e.preventDefault(), redo());
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if ((e.key === 'r' || e.key === 'R') && (moving || placing || carrying)) return void (e.preventDefault(), turn());
    if (e.key === 'Escape') {
      e.preventDefault();
      if (moving || placing || carrying || pickSave || naming || drag) {
        naming = null;
        cancel();
      } else close();
    }
  }

  function close() {
    sound.play('ui-close', { gain: 0.5 });
    open = false;
  }

  function saveNamed() {
    if (!naming) return;
    const r = saveRoom(naming.name, naming.bp);
    rooms = savedRooms();
    naming = null;
    say(`Saved “${r.name}” to your library`, 'ok');
  }

  const toolInfo = $derived(TOOLS.find((t) => t.id === tool)!);
</script>

<svelte:window onkeydown={onkey} />

{#if open}
  <div class="panel" style:bottom="{insets.bottom + 46}px" style:left="{insets.left + 10}px" style:right="{insets.right + 10}px" role="dialog" aria-label="Build mode">
    <header>
      <div>
        {#if onswitch}
          <span class="seg mode" role="tablist"><button role="tab" aria-selected="true" class="on">🔨 Build</button><button role="tab" aria-selected="false" onclick={onswitch}>🛋 Buy</button></span>
        {:else}<div class="kicker">Build</div>{/if}
        {#if balance != null}<b>{coins(balance)} to spend</b>{/if}
        <small>{pending ? `${costLabel(pending)} not saved yet` : saving ? 'Saving…' : 'All saved'}</small>
      </div>
      <div class="right">
        <span class="seg" role="group" aria-label="Walls">
          {#each [['up', 'Walls up'], ['cutaway', 'Cutaway'], ['down', 'Walls down']] as [m, label] (m)}
            <button class:on={wallMode === m} onclick={() => setWalls(m as typeof wallMode)}>{label}</button>
          {/each}
        </span>
        <button onclick={undo} disabled={!canUndo} title="Undo (⌘Z)">↶</button>
        <button onclick={redo} disabled={!canRedo} title="Redo (⇧⌘Z)">↷</button>
        <button class="x" aria-label="Close build mode" onclick={close}>✕</button>
      </div>
    </header>

    <div class="tools" role="toolbar" aria-label="Build tools">
      {#each TOOLS as t (t.id)}
        <button class:on={tool === t.id} onclick={() => pick(t.id)} title={t.hint} aria-pressed={tool === t.id}>
          <span class="i" aria-hidden="true">{t.icon}</span><span>{t.label}</span>
        </button>
      {/each}
    </div>

    <div class="body">
      <div class="opts">
        {#if tool === 'room' || tool === 'wall'}
          <span class="lbl">Walls</span>
          {#each WALL_KINDS as k (k.id)}<button class="chip" class:on={wallKind === k.id} onclick={() => (wallKind = k.id)}>{k.label}</button>{/each}
          {#if tool === 'room'}
            <span class="lbl">Floor</span>
            <button class="chip" class:on={roomFloor == null} onclick={() => (roomFloor = null)}>Keep</button>
            {#each FLOORS as f (f.id)}<button class="sw" class:on={roomFloor === f.id} style:background={f.swatch} title={f.label} aria-label={f.label} onclick={() => (roomFloor = f.id)}></button>{/each}
          {/if}
        {:else if tool === 'door'}
          {#each DOORS as d (d.id)}<button class="chip" class:on={doorKind === d.id} onclick={() => (doorKind = d.id)}>{d.label}</button>{/each}
        {:else if tool === 'window'}
          {#each WINDOW_WIDTHS as w (w)}<button class="chip" class:on={windowW === w} onclick={() => (windowW = w)}>{w} m</button>{/each}
        {:else if tool === 'floor'}
          <button class="chip" class:on={floorWhole} onclick={() => (floorWhole = true)}>Whole room</button>
          <button class="chip" class:on={!floorWhole} onclick={() => (floorWhole = false)}>Tiles</button>
          {#each FLOORS as f (f.id)}<button class="sw" class:on={floorM === f.id} style:background={f.swatch} title={f.label} aria-label={f.label} onclick={() => (floorM = f.id)}></button>{/each}
        {:else if tool === 'paint'}
          <button class="chip" class:on={!paintWhole} onclick={() => (paintWhole = false)}>One side</button>
          <button class="chip" class:on={paintWhole} onclick={() => (paintWhole = true)}>Whole room</button>
          {#each [...COLOURS.map((c) => c.id), ...PAPERS.map((p) => p.id)] as id (id)}
            {@const look = paintLook(id)}
            <button class="sw" class:on={paint === id} style:background={look.css} title={look.label} aria-label={look.label} onclick={() => (paint = id)}></button>
          {/each}
        {:else if tool === 'move'}
          <button class="chip" onclick={turn} disabled={!moving}>⟳ Turn <kbd>R</kbd></button>
          {#if moving}<button class="chip" onclick={cancel}>Put back <kbd>Esc</kbd></button>{/if}
        {:else if tool === 'library'}
          {#if naming}
            <input class="name" bind:value={naming.name} maxlength="40" aria-label="Room name" onkeydown={(e) => e.key === 'Enter' && saveNamed()} />
            <button class="chip on" onclick={saveNamed}>Save</button>
            <button class="chip" onclick={() => (naming = null)}>Cancel</button>
          {:else}
            <button class="chip" class:on={pickSave} onclick={() => ((placing = null), (pickSave = !pickSave))}>＋ Save a room</button>
            {#each rooms as r (r.id)}
              <span class="saved" class:on={placing?.id === r.id}>
                <button class="chip" class:on={placing?.id === r.id} onclick={() => ((pickSave = false), (turns = 0), (placing = placing?.id === r.id ? null : r))} title="Put it down ({r.bp.w * 0.5} × {r.bp.d * 0.5} m)">{r.name} · {r.bp.w * 0.5}×{r.bp.d * 0.5}</button>
                <button class="forget" aria-label="Forget {r.name}" onclick={() => (forgetRoom(r.id), (rooms = savedRooms()), placing?.id === r.id && (placing = null))}>✕</button>
              </span>
            {:else}
              <span class="muted">No saved rooms yet (kept on this machine)</span>
            {/each}
            {#if placing}<button class="chip" onclick={turn}>⟳ Turn <kbd>R</kbd></button>{/if}
          {/if}
        {:else if tool === 'storage'}
          {#each stored as p (p.uid)}
            <button class="chip" class:on={carrying?.uid === p.uid} onclick={() => ((turns = 0), (carrying = carrying?.uid === p.uid ? null : p))}>{BY_ID.get(p.item)?.label ?? p.item}</button>
          {:else}
            <span class="muted">Storage is empty</span>
          {/each}
          {#if carrying}<button class="chip" onclick={turn}>⟳ Turn <kbd>R</kbd></button>{/if}
        {:else}
          <span class="muted">{toolInfo.hint}</span>
        {/if}
      </div>

      <ul class="check" aria-label="What every office needs">
        {#each checks as c (c.need)}
          <li class:bad={!c.have || !c.reached} title={!c.have ? 'Missing' : !c.reached ? 'Not reachable from the front door' : 'In reach of the front door'}>{c.have && c.reached ? '✓' : '✕'} {c.label}</li>
        {/each}
        {#if stored.length}<li class="store"><button onclick={() => pick('storage')}>📦 {stored.length} in storage</button></li>{/if}
      </ul>
    </div>

    <div class="status" aria-live="polite">
      {#if toast}
        <span class="toast {toast.tone}">{toast.text}</span>
      {:else if pv}
        <span>{pv.text}</span>
        {#if pv.bad}<span class="bad">{pv.bad}</span>{:else if pv.cost != null}<span class="cost" class:back={pv.cost < 0}>{costLabel(pv.cost)}</span>{/if}
      {:else}
        <span class="muted">{toolInfo.hint}</span>
      {/if}
    </div>
  </div>
{/if}

<style>
  .panel { font: 500 12.5px/1.35 'Inter Variable', Inter, system-ui, sans-serif; color: #eef1f7; }
  .panel { position: absolute; z-index: 401; max-width: 1060px; margin: 0 auto; padding: 12px; border-radius: 16px; background: rgba(22, 26, 36, 0.96); box-shadow: 0 24px 60px -24px rgba(0, 0, 0, 0.8), inset 0 0 0 1px rgba(255, 255, 255, 0.08); }
  header { display: flex; justify-content: space-between; align-items: start; gap: 8px; margin-bottom: 8px; }
  .kicker { font: 700 10px/1 ui-monospace, monospace; letter-spacing: 0.08em; text-transform: uppercase; opacity: 0.6; margin-bottom: 3px; }
  small { font-size: 11px; opacity: 0.6; margin-left: 6px; }
  .right { display: flex; align-items: center; gap: 6px; }
  .right > button, .seg button { padding: 4px 10px; border: 0; border-radius: 999px; background: rgba(255, 255, 255, 0.08); color: inherit; font: inherit; cursor: pointer; }
  .right > button:disabled { opacity: 0.35; cursor: default; }
  .seg { display: inline-flex; gap: 2px; padding: 2px; border-radius: 999px; background: rgba(255, 255, 255, 0.05); }
  .seg button { background: none; }
  .mode { margin-bottom: 4px; }
  .seg button.on { background: rgba(126, 224, 161, 0.22); color: #c8f5d6; }
  .x { background: none !important; color: #9aa2b6; font-size: 15px; }
  .tools { display: flex; gap: 4px; overflow-x: auto; scrollbar-width: none; margin-bottom: 8px; }
  .tools button { flex: none; display: flex; flex-direction: column; align-items: center; gap: 1px; min-width: 68px; padding: 6px 6px 5px; border: 0; border-radius: 12px; background: rgba(255, 255, 255, 0.05); color: inherit; font: inherit; font-size: 11px; cursor: pointer; }
  .tools button:hover { background: rgba(126, 224, 161, 0.12); }
  .tools button.on { background: #7ee0a1; color: #0f2a1a; font-weight: 600; }
  .i { font-size: 17px; line-height: 1.2; }
  .body { display: flex; gap: 12px; align-items: start; }
  .opts { flex: 1; display: flex; flex-wrap: wrap; align-items: center; gap: 5px; min-height: 28px; }
  .lbl { font-size: 11px; opacity: 0.55; margin: 0 2px 0 6px; }
  .lbl:first-child { margin-left: 0; }
  .chip { padding: 4px 11px; border: 0; border-radius: 999px; background: rgba(255, 255, 255, 0.08); color: inherit; font: inherit; cursor: pointer; }
  .chip.on { background: rgba(126, 224, 161, 0.25); color: #c8f5d6; box-shadow: inset 0 0 0 1px rgba(126, 224, 161, 0.6); }
  .chip:disabled { opacity: 0.4; cursor: default; }
  .sw { width: 24px; height: 24px; padding: 0; border: 0; border-radius: 7px; cursor: pointer; box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.25); }
  .sw.on { box-shadow: 0 0 0 2px #7ee0a1; }
  .saved { display: inline-flex; align-items: center; }
  .forget { border: 0; background: none; color: #9aa2b6; cursor: pointer; font-size: 11px; }
  .name { padding: 4px 10px; border: 0; border-radius: 999px; background: rgba(255, 255, 255, 0.1); color: inherit; font: inherit; outline: none; box-shadow: inset 0 0 0 1px rgba(126, 224, 161, 0.5); }
  .muted { opacity: 0.6; }
  .check { list-style: none; margin: 0; padding: 6px 10px; display: grid; grid-template-columns: repeat(2, auto); gap: 2px 12px; border-radius: 10px; background: rgba(255, 255, 255, 0.04); font-size: 11.5px; }
  .check li { color: #9ee7b5; white-space: nowrap; }
  .check li.bad { color: #ff9b8f; }
  .check .store button { border: 0; background: none; color: #f0c27e; font: inherit; cursor: pointer; padding: 0; }
  .status { display: flex; gap: 10px; align-items: center; margin-top: 8px; min-height: 18px; font-size: 12px; }
  .cost { color: #8ff0b0; font-weight: 600; }
  .cost.back { color: #f0c27e; }
  .bad { color: #ff9b8f; }
  .toast.ok { color: #8ff0b0; }
  .toast.bad { color: #ff9b8f; }
  kbd { font: 600 10px ui-monospace, monospace; opacity: 0.6; margin-left: 3px; }
</style>
