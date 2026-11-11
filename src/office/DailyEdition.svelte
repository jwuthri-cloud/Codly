<!--
  "The Daily Commit": the office's morning paper, once a day, from the core's digest of what
  really happened since the last edition (daily.ts). The standup runs while it is up.
-->
<script lang="ts">
  import type { Edition } from './daily';

  interface Props {
    paper: Edition;
    /** Agent id → name (agents let go keep their name here). */
    nameOf: (id: string) => string;
    /** Who sat down at the meeting table for the standup. */
    standup: string[];
    top: number;
    onclose: () => void;
  }
  let { paper, nameOf, standup, top, onclose }: Props = $props();

  const date = new Date().toLocaleDateString(undefined, { weekday: 'long', month: 'long', day: 'numeric' });
</script>

<div class="paper" style:top="{top + 12}px" role="dialog" aria-label="The Daily Commit">
  <button class="x" aria-label="Close the paper" onclick={onclose}>✕</button>
  <header>
    <div class="mast">The Daily Commit</div>
    <div class="date">{date}</div>
  </header>
  <h2>{paper.headline}</h2>
  {#if paper.mvp}
    <p class="mvp">⭐ <b>{nameOf(paper.mvp.id)}</b> is yesterday’s MVP with <b>+{paper.mvp.xp} XP</b></p>
  {/if}
  {#if paper.notes.length}
    <ul class="notes">
      {#each paper.notes.slice(0, 5) as n, i (i)}<li><b>{nameOf(n.id)}</b>: {n.text}</li>{/each}
    </ul>
  {/if}
  {#if paper.shipped.length}
    <div class="col">
      <div class="label">Shipped</div>
      <ul>{#each paper.shipped.slice(0, 5) as t, i (i)}<li>{t}</li>{/each}</ul>
      {#if paper.shipped.length > 5}<div class="more">and {paper.shipped.length - 5} more</div>{/if}
    </div>
  {/if}
  {#if standup.length}
    <p class="standup">☕ Morning standup in the meeting room: {standup.map(nameOf).join(', ')}</p>
  {/if}
</div>

<style>
  /* Newsprint: the paper keeps its own three colours, the same in both themes. */
  .paper { --paper: #f6f0e2; --paper-ink: #2a2419; --paper-dim: #6b604c; }
  .paper { position: absolute; left: 50%; transform: translateX(-50%); z-index: 410; width: min(380px, calc(100% - 32px)); max-height: calc(100% - 120px); overflow: auto; padding: 14px 16px 12px; border-radius: 14px; background: var(--paper); color: var(--paper-ink); box-shadow: var(--pop-shadow); font: 400 12.5px/1.45 var(--of-font); pointer-events: auto; animation: drop 0.3s ease-out both; }
  header { text-align: center; border-bottom: 2px solid var(--paper-ink); padding-bottom: 6px; margin-bottom: 8px; }
  .mast { font: 700 22px/1.1 'Instrument Serif', 'Times New Roman', Georgia, serif; letter-spacing: 0.01em; }
  .date { margin-top: 2px; color: var(--paper-dim); font-size: 10.5px; text-transform: uppercase; letter-spacing: 0.08em; }
  h2 { margin: 0 0 6px; font: 600 18px/1.2 'Instrument Serif', Georgia, serif; }
  .mvp { margin: 0 0 8px; }
  .notes, .col ul { margin: 0 0 8px; padding-left: 16px; }
  .notes li, .col li { margin: 1px 0; }
  .label { color: var(--paper-dim); font-size: 10.5px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.08em; }
  .more { color: var(--paper-dim); font-size: 11px; }
  .standup { margin: 6px 0 0; padding-top: 8px; border-top: 1px dashed color-mix(in srgb, var(--paper-ink) 30%, transparent); color: color-mix(in srgb, var(--paper-ink) 84%, var(--paper)); }
  .x { position: absolute; top: 8px; right: 8px; border: 0; background: none; color: var(--paper-dim); font-size: 13px; cursor: pointer; }
  @keyframes drop { from { opacity: 0; transform: translate(-50%, -8px); } }
  @media (prefers-reduced-motion: reduce) { .paper { animation: none; } }
</style>
