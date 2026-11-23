// The secondary inks carry small text the user must read: times, the queue and status lines,
// the model and permission labels, diff line numbers. WCAG AA asks 4.5:1 for such text. This
// reads the real token values and computes the ratios, so a new value that looks fine but
// measures 4.3:1 fails here.
import { describe, expect, it } from 'vitest';
import css from '../tokens.css?raw';

type Rgb = [number, number, number];
type Rgba = { rgb: Rgb; alpha: number };

/** The declarations of the block that starts with `selector`. */
function block(selector: string): Map<string, string> {
  const start = css.indexOf(`${selector} {`);
  expect(start, selector).toBeGreaterThanOrEqual(0);
  const body = css.slice(start, css.indexOf('\n}', start));
  return new Map([...body.matchAll(/(--[\w-]+):\s*([^;]+);/g)].map((m): [string, string] => [m[1], m[2].trim()]));
}

function color(value: string): Rgba {
  const hex = /^#([0-9a-f]{6})$/i.exec(value);
  if (hex) return { rgb: [0, 2, 4].map((i) => parseInt(hex[1].slice(i, i + 2), 16)) as Rgb, alpha: 1 };
  const rgba = /^rgba\(\s*(\d+),\s*(\d+),\s*(\d+),\s*([\d.]+)\s*\)$/.exec(value);
  if (rgba) return { rgb: [Number(rgba[1]), Number(rgba[2]), Number(rgba[3])], alpha: Number(rgba[4]) };
  throw new Error(`not a colour this test reads: ${value}`);
}

/** A translucent colour painted on an opaque one. */
const over = (top: Rgba, under: Rgb): Rgb => top.rgb.map((c, i) => c * top.alpha + under[i] * (1 - top.alpha)) as Rgb;

/** WCAG 2 relative luminance. */
function luminance(rgb: Rgb): number {
  const [r, g, b] = rgb.map((c) => {
    const v = c / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG 2 contrast ratio, 1 to 21. */
function contrast(a: Rgb, b: Rgb): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

const AA_TEXT = 4.5;
/** AA for large text and for marks that carry meaning (WCAG 1.4.3, 1.4.11). */
const AA_LARGE = 3;

describe.each([
  ['night', ':root, [data-theme="dark"]'],
  ['day', '[data-theme="light"]'],
])('secondary text in the %s theme', (_name, selector) => {
  const t = block(selector);
  const ink = (n: number) => color(t.get(`--ink-${n}`)!).rgb;
  const solids: [string, Rgb][] = [
    ['--glass-solid', color(t.get('--glass-solid')!).rgb],
    ['--pop', color(t.get('--pop')!).rgb],
  ];
  const raised: [string, Rgb][] = solids.map(([name, bg]) => [`--surface-3 on ${name}`, over(color(t.get('--surface-3')!), bg)]);

  it.each(solids)('--ink-3 and --ink-4 are readable on %s', (_bg, bg) => {
    expect(contrast(ink(3), bg)).toBeGreaterThanOrEqual(AA_TEXT);
    expect(contrast(ink(4), bg)).toBeGreaterThanOrEqual(AA_TEXT);
  });

  it.each(raised)('--ink-3 is readable on %s, and --ink-4 keeps the large-text ratio there', (_bg, bg) => {
    expect(contrast(ink(3), bg)).toBeGreaterThanOrEqual(AA_TEXT);
    expect(contrast(ink(4), bg)).toBeGreaterThanOrEqual(AA_LARGE);
  });

  it('the four inks stay four visible steps', () => {
    const bg = solids[0][1];
    const ratios = [1, 2, 3, 4].map((n) => contrast(ink(n), bg));
    for (let i = 1; i < ratios.length; i++) expect(ratios[i - 1] / ratios[i]).toBeGreaterThanOrEqual(1.15);
  });
});
