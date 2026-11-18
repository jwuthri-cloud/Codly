<!--
  Voice: talk to this agent out loud (`codly serve` only, with the voice relay running next to it).
  Click to start a call, click again to hang up. While a call runs, a strip above the composer
  shows its state and the last lines; the agent's real messages still land in the thread.
-->
<script lang="ts">
  import { voice } from '../lib/state/voice.svelte';

  interface Props {
    /** The 1:1 thread (`agent:<id>`). */
    thread: string;
  }
  let { thread }: Props = $props();

  const here = $derived(voice.thread === thread);
  const live = $derived(here && voice.phase !== 'off' && voice.phase !== 'error');
  const label = $derived(live ? 'Hang up' : 'Talk to this agent');
  const STATE: Record<string, string> = { connecting: 'Connecting…', listening: 'Listening', thinking: 'Thinking…', speaking: 'Speaking' };

  function toggle() {
    if (live) voice.stop();
    else void voice.start(thread);
  }
</script>

{#if voice.available}
  {#if live || (voice.phase === 'error' && voice.error)}
    <div class="strip" class:err={voice.phase === 'error'} role="status" aria-live="polite">
      {#if voice.phase === 'error'}
        <span class="state">{voice.error}</span>
        <button class="link" onclick={() => (voice.error = null, voice.phase = 'off')}>Dismiss</button>
      {:else}
        <span class="dot {voice.phase}"></span>
        <span class="state">{STATE[voice.phase] ?? voice.phase}</span>
        {#if voice.lines.length}<span class="last">{voice.lines.at(-1)?.text}</span>{/if}
        {#if voice.phase === 'speaking'}<button class="link" onclick={() => voice.interrupt()}>Stop talking</button>{/if}
        {#if voice.error}<span class="warn">{voice.error}</span>{/if}
      {/if}
    </div>
  {/if}
  <button class="voice" class:on={live} onclick={toggle} aria-label={label} data-tip={label} aria-pressed={live}>
    <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      {#if live}
        <path d="M3 5.5c0-1 .8-1.8 1.8-1.8h2.4l1.6 4-2 1.3a11 11 0 0 0 6.2 6.2l1.3-2 4 1.6v2.4c0 1-.8 1.8-1.8 1.8A16.7 16.7 0 0 1 3 5.5Z" transform="rotate(135 12 12)" />
      {:else}
        <path d="M4 12h1M8 8v8M12 5v14M16 8v8M20 12h-1" />
      {/if}
    </svg>
  </button>
{/if}

<style>
  .voice { flex: none; width: 30px; height: 30px; border-radius: var(--r-full); display: grid; place-items: center; color: var(--ink-2); }
  .voice:hover { background: var(--surface-2); color: var(--ink-1); }
  .voice.on { background: var(--danger, #d4483b); color: white; }
  .strip { position: absolute; left: 0; right: 0; bottom: calc(100% + 6px); display: flex; align-items: center; gap: 8px; padding: 6px 12px; border-radius: var(--r-lg); background: var(--surface-1); box-shadow: inset 0 0 0 1px var(--line); font-size: var(--t-sm); color: var(--ink-2); min-width: 0; }
  .strip.err { color: var(--ink-1); }
  .state { flex: none; font-weight: var(--w-med); color: var(--ink-1); }
  .last { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--ink-3); }
  .warn { color: var(--danger, #d4483b); }
  .link { flex: none; color: var(--ink-2); text-decoration: underline; font-size: var(--t-sm); }
  .dot { flex: none; width: 8px; height: 8px; border-radius: 50%; background: var(--ink-3); }
  .dot.listening { background: #3fb27f; }
  .dot.speaking { background: #4c8bf5; animation: pulse 1s infinite; }
  .dot.thinking, .dot.connecting { background: #e0a43a; animation: pulse 1.4s infinite; }
  @keyframes pulse { 50% { opacity: 0.35; } }
</style>
