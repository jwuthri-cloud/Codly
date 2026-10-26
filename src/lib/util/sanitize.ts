// Display sanitization for untrusted text (file names, commands, model output) — design §8.6.
// Svelte's `{text}` interpolation is already textContent; this strips what textContent doesn't:
// control characters, bidi overrides and zero-width characters, then truncates by grapheme.

// C0 (except \t \n \r) and C1 controls.
const CONTROL = /[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F-\u009F]/g;
// Bidi embeddings/overrides/isolates and marks.
const BIDI = /[‪-‮⁦-⁩‎‏؜]/g;
// Zero-width characters (keep ZWJ out of emoji sequences? we strip it: labels are short).
const ZERO_WIDTH = /[​-‍⁠﻿]/g;
// ANSI escape sequences (CSI / OSC), common in tool output.
const ANSI = /\u001B(?:\[[0-?]*[ -/]*[@-~]|\][^\u0007\u001B]*(?:\u0007|\u001B\\)|[@-Z\\-_])/g;

/** Strip escapes, controls, bidi and zero-width characters. Keeps newlines and tabs. */
export function clean(s: string | null | undefined): string {
  if (!s) return '';
  return s.replace(ANSI, '').replace(CONTROL, '').replace(BIDI, '').replace(ZERO_WIDTH, '');
}

let segmenter: Intl.Segmenter | null = null;
function graphemes(s: string): string[] {
  if (typeof Intl !== 'undefined' && 'Segmenter' in Intl) {
    segmenter ??= new Intl.Segmenter(undefined, { granularity: 'grapheme' });
    return Array.from(segmenter.segment(s), (g) => g.segment);
  }
  return Array.from(s);
}

/** Truncate by grapheme with an ellipsis. */
export function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  const g = graphemes(s);
  if (g.length <= max) return s;
  return g.slice(0, Math.max(1, max - 1)).join('').trimEnd() + '…';
}

/** One-line label: cleaned, whitespace collapsed, truncated. */
export function oneLine(s: string | null | undefined, max = 120): string {
  return truncate(clean(s).replace(/\s+/g, ' ').trim(), max);
}

export function basename(path: string | null | undefined): string {
  if (!path) return '';
  const p = path.replace(/[\\/]+$/, '');
  const i = Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\'));
  return i >= 0 ? p.slice(i + 1) : p;
}

export function dirname(path: string): string {
  const i = path.lastIndexOf('/');
  return i > 0 ? path.slice(0, i) : '';
}

/** "~/…" for paths under the home directory (display only). */
export function tildify(path: string | null | undefined): string {
  if (!path) return '';
  return path.replace(/^\/Users\/[^/]+/, '~').replace(/^\/home\/[^/]+/, '~');
}

/** First `n` whitespace-separated tokens of a command (bubble rule: commands show 2 tokens). */
export function commandHead(cmd: string, n = 2): string {
  const parts = clean(cmd).trim().split(/\s+/);
  return parts.slice(0, n).join(' ') + (parts.length > n ? '…' : '');
}
