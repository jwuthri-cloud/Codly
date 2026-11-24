<!-- A room's header: eyebrow, serif title, one line of facts, lenses and actions on the right.
     In a narrow room (a small window with a sheet open) the right side goes under the title,
     so the title keeps its words. A room that passes one fact (a sentence: Changes, Knowledge,
     the Plan room with no plan, the Terminal's folder) lets it wrap to two lines. A room that
     passes several (the Plan room: done count, branch, time) keeps each on one line; a fact that
     is cut shows its whole text as a tooltip, unless the room gave it a tooltip of its own. -->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { cutTip } from '../lib/util/cutTip';
  interface Props {
    eyebrow: string;
    title: string;
    sub?: Snippet;
    right?: Snippet;
  }
  let { eyebrow, title, sub, right }: Props = $props();

  /** Each fact of `facts` that is cut gets its whole text as a tooltip, again when the header's
   *  width or a fact's text changes. A tooltip the room set itself is left as it is. */
  function tipsWhenCut(facts: HTMLElement) {
    const check = () => {
      for (const fact of facts.children) {
        if (!(fact instanceof HTMLElement)) continue;
        if (fact.dataset.tip !== undefined && fact.dataset.cutTip === undefined) continue;
        const tip = cutTip(fact, fact.textContent?.trim() ?? '');
        if (tip) {
          fact.dataset.tip = tip;
          fact.dataset.cutTip = '';
        } else {
          delete fact.dataset.tip;
          delete fact.dataset.cutTip;
        }
      }
    };
    const resized = new ResizeObserver(check);
    resized.observe(facts);
    const changed = new MutationObserver(check);
    changed.observe(facts, { childList: true, characterData: true, subtree: true });
    check();
    return {
      destroy() {
        resized.disconnect();
        changed.disconnect();
      },
    };
  }
</script>

<header class="rh" data-tauri-drag-region>
  <div class="l">
    <p class="eyebrow">{eyebrow}</p>
    <h1 class="serif truncate">{title}</h1>
    {#if sub}<div class="sub" use:tipsWhenCut>{@render sub()}</div>{/if}
  </div>
  {#if right}<div class="r">{@render right()}</div>{/if}
</header>

<style>
  .rh { display: flex; flex-wrap: wrap; align-items: flex-end; gap: 12px 16px; padding: 24px 28px 18px; flex-shrink: 0; }
  .l { flex: 1 1 320px; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  h1 { font-size: var(--d-lg); line-height: 1.05; color: var(--ink-1); }
  .sub { font-size: var(--t-md); color: var(--ink-3); display: flex; align-items: center; gap: 0 8px; flex-wrap: wrap; min-width: 0; }
  /* Several facts: one line each. */
  .sub > :global(*) { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* One fact: two lines at most. */
  .sub > :global(:only-child) { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; white-space: normal; overflow-wrap: anywhere; }
  .r { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }
</style>
