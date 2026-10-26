<script lang="ts">
  import type { Billing } from '../lib/gen/Billing';
  import { BILLING_LABEL, BILLING_TIP } from '../lib/util/labels';

  interface Props {
    billing: Billing;
    plan?: string | null;
  }
  let { billing, plan = null }: Props = $props();
</script>

<span class="bb b-{billing}" data-tip={BILLING_TIP[billing]}>
  {BILLING_LABEL[billing]}{#if plan}<span class="plan">· {plan}</span>{/if}
</span>

<style>
  .bb { display: inline-flex; align-items: center; gap: 4px; height: 20px; padding: 0 8px; border-radius: var(--r-full); font-size: var(--t-xs); font-weight: var(--w-med); line-height: 1; white-space: nowrap; color: var(--ink-2); background: var(--surface-2); }
  .plan { color: var(--ink-3); font-weight: var(--w-reg); }
  .b-subscription { color: var(--ok); background: var(--ok-soft); }
  .b-api_key { color: var(--amber-ink); background: var(--amber-soft); }
</style>
