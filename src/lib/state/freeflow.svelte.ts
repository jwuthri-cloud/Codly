// Free Flow: record in the webview, transcribe in the core, hand the text to a composer.
// One recording at a time, app-wide. Audio is captured only between start() and stop(), and
// goes nowhere until stop() hands it to the core (which sends it to Groq and nowhere else).
import { api, isTauriApp } from '../api';
import type { DictationStatus } from '../gen/DictationStatus';
import { MAX_RECORD_MS, pickMime, toBase64 } from '../util/freeflow';
import { toast } from './toasts.svelte';

type Phase = 'idle' | 'starting' | 'recording' | 'transcribing';

class FreeFlow {
  status = $state<DictationStatus | null>(null);
  phase = $state<Phase>('idle');
  /** Seconds recorded so far, for the button's label. */
  seconds = $state(0);

  private rec: MediaRecorder | null = null;
  private stream: MediaStream | null = null;
  private chunks: Blob[] = [];
  private tick: ReturnType<typeof setInterval> | null = null;
  private limit: ReturnType<typeof setTimeout> | null = null;
  private onText: ((t: string) => void) | null = null;
  private discard = false;
  /** The browser demo has no real microphone: it plays a short pretend recording. */
  private fake = false;
  /** Stop/cancel arrived while the permission prompt was still open: don't record at all. */
  private abortStart = false;

  /** Free Flow is on and a key is saved: the mic can be offered. */
  get ready(): boolean {
    return !!this.status?.enabled && !!this.status.key_saved;
  }

  async load(): Promise<void> {
    try {
      this.status = await api.dictationStatus();
    } catch {
      this.status = null; // no backend: the mic stays hidden
    }
  }

  set(status: DictationStatus) {
    this.status = status;
  }

  async start(onText: (t: string) => void): Promise<void> {
    if (this.phase !== 'idle') return;
    if (!this.ready) {
      toast({ kind: 'info', title: 'Free Flow is off', body: 'Turn it on and add your Groq key in Settings → Free Flow.' });
      return;
    }
    this.onText = onText;
    this.discard = false;
    this.chunks = [];
    this.fake = !isTauriApp();
    this.abortStart = false;
    this.phase = 'starting';
    try {
      if (this.fake) {
        this.mark();
        return;
      }
      const mime = pickMime((m) => MediaRecorder.isTypeSupported(m));
      if (!mime) throw new Error('This webview cannot record audio.');
      this.stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      // Stopped before the permission prompt was answered: let go of the mic again.
      if (this.abortStart) {
        this.release();
        this.phase = 'idle';
        return;
      }
      this.rec = new MediaRecorder(this.stream, { mimeType: mime });
      this.rec.ondataavailable = (e) => e.data.size && this.chunks.push(e.data);
      this.rec.onstop = () => void this.finish(mime);
      this.rec.start(1000);
      this.mark();
    } catch (e) {
      this.release();
      this.phase = 'idle';
      const denied = e instanceof DOMException && (e.name === 'NotAllowedError' || e.name === 'SecurityError');
      toast({
        kind: 'error',
        title: denied ? 'Microphone blocked' : 'Could not start recording',
        body: denied ? 'Allow Codly under System Settings → Privacy & Security → Microphone.' : String((e as Error)?.message ?? e),
      });
    }
  }

  private mark() {
    this.phase = 'recording';
    this.seconds = 0;
    this.tick = setInterval(() => this.seconds++, 1000);
    this.limit = setTimeout(() => this.stop(), MAX_RECORD_MS);
  }

  /** Stop and transcribe. */
  stop() {
    if (this.phase === 'starting') this.abortStart = true;
    if (this.phase !== 'recording') return;
    this.halt();
    if (this.fake) void this.finish('audio/webm');
    else if (this.rec && this.rec.state !== 'inactive') this.rec.stop();
    else void this.finish('audio/webm');
  }

  /** Stop and throw the clip away (Escape, focus lost, another key during a hold). */
  cancel() {
    if (this.phase === 'starting') this.abortStart = true;
    if (this.phase !== 'recording') return;
    this.discard = true;
    this.stop();
  }

  private halt() {
    if (this.tick) clearInterval(this.tick);
    if (this.limit) clearTimeout(this.limit);
    this.tick = this.limit = null;
    this.phase = 'transcribing';
  }

  private release() {
    this.stream?.getTracks().forEach((t) => t.stop()); // the OS mic indicator goes off here
    this.stream = null;
    this.rec = null;
  }

  private async finish(mime: string) {
    this.release();
    const onText = this.onText;
    this.onText = null;
    try {
      if (this.discard) return;
      const bytes = this.fake ? new Uint8Array(2048) : new Uint8Array(await new Blob(this.chunks, { type: mime }).arrayBuffer());
      this.chunks = [];
      if (bytes.length < 1500 && !this.fake) return; // a bare click: nothing was said
      const text = await api.dictationTranscribe(toBase64(bytes), mime);
      if (text) onText?.(text);
      else toast({ kind: 'info', title: 'Nothing heard', duration: 3000 });
    } catch (e) {
      toast({ kind: 'error', title: 'Dictation failed', body: String((e as Error)?.message ?? e) });
    } finally {
      this.phase = 'idle';
    }
  }
}

export const freeflow = new FreeFlow();
