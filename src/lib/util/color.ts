// Resolve CSS custom properties (OKLCH) to hex for consumers that can't parse oklch()
// (xterm's theme parser). The browser does the color math via a 1×1 canvas.

let ctx: CanvasRenderingContext2D | null = null;

export function cssToHex(color: string): string {
  ctx ??= document.createElement('canvas').getContext('2d', { willReadFrequently: true });
  if (!ctx) return '#000000';
  ctx.clearRect(0, 0, 1, 1);
  ctx.fillStyle = '#000';
  ctx.fillStyle = color;
  ctx.fillRect(0, 0, 1, 1);
  const [r, g, b, a] = ctx.getImageData(0, 0, 1, 1).data;
  const hex = (n: number) => n.toString(16).padStart(2, '0');
  return `#${hex(r)}${hex(g)}${hex(b)}${a < 255 ? hex(a) : ''}`;
}

/** Read a token from :root (or `el`) and return it as hex. */
export function tokenHex(name: string, el: Element = document.documentElement): string {
  const v = getComputedStyle(el).getPropertyValue(name).trim();
  return v ? cssToHex(v) : '#000000';
}
