<!--
  Free Flow: the composer's mic. Click to record, click again to stop; or hold Option (alone,
  about a third of a second) while the composer is focused and let go to stop. The transcript
  is added to the draft for you to read and send; nothing is ever sent for you. Shown only when
  Free Flow is on and a Groq key is saved (Settings → Free Flow).
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { freeflow } from '../lib/state/freeflow.svelte';
  import { HoldOption } from '../lib/util/freeflow';

  interface Props {
    /** The composer's textarea: Option-hold listens on it. */
    target: HTMLElement | undefined;
    /** The thread the recording started in, so a late transcript lands in the right draft. */
    thread: string;
    onText: (thread: string, text: string) => void;
  }
  let { target, thread, onText }: Props = $props();

  const recording = $derived(freeflow.phase === 'recording');
  const busy = $derived(freeflow.phase === 'transcribing');
  const label = $derived(recording ? 'Stop and transcribe' : busy ? 'Transcribing…' : 'Dictate');

  onMount(() => void freeflow.load());

  function begin() {
    const t = thread;
    return freeflow.start((text) => onText(t, text));
  }

  function toggle() {
    if (recording) freeflow.stop();
    else void begin();
  }

  // Hold-Option-to-talk: only ends a recording that the hold itself started.
  let heldStart = false;
  const hold = new HoldOption({
    onStart: () => {
      if (freeflow.phase !== 'idle' || !freeflow.ready) return;
      heldStart = true;
      void begin();
    },
    onStop: () => {
      if (heldStart) freeflow.stop();
      heldStart = false;
    },
    onCancel: () => {
      if (heldStart) freeflow.cancel();
      heldStart = false;
    },
  });

  $effect(() => {
    const el = target;
    if (!el || !freeflow.ready) return;
    const down = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && recording) freeflow.cancel();
      hold.keydown(e);
    };
    const up = (e: KeyboardEvent) => hold.keyup(e);
    const away = () => hold.abort();
    el.addEventListener('keydown', down);
    el.addEventListener('keyup', up);
    el.addEventListener('blur', away);
    return () => {
      el.removeEventListener('keydown', down);
      el.removeEventListener('keyup', up);
      el.removeEventListener('blur', away);
      hold.abort();
    };
  });
</script>

{#if freeflow.ready}
  <button
    class="mic"
    class:rec={recording}
    class:busy
    disabled={busy}
    onclick={toggle}
    aria-label={label}
    aria-pressed={recording}
    data-tip={recording ? 'Stop and transcribe (Esc discards)' : busy ? 'Transcribing…' : 'Dictate — or hold ⌥ in the message box'}
  >
    {#if recording}
      <span class="secs">{Math.floor(freeflow.seconds / 60)}:{String(freeflow.seconds % 60).padStart(2, '0')}</span>
    {:else}
      <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <rect x="6" y="2" width="4" height="7.5" rx="2" />
        <path d="M3.75 7.75a4.25 4.25 0 0 0 8.5 0M8 12v2.25M5.75 14.25h4.5" />
      </svg>
    {/if}
  </button>
{/if}

<style>
  .mic { min-width: 34px; height: 34px; padding: 0 9px; border-radius: var(--r-full); display: inline-flex; align-items: center; justify-content: center; color: var(--ink-3); background: var(--surface-2); flex-shrink: 0; transition: background var(--d-2), color var(--d-2); }
  .mic:hover:not(:disabled) { color: var(--ink-1); background: var(--surface-3); }
  .mic.busy { opacity: 0.5; cursor: progress; }
  .mic.rec { color: #fff; background: var(--bad); animation: mic-pulse 1.4s ease-in-out infinite; }
  .secs { font-size: var(--t-sm); font-weight: var(--w-semi); font-variant-numeric: tabular-nums; }
  @keyframes mic-pulse { 50% { box-shadow: 0 0 0 5px color-mix(in srgb, var(--bad) 25%, transparent); } }
  @media (prefers-reduced-motion: reduce) { .mic.rec { animation: none; } }
</style>
