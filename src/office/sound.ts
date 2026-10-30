// The office's sounds: samples in public/audio (CC0, see CREDITS.txt) and a few made here (a
// crowd, a referee's whistle, an espresso machine's hiss, a water cooler's glug). Sounds in the
// office are placed: louder near the camera's focus and when zoomed in, panned left or right.
// One shared instance (one office at a time); muted is remembered on this machine.

export type Sfx =
  | 'ui-click' | 'ui-open' | 'ui-close' | 'ui-buy' | 'ui-sell' | 'ui-place' | 'ui-rotate' | 'ui-error'
  | 'pool-ball' | 'pool-cue' | 'pool-pocket' | 'foos-kick' | 'foos-spin' | 'pad'
  | 'fridge-open' | 'fridge-close' | 'key' | 'jingle-merge' | 'jingle-promote';

/** Variants picked at random (pool-ball-1 … pool-ball-3). */
const VARIANTS: Partial<Record<Sfx, number>> = { 'pool-ball': 3, 'foos-kick': 2, pad: 3, key: 8 };
const BASE = `${import.meta.env?.BASE_URL ?? '/'}audio/`;
const MUTE_KEY = 'codly.sound';

/** Where a sound is, as the listener hears it: 0 … 1 loudness and −1 … 1 pan. */
export type Hearing = (x: number, z: number) => { gain: number; pan: number };

class Sound {
  private ctx: AudioContext | null = null;
  private out: GainNode | null = null;
  private buffers = new Map<string, Promise<AudioBuffer | null>>();
  private noiseBuf: AudioBuffer | null = null;
  private song: { name: string; src: AudioBufferSourceNode | null; gain: GainNode } | null = null;
  muted = (() => {
    try {
      return localStorage.getItem(MUTE_KEY) === 'off';
    } catch {
      return false;
    }
  })();
  /** How the office hears a point (the engine sets it from its camera). */
  hearing: Hearing | null = null;

  constructor() {
    if (typeof window === 'undefined') return;
    // Browsers start audio only after a gesture.
    const wake = () => void this.audio()?.resume();
    window.addEventListener('pointerdown', wake, { capture: true });
    window.addEventListener('keydown', wake, { capture: true });
  }

  private audio(): AudioContext | null {
    if (this.ctx) return this.ctx;
    const AC = typeof window !== 'undefined' ? (window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext) : undefined;
    if (!AC) return null;
    this.ctx = new AC();
    this.out = this.ctx.createGain();
    this.out.gain.value = this.muted ? 0 : 0.8;
    this.out.connect(this.ctx.destination);
    return this.ctx;
  }

  setMuted(m: boolean) {
    this.muted = m;
    try {
      localStorage.setItem(MUTE_KEY, m ? 'off' : 'on');
    } catch {
      // Private mode: it holds for this run.
    }
    if (this.out && this.ctx) this.out.gain.setTargetAtTime(m ? 0 : 0.8, this.ctx.currentTime, 0.05);
  }

  private load(file: string): Promise<AudioBuffer | null> {
    let b = this.buffers.get(file);
    if (!b) {
      const ctx = this.audio();
      b = ctx
        ? fetch(BASE + file)
            .then((r) => r.arrayBuffer())
            .then((a) => ctx.decodeAudioData(a))
            .catch(() => null)
        : Promise.resolve(null);
      this.buffers.set(file, b);
    }
    return b;
  }

  /** A chain into the speakers: gain (and pan) for a sound at `at` (floor plan), or flat. */
  private chain(gain: number, at?: [number, number]): AudioNode | null {
    const ctx = this.audio();
    if (!ctx || !this.out || this.muted || ctx.state !== 'running') return null;
    let g = gain;
    let pan = 0;
    if (at && this.hearing) {
      const h = this.hearing(at[0], at[1]);
      g *= h.gain;
      pan = h.pan;
    }
    if (g < 0.01) return null;
    const v = ctx.createGain();
    v.gain.value = g;
    if (pan && ctx.createStereoPanner) {
      const p = ctx.createStereoPanner();
      p.pan.value = Math.max(-1, Math.min(1, pan));
      v.connect(p).connect(this.out);
    } else v.connect(this.out);
    return v;
  }

  /** Play a sample; `at` places it in the office (floor plan x, z). */
  play(name: Sfx, opts: { at?: [number, number]; gain?: number; rate?: number } = {}) {
    const node = this.chain(opts.gain ?? 1, opts.at);
    if (!node) return;
    const n = VARIANTS[name];
    const file = `${name}${n ? `-${1 + Math.floor(Math.random() * n)}` : ''}.m4a`;
    void this.load(file).then((buf) => {
      if (!buf || !this.ctx) return;
      const s = this.ctx.createBufferSource();
      s.buffer = buf;
      s.playbackRate.value = opts.rate ?? 0.94 + Math.random() * 0.12;
      s.connect(node);
      s.start();
    });
  }

  private noise(): AudioBuffer | null {
    const ctx = this.audio();
    if (!ctx) return null;
    if (!this.noiseBuf) {
      this.noiseBuf = ctx.createBuffer(1, ctx.sampleRate * 2, ctx.sampleRate);
      const d = this.noiseBuf.getChannelData(0);
      for (let i = 0; i < d.length; i++) d[i] = Math.random() * 2 - 1;
    }
    return this.noiseBuf;
  }

  /** Filtered noise shaped by an envelope ([time s, level] points). */
  private noiseBurst(node: AudioNode, filter: BiquadFilterType, freq: number, q: number, env: [number, number][]) {
    const ctx = this.ctx!;
    const s = ctx.createBufferSource();
    s.buffer = this.noise();
    s.loop = true;
    const f = ctx.createBiquadFilter();
    f.type = filter;
    f.frequency.value = freq;
    f.Q.value = q;
    const g = ctx.createGain();
    const t0 = ctx.currentTime;
    g.gain.setValueAtTime(0, t0);
    for (const [t, v] of env) g.gain.linearRampToValueAtTime(v, t0 + t);
    s.connect(f).connect(g).connect(node);
    s.start(t0);
    s.stop(t0 + env[env.length - 1][0] + 0.05);
  }

  /** The made-up sounds: the TV crowd, a whistle, steam from the espresso machine, a glug. */
  synth(kind: 'crowd' | 'crowd-goal' | 'crowd-ooh' | 'whistle' | 'whistle-end' | 'hiss' | 'glug', opts: { at?: [number, number]; gain?: number } = {}) {
    const node = this.chain(opts.gain ?? 1, opts.at);
    if (!node || !this.ctx) return;
    const ctx = this.ctx;
    switch (kind) {
      case 'crowd':
        return this.noiseBurst(node, 'bandpass', 900, 0.6, [[0.6, 0.18], [1.6, 0.22], [2.4, 0]]);
      case 'crowd-goal':
        return this.noiseBurst(node, 'bandpass', 1100, 0.5, [[0.15, 0.55], [1.2, 0.5], [2.8, 0]]);
      case 'crowd-ooh':
        return this.noiseBurst(node, 'lowpass', 600, 0.8, [[0.2, 0.3], [0.9, 0.12], [1.4, 0]]);
      case 'hiss':
        // Grinding, then steam.
        this.noiseBurst(node, 'bandpass', 2600, 1.4, [[0.05, 0.12], [0.6, 0.12], [0.7, 0]]);
        return this.noiseBurst(node, 'highpass', 4200, 0.7, [[0.9, 0], [1.1, 0.22], [2.2, 0.18], [2.5, 0]]);
      case 'whistle':
      case 'whistle-end': {
        // A pea whistle: a high tone with a fast trill; full time is three blasts.
        const blasts = kind === 'whistle' ? [[0, 0.32]] : [[0, 0.22], [0.32, 0.22], [0.64, 0.6]];
        for (const [start, len] of blasts) {
          const o = ctx.createOscillator();
          o.frequency.value = 2900;
          const lfo = ctx.createOscillator();
          lfo.frequency.value = 38;
          const depth = ctx.createGain();
          depth.gain.value = 140;
          lfo.connect(depth).connect(o.frequency);
          const g = ctx.createGain();
          const t0 = ctx.currentTime + start;
          g.gain.setValueAtTime(0, t0);
          g.gain.linearRampToValueAtTime(0.08, t0 + 0.02);
          g.gain.setValueAtTime(0.08, t0 + len - 0.04);
          g.gain.linearRampToValueAtTime(0, t0 + len);
          o.connect(g).connect(node);
          for (const n of [o, lfo]) {
            n.start(t0);
            n.stop(t0 + len + 0.02);
          }
        }
        return;
      }
      case 'glug': {
        // Bubbles rising in the bottle: short falling blips.
        for (let i = 0; i < 6; i++) {
          const o = ctx.createOscillator();
          o.type = 'sine';
          const t0 = ctx.currentTime + i * 0.28 + Math.random() * 0.08;
          const f0 = 320 + Math.random() * 160;
          o.frequency.setValueAtTime(f0, t0);
          o.frequency.exponentialRampToValueAtTime(f0 * 1.8, t0 + 0.09);
          const g = ctx.createGain();
          g.gain.setValueAtTime(0, t0);
          g.gain.linearRampToValueAtTime(0.16, t0 + 0.01);
          g.gain.exponentialRampToValueAtTime(0.001, t0 + 0.12);
          o.connect(g).connect(node);
          o.start(t0);
          o.stop(t0 + 0.14);
        }
        return;
      }
    }
  }

  /** Background music (looped, faded in), or null to fade it out. */
  music(name: string | null) {
    if ((this.song?.name ?? null) === name) return;
    const ctx = this.audio();
    if (!ctx || !this.out) return;
    const old = this.song;
    if (old) {
      old.gain.gain.setTargetAtTime(0, ctx.currentTime, 0.4);
      setTimeout(() => old.src?.stop(), 2000);
    }
    this.song = null;
    if (!name) return;
    const gain = ctx.createGain();
    gain.gain.value = 0;
    gain.connect(this.out);
    const song = { name, src: null as AudioBufferSourceNode | null, gain };
    this.song = song;
    void this.load(`${name}.m4a`).then((buf) => {
      if (!buf || this.song !== song) return;
      const s = ctx.createBufferSource();
      s.buffer = buf;
      s.loop = true;
      s.connect(gain);
      s.start();
      song.src = s;
      gain.gain.setTargetAtTime(0.35, ctx.currentTime, 0.6);
    });
  }
}

export const sound = new Sound();
