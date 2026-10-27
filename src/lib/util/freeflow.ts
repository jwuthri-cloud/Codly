// Free Flow helpers that need no DOM: how a transcript joins the draft, the hold-Option
// gesture, and the clip limits. The recorder and Tauri calls live in state/freeflow.svelte.ts.

/** A recording stops by itself after this long: Whisper does not need more, and the clip stays small. */
export const MAX_RECORD_MS = 120_000;
/** Option must be held this long, alone, before recording starts. */
export const HOLD_MS = 320;

/** Add a transcript to the draft: after what is there, one space apart, never replacing it. */
export function appendTranscript(draft: string, transcript: string): string {
  const t = transcript.trim();
  if (!t) return draft;
  if (!draft.trim()) return t;
  return /\s$/.test(draft) ? draft + t : `${draft} ${t}`;
}

/** The first container the webview can record (Safari/WKWebView: mp4; Chromium: webm). */
export function pickMime(supports: (mime: string) => boolean): string | null {
  return ['audio/webm;codecs=opus', 'audio/webm', 'audio/mp4', 'audio/ogg;codecs=opus'].find(supports) ?? null;
}

/** Bytes → base64 for the IPC call, in slices so a long clip does not overflow the call stack. */
export function toBase64(bytes: Uint8Array): string {
  let bin = '';
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(bin);
}

/**
 * Hold-Option-to-talk. Feed it the composer's key events: a solo Option press arms a timer; if
 * it survives `HOLD_MS` (no other key, no release) `onStart` fires, and releasing Option fires
 * `onStop`. Any other key before the timer cancels quietly (it was Option+letter, a shortcut or
 * an accented character); any other key after it calls `onCancel`. It never prevents default.
 */
export class HoldOption {
  private timer: ReturnType<typeof setTimeout> | null = null;
  private active = false;

  constructor(
    private cb: { onStart: () => void; onStop: () => void; onCancel: () => void },
    private holdMs = HOLD_MS,
  ) {}

  private isOption = (e: { code: string }) => e.code === 'AltLeft' || e.code === 'AltRight';

  keydown(e: { code: string; repeat?: boolean; metaKey?: boolean; ctrlKey?: boolean; shiftKey?: boolean }) {
    if (this.isOption(e)) {
      if (e.repeat || this.timer || this.active) return; // key auto-repeat
      if (e.metaKey || e.ctrlKey || e.shiftKey) return; // not alone
      this.timer = setTimeout(() => {
        this.timer = null;
        this.active = true;
        this.cb.onStart();
      }, this.holdMs);
      return;
    }
    this.abort();
  }

  keyup(e: { code: string }) {
    if (!this.isOption(e)) return;
    if (this.timer) this.clear(); // tapped, not held
    else if (this.active) {
      this.active = false;
      this.cb.onStop();
    }
  }

  /** Focus left or another key came: drop a pending arm, cancel a running hold. */
  abort() {
    this.clear();
    if (this.active) {
      this.active = false;
      this.cb.onCancel();
    }
  }

  private clear() {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }
}
