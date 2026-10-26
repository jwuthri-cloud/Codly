// Entry for office-demo.html: `pnpm dev` → http://localhost:1420/office-demo.html
//   ?theme=light   start in daylight
//   ?skip=30       fold the first 30 s of the session into history (replay), play the rest live
//   ?speed=2       play faster
//   ?paused=1      history only, no live feed (a still office: zero frames once settled)
//   ?backdrop=1    backdrop mode, with a mock conversation column on top
//   ?rm=1          reduced motion
//   ?select=a-ada  start with an agent selected
import '@fontsource-variable/inter';
import '@fontsource-variable/jetbrains-mono';
import { mount } from 'svelte';
import Harness from './demo/Harness.svelte';

// Count animation frames and timers requested by anything on the page (idle verification).
const w = window as unknown as { __raf: number; __timeouts: number; __OFFICE_DEBUG: boolean };
w.__OFFICE_DEBUG = true;
w.__raf = 0;
w.__timeouts = 0;
const raf = window.requestAnimationFrame.bind(window);
window.requestAnimationFrame = (cb: FrameRequestCallback) => {
  w.__raf++;
  return raf(cb);
};
const st = window.setTimeout.bind(window);
window.setTimeout = ((cb: TimerHandler, ms?: number, ...args: unknown[]) => {
  w.__timeouts++;
  return st(cb, ms, ...args);
}) as typeof window.setTimeout;

const params = new URLSearchParams(location.search);
document.documentElement.dataset.theme = params.get('theme') === 'light' ? 'light' : 'dark';

mount(Harness, { target: document.getElementById('office')! });
