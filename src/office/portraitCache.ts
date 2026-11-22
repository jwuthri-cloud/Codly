// Portraits for the app's own UI (crew strip, chat, sheets), drawn off the first paint.
// portrait.ts draws one (three.js, the character builder, a WebGL context); this cache asks for
// it only when the page has painted, one portrait at a time, and draws each agent once: at one
// size, which <img> scales. Until a portrait is ready `get` returns '' and the caller shows its
// monogram. No three.js here: this file is in the startup graph, portrait.ts must not be.

import type { Agent } from '../lib/gen/Agent';

/** What a portrait is drawn from (the same fields the office builds the character from). */
export type Sitter = Pick<Agent, 'id' | 'color' | 'look_seed' | 'provider' | 'role'>;

/** Draws a portrait at `px` CSS pixels: a PNG data URL, or '' when it cannot (no WebGL). */
export type Draw = (agent: Sitter, px: number) => string;

export interface PortraitHost {
  /** Load the drawing code (a dynamic import of portrait.ts). */
  load(): Promise<Draw>;
  /** Run `cb` once the page has painted. */
  later(cb: () => void): void;
}

/** The one size every portrait is drawn at (CSS px; the biggest the UI shows). */
export const PORTRAIT_PX = 72;

/** Portraits kept at most (job applicants come and go; an agent's look can change). */
const KEEP = 300;

const keyOf = (a: Sitter) => `${a.look_seed || a.id}|${a.color}|${a.provider}|${a.role}`;

export class PortraitCache {
  private drawn = new Map<string, string>();
  /** Drawn for the theme before this one: shown until the new one is ready. */
  private stale = new Map<string, string>();
  private queue = new Map<string, Sitter>();
  private loading: Promise<Draw> | null = null;
  private busy = false;

  /** `onready` is called each time a portrait is ready (or the theme changed): ask again. */
  constructor(
    private host: PortraitHost,
    private onready: () => void,
  ) {}

  /** The agent's portrait, or '' while it is not drawn yet (it is queued). */
  get(a: Sitter): string {
    const key = keyOf(a);
    const hit = this.drawn.get(key);
    if (hit != null) return hit;
    if (!this.queue.has(key)) {
      this.queue.set(key, a);
      this.pump();
    }
    return this.stale.get(key) ?? '';
  }

  /** The theme changed (a portrait sits on a disc in the theme's colours): draw them again. */
  retheme() {
    this.stale = this.drawn;
    this.drawn = new Map();
    this.onready();
  }

  private pump() {
    if (this.busy) return;
    this.busy = true;
    this.host.later(() => void this.start());
  }

  private async start() {
    let draw: Draw;
    try {
      draw = await (this.loading ??= this.host.load());
    } catch (e) {
      // The monograms stay; the next portrait asked for tries the load again.
      console.error('[codly] portraits could not load', e);
      this.loading = null;
      this.queue.clear();
      this.busy = false;
      return;
    }
    this.next(draw);
  }

  private next(draw: Draw) {
    const first = this.queue.entries().next();
    if (first.done) {
      this.busy = false;
      return;
    }
    const [key, agent] = first.value;
    this.queue.delete(key);
    let url = '';
    try {
      url = draw(agent, PORTRAIT_PX);
    } catch (e) {
      console.error('[codly] a portrait could not be drawn', e);
    }
    if (this.drawn.size >= KEEP) this.drawn.clear();
    this.drawn.set(key, url);
    this.stale.delete(key);
    this.onready();
    if (this.queue.size) this.host.later(() => this.next(draw));
    else this.busy = false;
  }
}
