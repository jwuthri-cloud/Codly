<!--
  An agent's scrapbook: every milestone of its career (career.rs), newest first, by day.
  Shown in its panel's Career tab and for a ghost by the memory archive. With `highlights`, only
  the big moments (promotions, firsts, mentors, trust) show at first, the rest a click away.
-->
<script lang="ts">
  import type { Milestone } from '../lib/gen/Milestone';

  let { milestones, empty = 'Its first commit, merges and promotions will show up here.', highlights = false }: { milestones: Milestone[]; empty?: string; highlights?: boolean } = $props();

  const BIG = /^(Promoted|First |Legend|Mentored by|Took .* under its wing|Trusted as|Joined)/;
  const HIGHLIGHTS = 5;
  let all = $state(false);
  const big = $derived(milestones.filter((m) => BIG.test(m.text)).slice(-HIGHLIGHTS).reverse());
  const short = $derived(highlights && !all);

  const icon = (t: string) =>
    t.startsWith('Promoted') ? '⭐' : t.includes('merge') ? '🎉' : t.includes('commit') ? '🔨' : t === 'Came back' ? '👻' : t.startsWith('Moved') ? '🧳' : t.startsWith('Joined') ? '👋' : '•';
  const days = $derived.by(() => {
    const out: { day: string; items: Milestone[] }[] = [];
    for (const m of [...milestones].reverse()) {
      const day = new Date(Number(m.at)).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });
      if (out.at(-1)?.day === day) out.at(-1)!.items.push(m);
      else out.push({ day, items: [m] });
    }
    return out;
  });
</script>

{#if short && big.length}
  <ol class="book">
    {#each big as m (m.at + m.text)}
      <li><div class="page"><span class="ic" aria-hidden="true">{icon(m.text)}</span><b>{m.text}</b><span class="t">{new Date(Number(m.at)).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}</span></div></li>
    {/each}
  </ol>
  {#if milestones.length > big.length}<button class="more" onclick={() => (all = true)}>All {milestones.length} moments</button>{/if}
{:else if days.length}
  {#if highlights && all}<button class="more" onclick={() => (all = false)}>Highlights only</button>{/if}
  <ol class="book">
    {#each days as d (d.day)}
      <li>
        <span class="day">{d.day}</span>
        {#each d.items as m (m.at + m.text)}
          <div class="page"><span class="ic" aria-hidden="true">{icon(m.text)}</span><b>{m.text}</b><span class="t">{new Date(Number(m.at)).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })}</span></div>
        {/each}
      </li>
    {/each}
  </ol>
{:else}
  <p class="empty">{empty}</p>
{/if}

<style>
  .book { list-style: none; margin: 0; padding: 0 0 0 10px; max-height: 190px; overflow-y: auto; border-left: 2px solid rgba(255, 255, 255, 0.12); }
  .day { display: block; margin: 6px 0 3px -10px; padding-left: 10px; font: 700 10px/1 ui-monospace, monospace; letter-spacing: 0.06em; text-transform: uppercase; opacity: 0.55; }
  .page { display: grid; grid-template-columns: 18px 1fr auto; gap: 6px; align-items: center; padding: 3px 0; font-size: 12px; }
  .page b { font-weight: 600; }
  .t { font-size: 11px; opacity: 0.5; }
  .empty { margin: 4px 0; font-size: 12px; opacity: 0.6; }
  .more { margin: 4px 0 0; padding: 0; border: 0; background: none; color: #9aa2b6; font: 500 11px inherit; text-decoration: underline; cursor: pointer; }
</style>
