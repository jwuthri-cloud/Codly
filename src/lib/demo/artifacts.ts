// Artifacts the demo agents produce: a contrast chart (SVG) and its data (CSV) from t1, an API
// document (Markdown) from t2, a generated illustration (PNG, drawn with a canvas at runtime)
// and an interactive preview page (HTML) from t3. Bytes are real files the viewer can open.

const enc = new TextEncoder();
export const bytesOf = (s: string): ArrayBuffer => enc.encode(s).buffer as ArrayBuffer;

// ───────────── WCAG contrast of the demo's real tokens (tokens.css) ─────────────

const TOKENS = {
  light: { bg: '#ffffff', 'bg-subtle': '#f6f6f4', fg: '#1b1b18', 'fg-muted': '#6b6b66', accent: '#2f6fea' },
  dark: { bg: '#121210', 'bg-subtle': '#1a1a17', fg: '#f2f2ee', 'fg-muted': '#9a9a93', accent: '#6f9bff' },
} as const;

function lum(hex: string) {
  const c = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
  return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
}
const ratio = (a: string, b: string) => {
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
};

const PAIRS: [fg: keyof (typeof TOKENS)['light'], bg: keyof (typeof TOKENS)['light']][] = [
  ['fg', 'bg'],
  ['fg-muted', 'bg'],
  ['accent', 'bg'],
  ['fg-muted', 'bg-subtle'],
];

export function contrastRows() {
  return (['light', 'dark'] as const).flatMap((theme) =>
    PAIRS.map(([fg, bg]) => {
      const r = ratio(TOKENS[theme][fg], TOKENS[theme][bg]);
      return { theme, fg, bg, ratio: Math.round(r * 100) / 100, aa: r >= 4.5 };
    }),
  );
}

export function contrastCsv(): string {
  const rows = contrastRows();
  return ['theme,foreground,background,ratio,passes_aa', ...rows.map((r) => `${r.theme},--${r.fg},--${r.bg},${r.ratio.toFixed(2)},${r.aa ? 'yes' : 'no'}`)].join('\n') + '\n';
}

export function contrastSvg(): string {
  const rows = contrastRows();
  const W = 720;
  const H = 400;
  const left = 64;
  const top = 72;
  const plotH = 250;
  const max = 21;
  const y = (v: number) => top + plotH - (v / max) * plotH;
  const group = (W - left - 32) / PAIRS.length;
  const bw = 34;
  const bars = PAIRS.map(([fg, bg], gi) => {
    const x0 = left + gi * group + group / 2 - bw - 3;
    const l = rows.find((r) => r.theme === 'light' && r.fg === fg && r.bg === bg)!;
    const d = rows.find((r) => r.theme === 'dark' && r.fg === fg && r.bg === bg)!;
    const bar = (r: typeof l, x: number, fill: string, ink: string) =>
      `<rect x="${x}" y="${y(r.ratio)}" width="${bw}" height="${top + plotH - y(r.ratio)}" rx="5" fill="${fill}"/>` +
      `<text x="${x + bw / 2}" y="${y(r.ratio) - 8}" text-anchor="middle" font-size="12" font-weight="600" fill="${ink}">${r.ratio.toFixed(1)}</text>`;
    return (
      bar(l, x0, '#d9dde7', '#3b4252') +
      bar(d, x0 + bw + 6, '#2b3245', '#2b3245') +
      `<text x="${x0 + bw + 3}" y="${top + plotH + 22}" text-anchor="middle" font-size="12" fill="#3b4252">--${fg}</text>` +
      `<text x="${x0 + bw + 3}" y="${top + plotH + 38}" text-anchor="middle" font-size="11" fill="#7b8294">on --${bg}</text>`
    );
  }).join('');
  const grid = [0, 3, 7, 14, 21].map((v) => `<line x1="${left}" x2="${W - 24}" y1="${y(v)}" y2="${y(v)}" stroke="#eceef3"/><text x="${left - 10}" y="${y(v) + 4}" text-anchor="end" font-size="11" fill="#9aa0ae">${v}</text>`).join('');
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" font-family="Inter, -apple-system, system-ui, sans-serif">
  <rect width="${W}" height="${H}" rx="14" fill="#ffffff"/>
  <text x="${left - 40}" y="34" font-size="18" font-weight="650" fill="#1d2230">Token contrast, light vs dark</text>
  <text x="${left - 40}" y="54" font-size="12" fill="#7b8294">WCAG 2.2 contrast ratio of each text token on its background · computed from src/styles/tokens.css</text>
  ${grid}
  <line x1="${left}" x2="${W - 24}" y1="${y(4.5)}" y2="${y(4.5)}" stroke="#e0752d" stroke-width="1.5" stroke-dasharray="5 4"/>
  <text x="${left - 10}" y="${y(4.5) + 4}" text-anchor="end" font-size="10.5" font-weight="700" fill="#c25e1c">AA</text>
  ${bars}
  <rect x="${W - 196}" y="24" width="12" height="12" rx="3" fill="#d9dde7"/><text x="${W - 178}" y="34" font-size="12" fill="#3b4252">light</text>
  <rect x="${W - 128}" y="24" width="12" height="12" rx="3" fill="#2b3245"/><text x="${W - 110}" y="34" font-size="12" fill="#3b4252">dark</text>
</svg>
`;
}

// ───────────── t2: the settings API, documented ─────────────

export const SETTINGS_API_MD = `# Settings API

The signed-in user's preferences. Every field is optional on write; the response is always the full object.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| \`locale\` | \`string\` | \`"en"\` | BCP 47 tag |
| \`emailDigest\` | \`boolean\` | \`true\` | weekly summary |
| \`theme\` | \`"system" \\| "light" \\| "dark"\` | \`"system"\` | **new** — follows the OS until set |

## \`GET /api/settings\`

\`\`\`json
{ "locale": "en", "emailDigest": true, "theme": "system" }
\`\`\`

## \`PATCH /api/settings\`

\`\`\`json
{ "theme": "dark" }
\`\`\`

- Validated with zod (\`settingsPatch\`): unknown fields are rejected with **400**.
- Values outside \`system | light | dark\` are rejected with **400**.
- Returns the full settings object, so the client can trust the response as the new state.

> Migration \`20261025093000_add_theme\` adds the column with a default of \`system\`: existing users keep following their OS.
`;

// ───────────── t3: a preview page for the toggle ─────────────

export const TOGGLE_PREVIEW_HTML = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>ThemeToggle — preview</title>
<style>
  :root { color-scheme: light; font-family: Inter, -apple-system, system-ui, sans-serif; }
  body { margin: 0; padding: 40px; background: #f4f4f1; color: #1b1b18; }
  h1 { font-size: 20px; margin: 0 0 6px; }
  p.lede { margin: 0 0 28px; color: #6b6b66; font-size: 14px; }
  .grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 18px; }
  .frame { border-radius: 14px; padding: 22px; box-shadow: 0 1px 0 rgba(0,0,0,.04), 0 8px 24px -12px rgba(0,0,0,.25); }
  .frame[data-theme=light] { background: #fff; color: #1b1b18; }
  .frame[data-theme=dark] { background: #121210; color: #f2f2ee; }
  .label { font-size: 12px; text-transform: uppercase; letter-spacing: .06em; opacity: .6; margin-bottom: 14px; }
  .row { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .seg { display: inline-flex; padding: 3px; border-radius: 10px; background: rgba(127,127,127,.14); }
  .seg button { font: inherit; font-size: 13px; border: 0; background: transparent; color: inherit; padding: 6px 12px; border-radius: 8px; cursor: pointer; }
  .seg button[aria-checked=true] { background: #2f6fea; color: #fff; }
  .frame[data-theme=dark] .seg button[aria-checked=true] { background: #6f9bff; color: #121210; }
  .note { margin-top: 26px; font-size: 13px; color: #6b6b66; }
  .live { display: none; }
  body.scripted .live { display: inline; }
  body.scripted .static { display: none; }
</style>
</head>
<body>
  <h1>Appearance</h1>
  <p class="lede">The three states of the new toggle on <code>/settings</code>, rendered with the real tokens.</p>
  <div class="grid">
    <section class="frame" data-theme="light"><div class="label">System → light</div><div class="row"><span>Theme</span><div class="seg" role="radiogroup"><button role="radio" aria-checked="true">System</button><button role="radio" aria-checked="false">Light</button><button role="radio" aria-checked="false">Dark</button></div></div></section>
    <section class="frame" data-theme="light"><div class="label">Light</div><div class="row"><span>Theme</span><div class="seg" role="radiogroup"><button role="radio" aria-checked="false">System</button><button role="radio" aria-checked="true">Light</button><button role="radio" aria-checked="false">Dark</button></div></div></section>
    <section class="frame" data-theme="dark"><div class="label">Dark</div><div class="row"><span>Theme</span><div class="seg" role="radiogroup"><button role="radio" aria-checked="false">System</button><button role="radio" aria-checked="false">Light</button><button role="radio" aria-checked="true">Dark</button></div></div></section>
  </div>
  <p class="note"><span class="static">Static preview — turn on “Run scripts” to click through the states.</span><span class="live">Scripts are on: click any option to switch that frame.</span></p>
  <script>
    document.body.classList.add('scripted');
    document.querySelectorAll('.frame').forEach(function (f) {
      f.querySelectorAll('button').forEach(function (b) {
        b.addEventListener('click', function () {
          f.querySelectorAll('button').forEach(function (x) { x.setAttribute('aria-checked', String(x === b)); });
          f.dataset.theme = b.textContent === 'Dark' ? 'dark' : 'light';
          f.querySelector('.label').textContent = b.textContent === 'System' ? 'System → light' : b.textContent;
        });
      });
    });
  </script>
</body>
</html>
`;

export const ILLUSTRATION_PROMPT =
  'A calm illustration for the Appearance setting: one horizon shared by a low sun on the left and a crescent moon on the right, soft gradient sky from warm day to deep night, minimal, no text.';

/** The "generated" illustration, painted with a canvas at runtime (PNG bytes). */
export async function illustrationPng(w = 960, h = 600): Promise<ArrayBuffer> {
  const canvas = typeof OffscreenCanvas !== 'undefined' ? new OffscreenCanvas(w, h) : Object.assign(document.createElement('canvas'), { width: w, height: h });
  const g = canvas.getContext('2d') as CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null;
  if (!g) return new ArrayBuffer(0);
  // Sky: day (left) → night (right).
  const sky = g.createLinearGradient(0, 0, w, 0);
  sky.addColorStop(0, '#f7d9b0');
  sky.addColorStop(0.38, '#e9a98f');
  sky.addColorStop(0.62, '#5b5a8e');
  sky.addColorStop(1, '#141a33');
  g.fillStyle = sky;
  g.fillRect(0, 0, w, h);
  const fade = g.createLinearGradient(0, 0, 0, h);
  fade.addColorStop(0, 'rgba(255,255,255,0.18)');
  fade.addColorStop(0.7, 'rgba(255,255,255,0)');
  g.fillStyle = fade;
  g.fillRect(0, 0, w, h);
  // Stars on the night side (seeded, so the "generation" is stable).
  let seed = 7;
  const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
  for (let i = 0; i < 70; i++) {
    const x = w * (0.58 + rnd() * 0.42);
    const y = h * rnd() * 0.55;
    g.globalAlpha = 0.25 + rnd() * 0.6 * ((x / w - 0.55) * 2.2);
    g.fillStyle = '#ffffff';
    g.beginPath();
    g.arc(x, y, 0.6 + rnd() * 1.3, 0, Math.PI * 2);
    g.fill();
  }
  g.globalAlpha = 1;
  // Sun.
  const sunX = w * 0.24;
  const sunY = h * 0.56;
  const halo = g.createRadialGradient(sunX, sunY, 10, sunX, sunY, 210);
  halo.addColorStop(0, 'rgba(255,236,200,0.9)');
  halo.addColorStop(1, 'rgba(255,236,200,0)');
  g.fillStyle = halo;
  g.fillRect(0, 0, w, h);
  g.fillStyle = '#fff2d9';
  g.beginPath();
  g.arc(sunX, sunY, 64, 0, Math.PI * 2);
  g.fill();
  // Crescent moon.
  const mx = w * 0.78;
  const my = h * 0.3;
  const glow = g.createRadialGradient(mx, my, 30, mx, my, 120);
  glow.addColorStop(0, 'rgba(244,241,230,0.25)');
  glow.addColorStop(1, 'rgba(244,241,230,0)');
  g.fillStyle = glow;
  g.fillRect(mx - 130, my - 130, 260, 260);
  g.save();
  g.beginPath();
  g.arc(mx, my, 46, 0, Math.PI * 2);
  g.clip();
  g.beginPath();
  g.rect(mx - 50, my - 50, 100, 100);
  g.arc(mx + 20, my - 12, 42, 0, Math.PI * 2);
  g.fillStyle = '#f4f1e6';
  g.fill('evenodd');
  g.restore();
  // Horizon: layered hills.
  const hill = (base: number, amp: number, freq: number, phase: number, fill: string | CanvasGradient) => {
    g.fillStyle = fill;
    g.beginPath();
    g.moveTo(0, h);
    for (let x = 0; x <= w; x += 8) g.lineTo(x, h * base + Math.sin(x * freq + phase) * amp + Math.sin(x * freq * 2.7 + phase * 1.3) * amp * 0.35);
    g.lineTo(w, h);
    g.closePath();
    g.fill();
  };
  const near = g.createLinearGradient(0, 0, w, 0);
  near.addColorStop(0, '#b9785f');
  near.addColorStop(0.5, '#5d4a6b');
  near.addColorStop(1, '#151a2e');
  hill(0.68, 16, 0.006, 0.5, 'rgba(120,86,110,0.55)');
  hill(0.76, 22, 0.009, 2.1, near);
  hill(0.88, 12, 0.013, 4.2, 'rgba(14,16,30,0.85)');
  const blob = 'convertToBlob' in canvas ? await (canvas as OffscreenCanvas).convertToBlob({ type: 'image/png' }) : await new Promise<Blob>((r) => (canvas as HTMLCanvasElement).toBlob((b) => r(b ?? new Blob()), 'image/png'));
  return blob.arrayBuffer();
}
