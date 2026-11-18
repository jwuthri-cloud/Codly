/**
 * Voice: talk to one agent out loud. The mic streams to the voice relay (`voice/relay.py`, through
 * `codly serve`'s `/api/voice/ws`), where OpenAI Realtime is the voice and the agent is the brain.
 * The relay's model drives the agent with two tools that run here, where Codly's API and the
 * agent's live state are: `ask_agent` sends a chat message to the agent's thread, and the agent's
 * reply goes back to the model as `agent_reply` when its turn ends. `agent_status` reads its state.
 *
 * Web only (`codly serve`): the desktop window has no relay next to it.
 */
import type { Agent } from '../gen/Agent';
import { isWebApp } from '../api';
import { sendChat } from './actions';
import { store } from './store.svelte';

const SR = 24000;
/** How often a pending `ask_agent` checks whether the agent's turn ended. */
const POLL_MS = 500;
/** The longest agent reply passed to the voice model (it summarizes; the screen shows the rest). */
const REPLY_CAP = 4000;
/** Statuses where the agent waits for someone: its turn is over for now. */
const RESTING = new Set(['idle', 'done', 'failed', 'waiting_approval', 'blocked', 'rate_limited', 'offline']);

export type VoicePhase = 'off' | 'connecting' | 'listening' | 'thinking' | 'speaking' | 'error';
export type VoiceLine = { role: 'user' | 'voice' | 'agent'; text: string };

const WORKLET = `
class Cap extends AudioWorkletProcessor {
  constructor() { super(); this.buf = []; this.n = 0; }
  process(inputs) {
    const ch = inputs[0][0]; if (!ch) return true;
    this.buf.push(new Float32Array(ch)); this.n += ch.length;
    if (this.n >= 960) {
      const out = new Int16Array(this.n); let o = 0;
      for (const b of this.buf) for (let i = 0; i < b.length; i++) out[o++] = Math.max(-1, Math.min(1, b[i])) * 0x7fff;
      this.port.postMessage(out.buffer, [out.buffer]); this.buf = []; this.n = 0;
    }
    return true;
  }
}
registerProcessor("cap", Cap);`;

type Pending = { agentId: string; turnsBefore: number; sentAt: number; timer: ReturnType<typeof setInterval> };

class Voice {
  phase = $state<VoicePhase>('off');
  /** The thread (`agent:<id>`) the session talks to. */
  thread = $state<string | null>(null);
  lines = $state<VoiceLine[]>([]);
  error = $state<string | null>(null);
  model = $state<string | null>(null);

  readonly available = isWebApp();

  private ws: WebSocket | null = null;
  private ctx: AudioContext | null = null;
  private stream: MediaStream | null = null;
  private out: GainNode | null = null;
  private sources: AudioBufferSourceNode[] = [];
  private nextTime = 0;
  private pending: Pending[] = [];

  /** Start talking to the agent of `thread`. A session on another thread is ended first. */
  async start(thread: string): Promise<void> {
    const agent = store.threadAgent(thread);
    if (!agent) return;
    this.stop();
    this.thread = thread;
    this.lines = [];
    this.error = null;
    this.phase = 'connecting';
    try {
      this.ctx = new AudioContext({ sampleRate: SR });
      this.stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true, autoGainControl: true } });
      await this.ctx.audioWorklet.addModule(URL.createObjectURL(new Blob([WORKLET], { type: 'application/javascript' })));
      const node = new AudioWorkletNode(this.ctx, 'cap');
      this.ctx.createMediaStreamSource(this.stream).connect(node);
      node.port.onmessage = (e) => {
        if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(e.data);
      };
      this.out = this.ctx.createGain();
      this.out.connect(this.ctx.destination);
    } catch (e) {
      return this.fail(`Microphone unavailable: ${e instanceof Error ? e.message : String(e)}`);
    }
    const ws = new WebSocket(`${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/api/voice/ws`);
    ws.binaryType = 'arraybuffer';
    this.ws = ws;
    ws.onmessage = (e) => (e.data instanceof ArrayBuffer ? this.play(e.data) : this.onEvent(JSON.parse(e.data), agent));
    ws.onclose = () => {
      if (this.ws !== ws) return;
      if (this.phase !== 'error') this.fail('The voice connection closed.');
    };
  }

  stop(): void {
    const ws = this.ws;
    this.ws = null;
    ws?.close();
    for (const p of this.pending) clearInterval(p.timer);
    this.pending = [];
    this.clearPlayback();
    this.stream?.getTracks().forEach((t) => t.stop());
    this.stream = null;
    void this.ctx?.close();
    this.ctx = null;
    this.phase = 'off';
    this.thread = null;
  }

  /** Stop the voice mid-sentence (it also stops by itself when you start talking). */
  interrupt(): void {
    this.send({ type: 'interrupt' });
    this.clearPlayback();
  }

  private fail(msg: string): void {
    this.stop();
    this.error = msg;
    this.phase = 'error';
  }

  private send(m: unknown): void {
    if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(JSON.stringify(m));
  }

  private say(role: VoiceLine['role'], text: string): void {
    this.lines = [...this.lines, { role, text }].slice(-40);
  }

  private onEvent(m: Record<string, unknown>, agent: Agent): void {
    switch (m.type) {
      case 'ready':
        this.model = String(m.model ?? '');
        this.phase = 'listening';
        this.send({ type: 'context', agent: agent.name, role: agent.role, provider: agent.provider });
        break;
      case 'clear':
        this.clearPlayback();
        break;
      case 'user_start':
        this.phase = 'listening';
        break;
      case 'thinking':
        this.phase = 'thinking';
        break;
      case 'user':
        if (String(m.text ?? '').trim()) this.say('user', String(m.text));
        break;
      case 'assistant_done':
        if (String(m.text ?? '').trim()) this.say('voice', String(m.text));
        break;
      case 'tool_call':
        this.tool(String(m.id), String(m.name), (m.args ?? {}) as Record<string, unknown>, agent.id);
        break;
      case 'error':
        this.error = String(m.text ?? 'Voice error');
        break;
    }
  }

  private tool(id: string, name: string, args: Record<string, unknown>, agentId: string): void {
    const agent = store.agentMap.get(agentId);
    if (name === 'agent_status') {
      const last = store.live[agentId]?.messages.at(-1)?.text ?? null;
      this.send({ type: 'tool_result', id, output: { status: agent?.status ?? 'unknown', latest_message: last && clip(last, 1500) } });
      return;
    }
    if (name === 'ask_agent') {
      const message = String(args.message ?? '').trim();
      if (!message || !this.thread) {
        this.send({ type: 'tool_result', id, output: { error: 'empty message' } });
        return;
      }
      const turnsBefore = store.live[agentId]?.turns.length ?? 0;
      const sentAt = Date.now();
      const ok = sendChat(this.thread, message);
      this.say('agent', `→ ${message}`);
      this.send({ type: 'tool_result', id, output: ok ? { status: 'sent', note: 'The agent is working; its reply comes later.' } : { error: 'Codly refused the message' } });
      if (ok) this.watch(agentId, turnsBefore, sentAt);
      return;
    }
    this.send({ type: 'tool_result', id, output: { error: `unknown tool ${name}` } });
  }

  /** Waits for the agent's turn to end after `ask_agent`, then hands its reply to the voice. */
  private watch(agentId: string, turnsBefore: number, sentAt: number): void {
    const p: Pending = {
      agentId,
      turnsBefore,
      sentAt,
      timer: setInterval(() => {
        const live = store.live[agentId];
        const status = store.agentMap.get(agentId)?.status ?? 'offline';
        const turns = live?.turns ?? [];
        const ended = turns.length > p.turnsBefore && turns.at(-1)?.end != null;
        if (!ended || !RESTING.has(status)) return;
        clearInterval(p.timer);
        this.pending = this.pending.filter((x) => x !== p);
        const msgs = (live?.messages ?? []).filter((m) => m.ts >= p.sentAt);
        const finals = msgs.filter((m) => m.phase === 'final');
        const text = (finals.length ? finals : msgs.slice(-1)).map((m) => m.text).join('\n\n').trim();
        const head = status === 'waiting_approval' ? 'The agent is waiting for an approval in Codly.\n\n' : status === 'failed' ? 'The agent failed.\n\n' : '';
        this.send({ type: 'agent_reply', text: head + (text ? clip(text, REPLY_CAP) : '(no message)') });
      }, POLL_MS),
    };
    this.pending.push(p);
  }

  private play(buf: ArrayBuffer): void {
    const ctx = this.ctx;
    if (!ctx || !this.out) return;
    const pcm = new Int16Array(buf);
    const f = new Float32Array(pcm.length);
    for (let i = 0; i < pcm.length; i++) f[i] = pcm[i] / 0x8000;
    const ab = ctx.createBuffer(1, f.length, SR);
    ab.copyToChannel(f, 0);
    const src = ctx.createBufferSource();
    src.buffer = ab;
    src.connect(this.out);
    this.nextTime = Math.max(this.nextTime, ctx.currentTime + 0.04);
    src.start(this.nextTime);
    this.nextTime += ab.duration;
    this.sources.push(src);
    this.phase = 'speaking';
    src.onended = () => {
      this.sources = this.sources.filter((s) => s !== src);
      if (!this.sources.length && this.phase === 'speaking') this.phase = 'listening';
    };
  }

  private clearPlayback(): void {
    for (const s of this.sources) {
      try {
        s.stop();
      } catch {
        /* already stopped */
      }
    }
    this.sources = [];
    this.nextTime = 0;
  }
}

const clip = (s: string, n: number) => (s.length > n ? `${s.slice(0, n)}…` : s);

export const voice = new Voice();
