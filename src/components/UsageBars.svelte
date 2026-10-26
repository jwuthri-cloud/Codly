<!--
  An account's quota windows as quiet bars: "5-hour ▬▬▬── 62% · resets 14:00". Neutral until
  80% used (amber), red once the CLI says it is out. Values are what its agents last reported.
-->
<script lang="ts">
  import type { UsageWindow } from '../lib/gen/UsageWindow';
  import { resetLabel, sortWindows, toMs, usedNow, windowLabel } from '../lib/util/accounts';
  import { pct, resetsIn } from '../lib/util/format';

  interface Props {
    limits: UsageWindow[];
    now: number;
  }
  let { limits, now }: Props = $props();

  const rows = $derived(sortWindows(limits).map((w) => {
    const used = usedNow(w, now);
    const out = w.status === 'rejected' && used > 0;
    return { w, used, tone: out || used >= 0.98 ? 'out' : used >= 0.8 ? 'near' : 'ok' };
  }));
</script>

<ul class="ub">
  {#each rows as r (r.w.window)}
    <li class="t-{r.tone}">
      <span class="wl">{windowLabel(r.w.window)}</span>
      <span class="bar" role="meter" aria-label="{windowLabel(r.w.window)} window used" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(r.used * 100)}><i style:width="{Math.max(r.used > 0 ? 2 : 0, r.used * 100)}%"></i></span>
      <span class="p num">{pct(r.used)}</span>
      <span class="rs num" data-tip={r.w.resets_at != null && toMs(r.w.resets_at) > now ? resetsIn(toMs(r.w.resets_at), now) : undefined}>{resetLabel(r.w.resets_at, now)}</span>
    </li>
  {/each}
</ul>

<style>
  .ub { display: flex; flex-direction: column; gap: 5px; }
  li { display: grid; grid-template-columns: 92px minmax(60px, 1fr) 38px minmax(96px, auto); align-items: center; column-gap: 10px; font-size: var(--t-sm); line-height: 16px; color: var(--ink-3); }
  .wl { color: var(--ink-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .bar { height: 4px; border-radius: 2px; background: var(--surface-3); overflow: hidden; }
  .bar i { display: block; height: 100%; border-radius: 2px; background: var(--ink-3); transition: width var(--d-4) var(--ease-out); }
  .p { text-align: right; color: var(--ink-2); }
  .rs { white-space: nowrap; }
  .t-near .bar i { background: var(--amber); }
  .t-near .p { color: var(--amber-ink); }
  .t-out .bar i { background: var(--bad); }
  .t-out .p { color: var(--bad); }
</style>
