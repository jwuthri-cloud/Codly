<!--
  The call strip above the composer: the call's state and its last line. Click it to open the full
  transcript (you, the voice, lookups, messages sent to agents and their replies). It sits in the
  page flow, so the row above the composer (New request, suggestions) moves up instead of hiding
  under it.
-->
<script lang="ts">
  import { voice } from '../lib/state/voice.svelte';

  interface Props {
    thread: string;
  }
  let { thread }: Props = $props();

  const here = $derived(voice.thread === thread);
  const live = $derived(here && voice.phase !== 'off' && voice.phase !== 'error');
  const STATE: Record<string, string> = { connecting: 'Connecting…', listening: 'Listening', thinking: 'Thinking…', speaking: 'Speaking' };
  const WHO: Record<string, string> = { user: 'You', voice: 'Voice', agent: 'Agent', lookup: 'Lookup' };
  let open = $state(false);
  let list = $state<HTMLElement>();
  $effect(() => {
    void voice.lines.length;
    if (open && list) list.scrollTop = list.scrollHeight;
  });
</script>

{#if voice.available && (live || (voice.phase === 'error' && voice.error))}
  <div class="call">
    {#if open && live}
      <div class="log" bind:this={list} role="log" aria-label="Voice transcript">
        {#each voice.lines as l, i (i)}
          <p class="line {l.role}"><b>{WHO[l.role]}</b>{l.text}</p>
        {:else}
          <p class="line empty">Nothing said yet.</p>
        {/each}
      </div>
    {/if}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="strip" class:err={voice.phase === 'error'} class:open role="status" aria-live="polite" onclick={() => (open = !open)} data-tip={open ? 'Hide the transcript' : 'Show the full transcript'}>
      {#if voice.phase === 'error'}
        <span class="state">{voice.error}</span>
        <button class="link" onclick={(e) => { e.stopPropagation(); voice.error = null; voice.phase = 'off'; }}>Dismiss</button>
      {:else}
        <span class="dot {voice.phase}"></span>
        <span class="state">{STATE[voice.phase] ?? voice.phase}</span>
        {#if voice.lines.length}<span class="last">{voice.lines.at(-1)?.text}</span>{/if}
        {#if voice.phase === 'speaking'}<button class="link" onclick={(e) => { e.stopPropagation(); voice.interrupt(); }}>Stop talking</button>{/if}
        {#if voice.error}<span class="warn">{voice.error}</span>{/if}
      {/if}
    </div>
  </div>
{/if}

<style>
  .call { position: relative; margin-bottom: 6px; }
  .strip { display: flex; align-items: center; gap: 8px; padding: 6px 12px; border-radius: var(--r-lg); background: var(--surface-1); box-shadow: inset 0 0 0 1px var(--line); font-size: var(--t-sm); color: var(--ink-2); min-width: 0; cursor: pointer; }
  .strip.err { color: var(--ink-1); }
  .log { position: absolute; left: 0; right: 0; bottom: calc(100% + 6px); max-height: min(50vh, 420px); overflow-y: auto; display: flex; flex-direction: column; gap: 8px; padding: 12px 14px; border-radius: var(--r-lg); background: var(--surface-1); box-shadow: inset 0 0 0 1px var(--line), 0 8px 24px rgb(0 0 0 / 0.18); font-size: var(--t-sm); line-height: var(--lh-sm); z-index: 2; }
  .line { color: var(--ink-2); white-space: pre-wrap; overflow-wrap: anywhere; }
  .line b { display: inline-block; min-width: 46px; margin-right: 6px; font-weight: var(--w-med); color: var(--ink-1); }
  .line.agent, .line.lookup { color: var(--ink-3); }
  .line.empty { color: var(--ink-3); }
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
