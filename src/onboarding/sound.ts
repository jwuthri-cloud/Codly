// Tiny WebAudio blips, no assets. Silent unless the user left sound on, and never under reduced motion.
let ctx: AudioContext | null = null;

function tone(freq: number, at: number, dur: number, type: OscillatorType = 'square', gain = 0.05) {
  const c = ctx!;
  const o = c.createOscillator();
  const g = c.createGain();
  o.type = type;
  o.frequency.value = freq;
  g.gain.setValueAtTime(0, c.currentTime + at);
  g.gain.linearRampToValueAtTime(gain, c.currentTime + at + 0.01);
  g.gain.exponentialRampToValueAtTime(0.0001, c.currentTime + at + dur);
  o.connect(g).connect(c.destination);
  o.start(c.currentTime + at);
  o.stop(c.currentTime + at + dur + 0.02);
}

export type Blip = 'tap' | 'level' | 'done';
const NOTES: Record<Blip, [number, number, number][]> = {
  tap: [[660, 0, 0.06]],
  level: [[523, 0, 0.09], [659, 0.09, 0.09], [784, 0.18, 0.16]],
  done: [[523, 0, 0.1], [659, 0.1, 0.1], [784, 0.2, 0.1], [1047, 0.3, 0.3]],
};

export function blip(kind: Blip, enabled: boolean) {
  if (!enabled || typeof AudioContext === 'undefined') return;
  try {
    ctx ??= new AudioContext();
    if (ctx.state === 'suspended') void ctx.resume();
    for (const [f, at, d] of NOTES[kind]) tone(f, at, d);
  } catch {
    /* no audio device */
  }
}
