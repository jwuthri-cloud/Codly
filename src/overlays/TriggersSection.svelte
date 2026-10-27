<!--
  Triggers: everything that can start work without you typing. Schedules (a prompt for the Lead
  on a clock), context rules (compact or clear an agent's conversation, only when the time gap
  AND the fill are met) and the network mode (what teammates may do here). Every edit saves at
  once; the core returns what it stored (ids, clamped numbers, clocks, the log of what fired).
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { ContextRule } from '../lib/gen/ContextRule';
  import type { NetworkMode } from '../lib/gen/NetworkMode';
  import type { Schedule } from '../lib/gen/Schedule';
  import type { TriggersConfig } from '../lib/gen/TriggersConfig';
  import { api } from '../lib/api';
  import { toast } from '../lib/state/toasts.svelte';
  import { ago } from '../lib/util/format';
  import { INTERVALS, NETWORK_MODES, everyLabel, inLabel, nextRunAt, ruleSentence } from '../lib/util/triggers';
  import Icon from '../components/Icon.svelte';

  let cfg = $state.raw<TriggersConfig | null>(null);
  let now = $state(Date.now());
  let saving = 0;

  async function load() {
    try {
      const fresh = await api.triggers();
      if (!saving) cfg = fresh; // never over a save in flight
    } catch (e) {
      if (!cfg) toast({ kind: 'error', title: 'Couldn’t load triggers', body: e instanceof Error ? e.message : String(e) });
    }
  }
  onMount(() => {
    load();
    // The core fires triggers on its own clock: re-read so "last fired" and the log stay true.
    const t = setInterval(() => {
      now = Date.now();
      load();
    }, 30_000);
    return () => clearInterval(t);
  });

  /** Show the change at once; keep what the core stored, or put the old one back on refusal. */
  async function save(next: TriggersConfig) {
    const prev = cfg;
    cfg = next;
    saving++;
    try {
      cfg = await api.setTriggers(next);
    } catch (e) {
      cfg = prev;
      toast({ kind: 'error', title: 'Couldn’t save triggers', body: e instanceof Error ? e.message : String(e) });
    } finally {
      saving--;
    }
  }

  const schedules = $derived(cfg?.schedules ?? []);
  function patchSchedule(i: number, p: Partial<Schedule>) {
    if (cfg) save({ ...cfg, schedules: schedules.map((s, j) => (j === i ? { ...s, ...p } : s)) });
  }
  function addSchedule() {
    if (!cfg) return;
    const blank: Schedule = { id: '', name: '', prompt: '', every_minutes: 60, enabled: true, created_at: Date.now(), last_fired_at: null };
    save({ ...cfg, schedules: [...schedules, blank] });
  }
  function removeSchedule(i: number) {
    if (cfg) save({ ...cfg, schedules: schedules.filter((_, j) => j !== i) });
  }
  const intervals = (m: number) => (INTERVALS.includes(m) ? INTERVALS : [...INTERVALS, m].sort((a, b) => a - b));

  function patchRule(which: 'compact' | 'clear', p: Partial<ContextRule>) {
    if (cfg) save({ ...cfg, context: { ...cfg.context, [which]: { ...cfg.context[which], ...p } } });
  }
  const num = (e: Event & { currentTarget: HTMLInputElement }, fallback: number) => {
    const n = Number(e.currentTarget.value);
    return Number.isFinite(n) ? Math.round(n) : fallback;
  };

  function setMode(mode: NetworkMode) {
    if (cfg) save({ ...cfg, network: { ...cfg.network, mode } });
  }
  function patchAdvanced(p: Partial<TriggersConfig['network']['advanced']>) {
    if (cfg) save({ ...cfg, network: { ...cfg.network, advanced: { ...cfg.network.advanced, ...p } } });
  }

  const log = $derived((cfg?.state.log ?? []).slice(0, 6));
</script>

<section>
  <h3 class="eyebrow">Triggers</h3>
  <p class="desc">Everything that can start work without you typing: a clock, a full context, a teammate.</p>

  {#if !cfg}
    <p class="desc">Loading…</p>
  {:else}
    <h4 class="r-title">Schedules</h4>
    <p class="desc">Ask the Lead something on a repeating clock, for example an hourly stand-up. It runs while Codly is open and a repository is, once if it was missed.</p>
    {#if schedules.length}
      <ul class="list">
        {#each schedules as s, i (s.id || i)}
          <li class="item" class:off={!s.enabled}>
            <div class="top">
              <input class="name" value={s.name} placeholder="Name" aria-label="Schedule name" spellcheck="false" maxlength="60" onchange={(e) => patchSchedule(i, { name: e.currentTarget.value })} />
              <span class="grow"></span>
              <label class="every">
                <span class="muted small">every</span>
                <select class="input" value={s.every_minutes} aria-label="Repeat every" onchange={(e) => patchSchedule(i, { every_minutes: Number(e.currentTarget.value) })}>
                  {#each intervals(s.every_minutes) as m (m)}<option value={m}>{everyLabel(m)}</option>{/each}
                </select>
              </label>
              <input type="checkbox" class="switch" checked={s.enabled} aria-label={`${s.name || 'Schedule'} on`} onchange={(e) => patchSchedule(i, { enabled: e.currentTarget.checked })} />
            </div>
            <textarea class="input prompt" rows="2" maxlength="2000" placeholder="What should the Lead do? e.g. Review every agent and tell me who is stuck." aria-label="Prompt for the Lead" value={s.prompt} onchange={(e) => patchSchedule(i, { prompt: e.currentTarget.value })}></textarea>
            <div class="foot">
              <span class="muted small">
                {s.last_fired_at ? `Last ran ${ago(s.last_fired_at, now)}` : 'Never ran'}
                {#if s.enabled && s.prompt.trim()} · next {inLabel(nextRunAt(s), now)}{:else if s.enabled} · needs a prompt{:else} · off{/if}
              </span>
              <span class="grow"></span>
              <button class="btn sm ghost rm" onclick={() => removeSchedule(i)}><Icon name="trash" size={13} />Remove</button>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
    <div><button class="btn sm ghost" onclick={addSchedule}><Icon name="plus" size={13} />Add schedule</button></div>

    <h4 class="r-title">Context rules</h4>
    <p class="desc">Long conversations get slow and expensive. A rule acts only when both are true: the time since last time has passed, and the agent’s context is at least this full. 0% means the clock alone decides. Agents mid-turn are left alone and tried again later.</p>
    <ul class="list">
      {#each [['compact', 'Compact', 'Summarises the conversation in place. Loses little.'], ['clear', 'Clear', 'Starts the conversation over.']] as const as [k, title, body] (k)}
        {@const r = cfg.context[k]}
        <li class="item" class:off={!r.enabled}>
          <div class="top">
            <span class="r-title">{title}</span>
            <span class="muted small">{body}</span>
            <span class="grow"></span>
            <input type="checkbox" class="switch" checked={r.enabled} aria-label={`${title} on`} onchange={(e) => patchRule(k, { enabled: e.currentTarget.checked })} />
          </div>
          <div class="rule">
            <label class="f"><span class="f-label">Every (hours)</span>
              <input class="input" type="number" min="1" max="168" value={r.every_hours} onchange={(e) => patchRule(k, { every_hours: num(e, r.every_hours) })} />
            </label>
            <label class="f"><span class="f-label">At least (% full)</span>
              <input class="input" type="number" min="0" max="100" value={r.min_percent} onchange={(e) => patchRule(k, { min_percent: num(e, r.min_percent) })} />
            </label>
          </div>
          <span class="muted small">{ruleSentence(k, r)}</span>
          {#if k === 'clear'}
            <p class="warn small">Clearing discards what the agent remembers mid-task, including decisions and half-finished work it has not written down. Agents holding a task are skipped. Off unless you turn it on.</p>
          {:else}
            <label class="f"><span class="f-label">Keep in mind when compacting <span class="muted">(optional)</span></span>
              <input class="input" value={cfg.context.compact_focus} maxlength="300" placeholder="file paths, open questions, the current plan" spellcheck="false" onchange={(e) => cfg && save({ ...cfg, context: { ...cfg.context, compact_focus: e.currentTarget.value } })} />
            </label>
            <span class="muted small">Compacting works on Claude agents today; other CLIs are skipped and noted in the activity below.</span>
          {/if}
        </li>
      {/each}
    </ul>

    <h4 class="r-title">Network</h4>
    <p class="desc">What teammates may do on this machine. Each level includes the one before.</p>
    <div class="modes">
      {#each NETWORK_MODES as m (m.id)}
        <button class="mode" class:on={cfg.network.mode === m.id} aria-pressed={cfg.network.mode === m.id} onclick={() => setMode(m.id)}>
          <span class="m-title"><span class="radio"></span>{m.title}</span>
          <span class="desc">{m.body}</span>
        </button>
      {/each}
    </div>
    <details class="adv">
      <summary>Advanced</summary>
      <label class="row">
        <div class="r-text"><span class="r-title">Confirm hand-offs</span><span class="desc">Work a teammate hands your agents waits for your approval, even in Open.</span></div>
        <input type="checkbox" class="switch" checked={cfg.network.advanced.confirm_handoffs} onchange={(e) => patchAdvanced({ confirm_handoffs: e.currentTarget.checked })} />
      </label>
      <label class="f"><span class="f-label">Only these teammates <span class="muted">(names, comma-separated; empty = anyone the level allows)</span></span>
        <input class="input" value={cfg.network.advanced.peers.join(', ')} placeholder="ana, bo" spellcheck="false" onchange={(e) => patchAdvanced({ peers: e.currentTarget.value.split(',') })} />
      </label>
      <p class="desc">This saves the setting and answers the permission check for connections Codly will make later; nothing listens on the network yet.</p>
    </details>

    <!-- Slot: Webhooks and Slack (their own agent) go here, as <h4 class="r-title"> blocks. -->
    <h4 class="r-title">Webhooks and Slack</h4>
    <p class="desc">Inbound triggers from other tools. Not set up yet.</p>

    {#if log.length}
      <h4 class="r-title">Recent activity</h4>
      <ul class="log">
        {#each log as l (l.ts + l.kind + l.label)}
          <li class:bad={!l.ok}>
            <span class="muted small">{ago(l.ts, now)}</span>
            <span class="small"><b>{l.kind === 'schedule' ? 'Schedule' : l.kind === 'compact' ? 'Compact' : 'Clear'}</b> {l.label}: {l.detail}</span>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</section>

<style>
  section { padding: 20px 0; box-shadow: inset 0 -1px 0 var(--line); display: flex; flex-direction: column; gap: 12px; }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .muted { color: var(--ink-3); font-weight: var(--w-reg); }
  .small { font-size: var(--t-sm); }
  .warn { color: var(--amber-ink); line-height: var(--lh-sm); }
  .grow { flex: 1; }
  .r-title { font-weight: var(--w-med); }
  h4 { margin-top: 6px; }
  .list { display: flex; flex-direction: column; border-radius: var(--r-md); background: var(--surface-1); }
  .item { display: flex; flex-direction: column; gap: 10px; padding: 12px 14px; }
  .item + .item { box-shadow: inset 0 1px 0 var(--line); }
  .item.off :is(.input, .name, .rule, .muted) { opacity: 0.6; }
  .top { display: flex; align-items: center; gap: 8px; min-height: 26px; min-width: 0; }
  .name { min-width: 8ch; max-width: 28ch; field-sizing: content; height: 26px; padding: 0 6px; margin-left: -6px; border: 0; border-radius: var(--r-sm); background: transparent; font-weight: var(--w-semi); font-size: var(--t-md); outline: none; color: var(--ink-1); }
  .name:hover, .name:focus { background: var(--surface-2); }
  .name:focus { box-shadow: inset 0 0 0 1px var(--focus); }
  .every { display: inline-flex; align-items: center; gap: 6px; }
  .prompt { resize: vertical; min-height: 44px; line-height: var(--lh-sm); padding-top: 6px; padding-bottom: 6px; }
  .foot { display: flex; align-items: center; gap: 6px; min-height: 26px; }
  .rm { color: var(--ink-3); }
  .rm:hover { color: var(--bad); background: var(--bad-soft); }
  .rule { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 8px; }
  .f { display: flex; flex-direction: column; gap: 5px; min-width: 0; }
  .f-label { font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-2); }
  .modes { display: flex; flex-direction: column; gap: 6px; }
  .mode { display: flex; flex-direction: column; gap: 3px; padding: 10px 14px; border-radius: var(--r-md); text-align: left; background: var(--surface-1); }
  .mode:hover { background: var(--surface-2); }
  .mode.on { background: var(--surface-3); box-shadow: inset 0 0 0 1.5px var(--ink-2); }
  .m-title { display: inline-flex; align-items: center; gap: 8px; font-weight: var(--w-semi); }
  .mode .desc { padding-left: 21px; }
  .radio { width: 13px; height: 13px; border-radius: 50%; box-shadow: inset 0 0 0 1.5px var(--line-3); flex-shrink: 0; }
  .on .radio { box-shadow: inset 0 0 0 4px var(--ink-1); }
  .adv { border-radius: var(--r-md); background: var(--surface-1); padding: 10px 14px; display: flex; flex-direction: column; gap: 10px; }
  .adv summary { cursor: pointer; font-weight: var(--w-med); color: var(--ink-2); }
  .adv[open] summary { margin-bottom: 10px; }
  .row { display: flex; align-items: center; gap: 16px; min-height: 42px; }
  .r-text { flex: 1; display: flex; flex-direction: column; gap: 2px; }
  .log { display: flex; flex-direction: column; gap: 4px; }
  .log li { display: flex; gap: 10px; align-items: baseline; }
  .log li .muted { min-width: 64px; }
  .log li.bad .small:last-child { color: var(--amber-ink); }
</style>
