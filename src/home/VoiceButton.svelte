<!--
  Voice: talk to this agent out loud (`codly serve` only, with the voice relay running next to it).
  Click to start a call, click again to hang up. While a call runs, `VoiceStrip` above the composer
  shows its state and transcript; the agent's real messages still land in the thread.
-->
<script lang="ts">
  import { voice } from '../lib/state/voice.svelte';

  interface Props {
    thread: string;
  }
  let { thread }: Props = $props();

  const here = $derived(voice.thread === thread);
  const live = $derived(here && voice.phase !== 'off' && voice.phase !== 'error');
  const label = $derived(live ? 'Hang up' : 'Talk out loud');

  function toggle() {
    if (live) voice.stop();
    else void voice.start(thread);
  }
</script>

{#if voice.available}
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
</style>
