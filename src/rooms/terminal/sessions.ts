// Terminal sessions outlive the panel: switching tabs detaches the xterm element and drops the
// WebGL renderer (GPU contexts are a scarce lease); coming back re-attaches it.
import type { FitAddon } from '@xterm/addon-fit';
import type { WebglAddon } from '@xterm/addon-webgl';
import type { ITheme, Terminal } from '@xterm/xterm';
import { api } from '../../lib/api';
import { tokenHex } from '../../lib/util/color';

export type Session = {
  key: string;
  cwd: string | null;
  label: string;
  ptyId: string;
  term: Terminal;
  fit: FitAddon;
  el: HTMLDivElement;
  webgl: WebglAddon | null;
  exited: boolean;
};

export const sessions = new Map<string, Session>();

export function xtermTheme(): ITheme {
  const t = (n: string) => tokenHex(n);
  const dark = document.documentElement.dataset.theme !== 'light';
  return {
    background: t('--glass-solid'),
    foreground: t('--ink-1'),
    cursor: t('--live'),
    cursorAccent: t('--glass-solid'),
    selectionBackground: dark ? '#62b6f744' : '#2f7fd633',
    black: dark ? '#262c3a' : '#3b3e47',
    red: t('--bad'),
    green: t('--ok'),
    yellow: t('--amber'),
    blue: t('--aurora-m'),
    magenta: t('--id-8'),
    cyan: t('--aurora-a'),
    white: t('--ink-2'),
    brightBlack: t('--ink-3'),
    brightRed: t('--bad'),
    brightGreen: t('--ok'),
    brightYellow: t('--amber-ink'),
    brightBlue: t('--focus'),
    brightMagenta: t('--aurora-b'),
    brightCyan: t('--aurora-a'),
    brightWhite: t('--ink-1'),
  };
}

export async function openSession(cwd: string | null, label: string): Promise<Session> {
  const key = cwd ?? '(repo)';
  const existing = sessions.get(key);
  if (existing && !existing.exited) return existing;

  const [{ Terminal }, { FitAddon }] = await Promise.all([import('@xterm/xterm'), import('@xterm/addon-fit')]);
  const term = new Terminal({
    fontFamily: '"JetBrains Mono Variable", ui-monospace, "SF Mono", Menlo, monospace',
    fontSize: 13,
    lineHeight: 1.25,
    cursorBlink: false,
    cursorStyle: 'bar',
    allowProposedApi: false,
    scrollback: 5000,
    theme: xtermTheme(),
    macOptionIsMeta: true,
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  const el = document.createElement('div');
  el.className = 'xterm-host';
  el.style.height = '100%';
  el.style.width = '100%';
  term.open(el);

  const ptyId = await api.ptyOpen(cwd);
  const s: Session = { key, cwd, label, ptyId, term, fit, el, webgl: null, exited: false };
  sessions.set(key, s);
  await api.ptyAttach(ptyId, (bytes) => term.write(bytes));
  // Keystrokes must reach the PTY in order: one write in flight at a time, the rest coalesced.
  let pending = '';
  let inFlight = false;
  const flush = async () => {
    if (inFlight || !pending || s.exited) return;
    inFlight = true;
    const data = pending;
    pending = '';
    try {
      await api.ptyWrite(ptyId, data);
    } catch {
      s.exited = true;
      term.write('\r\n\x1b[2m[process exited]\x1b[0m\r\n');
    } finally {
      inFlight = false;
      if (pending) flush();
    }
  };
  term.onData((d) => {
    pending += d;
    flush();
  });
  term.textarea?.addEventListener('focus', () => (term.options.cursorBlink = true));
  term.textarea?.addEventListener('blur', () => (term.options.cursorBlink = false));
  return s;
}

export async function enableWebgl(s: Session) {
  if (s.webgl) return;
  try {
    const { WebglAddon } = await import('@xterm/addon-webgl');
    const w = new WebglAddon();
    w.onContextLoss(() => {
      w.dispose();
      s.webgl = null;
    });
    s.term.loadAddon(w);
    s.webgl = w;
  } catch {
    s.webgl = null; // DOM renderer fallback
  }
}

export function disableWebgl(s: Session) {
  s.webgl?.dispose();
  s.webgl = null;
}

export async function closeSession(key: string) {
  const s = sessions.get(key);
  if (!s) return;
  sessions.delete(key);
  disableWebgl(s);
  s.term.dispose();
  await api.ptyClose(s.ptyId).catch(() => {});
}
