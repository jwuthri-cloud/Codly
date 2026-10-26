// Colors of the office. Two moods with the same furniture: light = soft daylight through the
// windows, dark = evening with warm lamplight and dusky windows. Base material colors stay the
// same; light, exposure and emissive surfaces change (so it reads as the same room at night).
// Identity hues are the app's --id-1..--id-8 (OKLCH), so a shirt matches the agent's chip.

import { Color } from 'three';

export type ThemeName = 'dark' | 'light';
export type RGB = [number, number, number];

/** OKLCH → linear sRGB (clamped). L in 0..1, C ~0..0.37, h in degrees. */
export function oklchLinear(L: number, C: number, h: number): RGB {
  const hr = (h * Math.PI) / 180;
  const a = C * Math.cos(hr);
  const b = C * Math.sin(hr);
  const l_ = L + 0.3963377774 * a + 0.2158037573 * b;
  const m_ = L - 0.1055613458 * a - 0.0638541728 * b;
  const s_ = L - 0.0894841775 * a - 1.291485548 * b;
  const l = l_ * l_ * l_;
  const m = m_ * m_ * m_;
  const s = s_ * s_ * s_;
  const r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s;
  const g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s;
  const bl = -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s;
  return [clamp01(r), clamp01(g), clamp01(bl)];
}

const clamp01 = (x: number) => (x < 0 ? 0 : x > 1 ? 1 : x);

function toSrgb(c: number): number {
  return c <= 0.0031308 ? 12.92 * c : 1.055 * Math.pow(c, 1 / 2.4) - 0.055;
}

export function linearToHex([r, g, b]: RGB): string {
  const h = (x: number) =>
    Math.round(clamp01(toSrgb(x)) * 255)
      .toString(16)
      .padStart(2, '0');
  return `#${h(r)}${h(g)}${h(b)}`;
}

export function oklchHex(L: number, C: number, h: number): string {
  return linearToHex(oklchLinear(L, C, h));
}

/** sRGB hex → linear RGB (what vertex colors and material colors want). */
export function lin(hex: string): RGB {
  const n = parseInt(hex.replace('#', ''), 16);
  const f = (v: number) => {
    const c = v / 255;
    return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  };
  return [f((n >> 16) & 255), f((n >> 8) & 255), f(n & 255)];
}

export function mix(a: RGB, b: RGB, t: number): RGB {
  return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
}

export function scale(a: RGB, k: number): RGB {
  return [a[0] * k, a[1] * k, a[2] * k];
}

export function color(rgb: RGB): Color {
  return new Color().setRGB(rgb[0], rgb[1], rgb[2]);
}

// ───────────── identity hues (the app's --id-1..--id-8) ─────────────

export const ID_HUES = [20, 60, 105, 150, 195, 240, 285, 330];

export function idHue(c: number): number {
  return ID_HUES[((Math.trunc(c) % 8) + 8) % 8];
}

/** The agent's identity color: shirt (cloth), glow (lights), ink (readable on the theme). */
export function identity(c: number, theme: ThemeName) {
  const h = idHue(c);
  const ch = h === 195 ? 0.1 : h === 105 ? 0.12 : 0.135;
  return {
    hue: h,
    shirt: oklchLinear(0.66, ch, h),
    shirtDark: oklchLinear(0.5, ch * 0.9, h),
    glow: oklchLinear(theme === 'dark' ? 0.8 : 0.68, ch + 0.03, h),
    ink: oklchHex(theme === 'dark' ? 0.8 : 0.5, ch, h),
    soft: oklchHex(theme === 'dark' ? 0.78 : 0.62, ch, h),
    screen: oklchHex(0.78, ch + 0.02, h),
  };
}

// ───────────── materials (sRGB hex, converted where used) ─────────────

export const MAT = {
  oak: '#d8b48c',
  oakDark: '#c9a07a',
  walnut: '#a3826a',
  walnutDark: '#86593e',
  carpetSage: '#a8b795',
  carpetBlue: '#b7b9a8',
  tileCool: '#cdd2d6',
  tileCoolLine: '#b5bcc2',
  tileWarm: '#eee3d1',
  tileTerracotta: '#d38c69',
  terrazzo: '#e7dfd3',
  wall: '#efe7da',
  wallInner: '#ece2d2',
  wallCap: '#5c4738',
  baseboard: '#cfc1ab',
  slab: '#c9b79e',
  slabDark: '#a8957b',
  birch: '#ead9bd',
  charcoal: '#36373d',
  metal: '#8e939b',
  fabricTeal: '#5f8f8a',
  fabricRust: '#b8643f',
  fabricMustard: '#d6a14a',
  fabricOlive: '#7d8a5c',
  leather: '#6e4632',
  monitor: '#25272e',
  screenOff: '#14161b',
  brass: '#c7a25c',
  shade: '#f3e6cc',
  shelf: '#a3744f',
  whiteboard: '#f6f5f1',
  frame: '#9aa1a8',
  cork: '#d8b083',
  rack: '#737985',
  rackFront: '#838a97',
  mailRed: '#cf5a3e',
  flag: '#f2c14e',
  cabinet: '#8d9d86',
  cabinetDark: '#76866f',
  counter: '#f3eee6',
  counterCab: '#7aa497',
  fridge: '#e9e5de',
  door: '#a8794f',
  doorFrame: '#6c4f3e',
  leaf: '#5d9a58',
  leafLight: '#7fb46a',
  leafDark: '#3f7c4c',
  potTerracotta: '#c2714f',
  potWhite: '#ece6dc',
  potCharcoal: '#3d3d43',
  paper: '#fbf8f1',
  glass: '#cfe3ea',
};

export const BOOKS = ['#c45b4a', '#e0a458', '#4f7c8a', '#6d8b5b', '#8c6bb1', '#d9c7a7', '#38506b', '#b8844f', '#9c3f3a', '#5a8f8a'];

export const SKIN = ['#f7d9c4', '#efc3a2', '#dca37d', '#c08259', '#93603f', '#6b432d'];
export const HAIR = ['#2b1d16', '#4a2f22', '#6b4429', '#9d6136', '#d6a85a', '#e9d4a3', '#8b8b8b', '#c4543c', '#2f3f6e', '#8e4e9a'];
export const HAIR_WEIGHTS = [16, 14, 12, 8, 8, 4, 3, 6, 2, 2];
export const PANTS = ['#3b4252', '#4a4237', '#2f3a3a', '#5b4f68', '#5a4636', '#2e3442'];

export const PROVIDER: Record<string, { bg: string; fg: string; label: string }> = {
  claude: { bg: '#d97757', fg: '#fff7f0', label: 'Claude Code' },
  codex: { bg: '#1e1f22', fg: '#f3f3f3', label: 'Codex' },
  gemini: { bg: '#4b8bf5', fg: '#ffffff', label: 'Gemini CLI' },
  opencode: { bg: '#d9d9d9', fg: '#1f1f1f', label: 'OpenCode' },
  cursor: { bg: '#151515', fg: '#e8e8e8', label: 'Cursor' },
  copilot: { bg: '#8957e5', fg: '#ffffff', label: 'Copilot' },
};

// ───────────── themes ─────────────

export interface OfficeTheme {
  name: ThemeName;
  /** CSS background around the diorama. */
  sky: string;
  hemiSky: RGB;
  hemiGround: RGB;
  hemiIntensity: number;
  key: RGB;
  keyIntensity: number;
  keyDir: [number, number, number];
  shadowStrength: number;
  exposure: number;
  /** Warm lamps: emissive strength of shades and the pools of light on the floor. */
  lamp: RGB;
  lampGlow: number;
  pool: number;
  /** Point lights (evening only). */
  points: number;
  screenGlow: number;
  window: { top: string; bottom: string; stars: boolean; glow: number };
  amber: RGB;
  danger: RGB;
  success: RGB;
  /** Floor label / overlay tint. */
  labelInk: string;
}

export const THEMES: Record<ThemeName, OfficeTheme> = {
  light: {
    name: 'light',
    sky: 'radial-gradient(120% 95% at 50% 8%, #fbf8f2 0%, #f3ece1 58%, #e9dfcf 100%)',
    hemiSky: lin('#fff8ec'),
    hemiGround: lin('#b9a68c'),
    hemiIntensity: 1.55,
    key: lin('#fff0da'),
    keyIntensity: 2.35,
    keyDir: [0.5, 1, 0.3],
    shadowStrength: 0.42,
    exposure: 1.0,
    lamp: lin('#ffd79a'),
    lampGlow: 0.25,
    pool: 0,
    points: 0,
    screenGlow: 0.85,
    window: { top: '#bfe0f2', bottom: '#eaf5f8', stars: false, glow: 0.9 },
    amber: lin('#f0a23a'),
    danger: lin('#d9534a'),
    success: lin('#3fae6a'),
    labelInk: 'rgba(92, 72, 56, 0.55)',
  },
  dark: {
    name: 'dark',
    sky: 'radial-gradient(120% 95% at 50% 0%, #20213a 0%, #15162a 52%, #0c0d18 100%)',
    hemiSky: lin('#9a93c4'),
    hemiGround: lin('#6a5040'),
    hemiIntensity: 0.85,
    key: lin('#ffd2a6'),
    keyIntensity: 1.3,
    keyDir: [0.35, 1, 0.25],
    shadowStrength: 0.55,
    exposure: 1.22,
    lamp: lin('#ffbe73'),
    lampGlow: 1.6,
    pool: 0.4,
    points: 1,
    screenGlow: 1.25,
    window: { top: '#1d2150', bottom: '#d9825f', stars: true, glow: 1.1 },
    amber: lin('#ffb347'),
    danger: lin('#ff6b5e'),
    success: lin('#5ee08f'),
    labelInk: 'rgba(236, 222, 200, 0.5)',
  },
};
