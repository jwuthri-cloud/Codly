<!--
  Settings → Autonomy & Budgets: the circuit breaker. A floor-wide guard against runaway agents
  and spend. When an agent crosses a limit Codly first steers it (a note in its next turn), then
  constrains it (interrupts and tells it to stop), and only with "Hard stop" kills it. Limits
  save at once; 0 turns a limit off. Every step lands on the agent's timeline as a decision.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { BreakerConfig } from '../lib/gen/BreakerConfig';
  import type { BreakerStatus } from '../lib/gen/BreakerStatus';
  import { api } from '../lib/api';
  import { store } from '../lib/state/store.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import { ago, tokens } from '../lib/util/format';

  let cfg = $state<BreakerConfig | null>(null);
  let status = $state<BreakerStatus | null>(null);

  async function load() {
    try {
      [cfg, status] = await Promise.all([api.breakerConfig(), api.breakerStatus()]);
    } catch {
      /* the section stays hidden until the core answers */
    }
  }
  onMount(() => {
    void load();
    const t = setInterval(() => api.breakerStatus().then((s) => (status = s)).catch(() => {}), 5000);
    return () => clearInterval(t);
  });

  /** Optimistic: shown at once, reverted with a toast if the core refuses. */
  async function save(patch: Partial<BreakerConfig>) {
    if (!cfg) return;
    const prev = cfg;
    cfg = { ...cfg, ...patch };
    try {
      cfg = await api.setBreakerConfig(cfg);
    } catch (e) {
      cfg = prev;
      toast({ kind: 'error', title: 'Couldn’t save the circuit breaker', body: e instanceof Error ? e.message : String(e) });
    }
  }
  /** A number field in `unit`s (the core stores plain counts). Blank or negative = off. */
  function num(e: Event, unit = 1) {
    const v = parseFloat((e.currentTarget as HTMLInputElement).value);
    return Number.isFinite(v) && v > 0 ? Math.round(v * unit) : 0;
  }
  async function reset() {
    try {
      await api.resetBreaker();
      status = await api.breakerStatus();
    } catch (e) {
      toast({ kind: 'error', title: 'Couldn’t reset', body: e instanceof Error ? e.message : String(e) });
    }
  }

  const agentName = (id: string | null) => store.agents.find((a) => a.id === id)?.name ?? 'An agent';
  /** The latest breaker decisions, newest first. */
  const events = $derived.by(() => {
    void store.rev; // the decision log is versioned by `rev`
    return store.decisions.filter((d) => d.type === 'breaker').slice(-6).reverse();
  });
  const flagged = $derived((status?.agents ?? []).filter((a) => a.level !== 'healthy'));
  const levelOf = (d: { detail: unknown }) => (d.detail as { level?: string } | null)?.level ?? 'healthy';
</script>

{#if cfg}
  <section>
    <h3 class="eyebrow">Autonomy & Budgets</h3>
    <p class="desc">
      A guard against runaway agents and spend. Over a limit, an agent is first <b>steered</b> (a note in its next turn), then <b>constrained</b>
      (interrupted and told to stop), and with Hard stop <b>stopped</b>. One step every 30 seconds; it steps back down as the agent recovers. The Lead is exempt.
    </p>

    <label class="row toggle">
      <div class="r-text"><span class="r-title">Circuit breaker</span><span class="desc">Watch every agent against the limits below.</span></div>
      <input type="checkbox" class="switch" checked={cfg.enabled} onchange={(e) => save({ enabled: e.currentTarget.checked })} />
    </label>

    <div class="limits" class:off={!cfg.enabled}>
      <div class="row">
        <div class="r-text">
          <span class="r-title">Token budget</span>
          <span class="desc">All agents together, since the app started or Reset. Cache reads don’t count.
            {#if status}<b>{tokens(status.floor_tokens)}</b> used.{/if}</span>
        </div>
        <input class="field" type="number" min="0" step="0.5" value={cfg.token_budget / 1e6 || ''} placeholder="Off" disabled={!cfg.enabled} aria-label="Token budget in millions" onchange={(e) => save({ token_budget: num(e, 1e6) })} />
        <span class="unit">M tokens</span>
      </div>
      <div class="row">
        <div class="r-text"><span class="r-title">Token velocity</span><span class="desc">Output one agent may sustain over the last two minutes.</span></div>
        <input class="field" type="number" min="0" step="10" value={cfg.velocity_per_min / 1000 || ''} placeholder="Off" disabled={!cfg.enabled} aria-label="Token velocity in thousands per minute" onchange={(e) => save({ velocity_per_min: num(e, 1000) })} />
        <span class="unit">k tok/min</span>
      </div>
      <div class="row">
        <div class="r-text"><span class="r-title">Repeated tool call</span><span class="desc">The same tool with the same input, this many times in a row.</span></div>
        <input class="field" type="number" min="0" step="1" value={cfg.repeated_tool_limit || ''} placeholder="Off" disabled={!cfg.enabled} aria-label="Repeated tool call limit" onchange={(e) => save({ repeated_tool_limit: num(e) })} />
        <span class="unit">calls</span>
      </div>
      <div class="row">
        <div class="r-text"><span class="r-title">Error storm</span><span class="desc">Errors and retries in a row, with no success between.</span></div>
        <input class="field" type="number" min="0" step="1" value={cfg.error_storm_limit || ''} placeholder="Off" disabled={!cfg.enabled} aria-label="Error storm limit" onchange={(e) => save({ error_storm_limit: num(e) })} />
        <span class="unit">errors</span>
      </div>
      <label class="row toggle">
        <div class="r-text"><span class="r-title">Hard stop</span><span class="desc">Kill an agent that is still over a limit after being constrained. Off: the breaker never goes past constraining. A stopped task is blocked until you move it to Ready.</span></div>
        <input type="checkbox" class="switch" checked={cfg.hard_stop} disabled={!cfg.enabled} onchange={(e) => save({ hard_stop: e.currentTarget.checked })} />
      </label>
    </div>

    <div class="foot">
      {#if flagged.length}
        <span class="desc">Now: {#each flagged as a, i (a.agent_id)}{i ? ', ' : ''}<b>{agentName(a.agent_id)}</b> {a.level}{/each}.</span>
      {:else}
        <span class="desc">All agents healthy.</span>
      {/if}
      <span class="grow"></span>
      <button class="btn" onclick={reset}>Reset counters</button>
    </div>

    {#if events.length}
      <ul class="log" aria-label="Recent breaker events">
        {#each events as d (d.id)}
          <li><span class="dot {levelOf(d)}"></span><span class="what">{d.summary}</span><span class="when">{ago(d.ts)}</span></li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

<style>
  section { padding: 20px 0; box-shadow: inset 0 -1px 0 var(--line); display: flex; flex-direction: column; gap: 12px; }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .desc b { color: var(--ink-2); font-weight: var(--w-med); }
  .grow { flex: 1; }
  .row { display: flex; align-items: center; gap: 12px; min-height: 42px; }
  .r-text { flex: 1; display: flex; flex-direction: column; gap: 2px; }
  .r-title { font-weight: var(--w-med); }
  .limits { display: flex; flex-direction: column; gap: 4px; }
  .limits.off { opacity: 0.55; }
  .field { width: 84px; text-align: right; font-size: var(--t-sm); }
  .unit { width: 64px; font-size: var(--t-sm); color: var(--ink-3); }
  .foot { display: flex; align-items: center; gap: 12px; }
  .btn { height: 28px; padding: 0 12px; border-radius: var(--r-full); background: var(--surface-1); font-size: var(--t-sm); font-weight: var(--w-med); }
  .btn:hover { background: var(--surface-2); }
  .log { display: flex; flex-direction: column; border-radius: var(--r-md); background: var(--surface-1); }
  .log li { display: flex; align-items: center; gap: 10px; padding: 8px 14px; font-size: var(--t-sm); }
  .log li + li { box-shadow: inset 0 1px 0 var(--line); }
  .what { flex: 1; min-width: 0; color: var(--ink-2); }
  .when { color: var(--ink-3); white-space: nowrap; }
  .dot { width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0; background: var(--ink-3); }
  .dot.healthy { background: var(--ok, var(--ink-3)); }
  .dot.steering { background: var(--amber-ink, var(--ink-2)); }
  .dot.constrained, .dot.stopped { background: var(--bad, var(--ink-1)); }
</style>
