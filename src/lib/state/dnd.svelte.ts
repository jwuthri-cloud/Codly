// Pointer-event drag and drop (HTML5 DnD conflicts with Tauri's file-drop handler).
// ponytail: hit-testing via elementsFromPoint on every move; no auto-scroll near edges yet.
// Sources and targets are Svelte actions; a single <DragGhost/> renders the floating card.

import type { Action } from 'svelte/action';

export type DragKind = 'task' | 'agent';
export type DragPayload = { kind: DragKind; id: string; label: string; color?: string };

type Target = {
  accepts: (p: DragPayload) => boolean;
  onDrop: (p: DragPayload) => void;
  /** Shown on the ghost while hovering this target. */
  hint?: (p: DragPayload) => string;
};

class DragState {
  payload = $state<DragPayload | null>(null);
  x = $state(0);
  y = $state(0);
  over = $state<string | null>(null);
  overOk = $state(false);
  hint = $state('');
}

export const drag = new DragState();
const targets = new Map<string, Target>();
let suppressClick = false;

function hitTest(x: number, y: number): string | null {
  for (const el of document.elementsFromPoint(x, y)) {
    const id = (el as HTMLElement).dataset?.dropId;
    if (id && targets.has(id)) return id;
  }
  return null;
}

/** `use:dragSource={{ kind, id, label }}` — a 4 px threshold keeps clicks working. */
export const dragSource: Action<HTMLElement, DragPayload> = (node, initial) => {
  let payload = initial;
  let start: { x: number; y: number; pointer: number } | null = null;

  const move = (e: PointerEvent) => {
    if (!start) return;
    if (!drag.payload) {
      if (Math.hypot(e.clientX - start.x, e.clientY - start.y) < 4) return;
      drag.payload = payload;
      node.setPointerCapture(start.pointer);
      document.body.dataset.dragging = payload.kind;
    }
    drag.x = e.clientX;
    drag.y = e.clientY;
    const over = hitTest(e.clientX, e.clientY);
    drag.over = over;
    const t = over ? targets.get(over) : null;
    drag.overOk = !!t && t.accepts(payload);
    drag.hint = t?.hint?.(payload) ?? '';
  };

  const end = (e: PointerEvent, cancelled = false) => {
    if (!start) return;
    const p = drag.payload;
    start = null;
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', up);
    window.removeEventListener('pointercancel', cancel);
    window.removeEventListener('keydown', esc, true);
    if (!p) return;
    suppressClick = true;
    setTimeout(() => (suppressClick = false), 0);
    const over = cancelled ? null : hitTest(e.clientX, e.clientY);
    const t = over ? targets.get(over) : null;
    drag.payload = null;
    drag.over = null;
    drag.hint = '';
    delete document.body.dataset.dragging;
    if (t && t.accepts(p)) t.onDrop(p);
  };
  const up = (e: PointerEvent) => end(e);
  const cancel = (e: PointerEvent) => end(e, true);
  const esc = (e: KeyboardEvent) => {
    if (e.key === 'Escape' && drag.payload) {
      e.preventDefault();
      e.stopPropagation();
      end(new PointerEvent('pointerup', { clientX: -1, clientY: -1 }), true);
    }
  };

  const down = (e: PointerEvent) => {
    if (e.button !== 0) return;
    // Interactive children (a nested button, an input) keep their own behaviour; the source
    // itself may be a button.
    const inner = (e.target as HTMLElement).closest('button, input, textarea, select, a, [data-no-drag]');
    if (inner && inner !== node && node.contains(inner) && !(inner as HTMLElement).hasAttribute('data-drag-ok')) return;
    start = { x: e.clientX, y: e.clientY, pointer: e.pointerId };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
    window.addEventListener('pointercancel', cancel);
    window.addEventListener('keydown', esc, true);
  };
  const click = (e: MouseEvent) => {
    if (suppressClick) {
      e.stopPropagation();
      e.preventDefault();
    }
  };

  node.addEventListener('pointerdown', down);
  node.addEventListener('click', click, true);
  return {
    update(p: DragPayload) {
      payload = p;
    },
    destroy() {
      node.removeEventListener('pointerdown', down);
      node.removeEventListener('click', click, true);
    },
  };
};

let seq = 0;
/** `use:dropTarget={{ accepts, onDrop }}` */
export const dropTarget: Action<HTMLElement, Target> = (node, initial) => {
  const id = `drop-${++seq}`;
  node.dataset.dropId = id;
  targets.set(id, initial);
  return {
    update(t: Target) {
      targets.set(id, t);
    },
    destroy() {
      targets.delete(id);
    },
  };
};

/** True while this element's drop target is hovered (for styling). */
export const isOver = (node: HTMLElement | undefined) => !!node && drag.over === node.dataset.dropId;
