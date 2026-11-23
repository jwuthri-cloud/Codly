<!--
  What the agent's CLI told about the run (a hook's word, a goal, an MCP server that did not
  start), as one quiet line. One notice: a click opens the agent's history there. Several (all
  of one turn, or the same words told again): the newest one, with how many there are; a click
  lists them all with their times, and each opens the history at its event.
-->
<script lang="ts">
  import { ui } from '../../lib/state/ui.svelte';
  import { clock } from '../../lib/util/format';
  import { clean } from '../../lib/util/sanitize';
  import Icon from '../../components/Icon.svelte';
  import { moreOf, type Note } from '../notices';

  interface Props {
    agentId: string;
    /** Oldest first, one or more. */
    notes: Note[];
  }
  let { agentId, notes }: Props = $props();
  let open = $state(false);
  const newest = $derived(notes[notes.length - 1]);
  const more = $derived(moreOf(notes));
</script>

{#if more == null}
  <button class="line" onclick={() => ui.selectAgent(agentId, newest.eventId)}><span class="text">{clean(newest.text)}</span><time class="num">{clock(newest.ts)}</time></button>
{:else}
  <button class="line" onclick={() => (open = !open)} aria-expanded={open}>
    <span class="text">{clean(newest.text)}</span><span class="more num">{more}</span><time class="num">{clock(newest.ts)}</time>
    <span class="tri" class:open><Icon name="chevron-right" size={11} stroke={2} /></span>
  </button>
  {#if open}
    <ol class="all">
      {#each notes as n (n.eventId)}
        <li><button class="one" onclick={() => ui.selectAgent(agentId, n.eventId)}><time class="num">{clock(n.ts)}</time><span class="text">{clean(n.text)}</span></button></li>
      {/each}
    </ol>
  {/if}
{/if}

<style>
  .line { display: flex; justify-content: center; align-items: baseline; gap: 8px; width: 100%; text-align: center; font-size: var(--t-sm); color: var(--ink-3); }
  .line:hover { color: var(--ink-2); }
  .text { min-width: 0; overflow-wrap: anywhere; }
  time { white-space: nowrap; color: var(--ink-4); }
  .more { white-space: nowrap; color: var(--ink-2); }
  .tri { display: inline-grid; align-self: center; color: var(--ink-4); transition: transform var(--d-2) var(--ease-out); }
  .tri.open { transform: rotate(90deg); }
  .all { margin: 6px auto 0; max-width: 66ch; display: flex; flex-direction: column; gap: 1px; animation: fade-in var(--d-2) var(--ease-out); }
  .one { display: flex; align-items: baseline; gap: 9px; width: 100%; padding: 4px 8px; border-radius: var(--r-sm); font-size: var(--t-sm); color: var(--ink-3); text-align: left; }
  .one:hover { background: var(--surface-1); color: var(--ink-2); }
</style>
