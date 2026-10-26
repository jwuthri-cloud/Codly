<!-- The five-state vocabulary as small shapes: shape AND colour, never colour alone.
     live ● · wait ◆ · blocked ✕ · idle ○ · done ✓ -->
<script lang="ts">
  import type { Tone } from '../lib/narrate';
  interface Props {
    tone: Tone;
    size?: number;
    label?: string;
  }
  let { tone, size = 10, label }: Props = $props();
</script>

<svg class="mark m-{tone}" width={size} height={size} viewBox="0 0 10 10" role={label ? 'img' : undefined} aria-label={label} aria-hidden={label ? undefined : 'true'}>
  {#if tone === 'live'}<circle cx="5" cy="5" r="3.6" fill="currentColor" />
  {:else if tone === 'wait'}<path d="M5 0.8L9.2 5 5 9.2 0.8 5z" fill="currentColor" />
  {:else if tone === 'blocked'}<path d="M2 2l6 6M8 2L2 8" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
  {:else if tone === 'done'}<path d="M1.6 5.4l2.3 2.3L8.6 2.6" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
  {:else}<circle cx="5" cy="5" r="3.2" fill="none" stroke="currentColor" stroke-width="1.4" />{/if}
</svg>

<style>
  .mark { flex-shrink: 0; display: block; }
  .m-live { color: var(--live); }
  .m-wait { color: var(--amber); }
  .m-blocked { color: var(--bad); }
  .m-done { color: var(--ok); }
  .m-idle, .m-off { color: var(--ink-3); }
</style>
