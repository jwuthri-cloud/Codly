<!--
  The agent's own work between two messages, folded into one quiet line ("Read 6 files ·
  searched twice"). Open it for each step (with provenance) and the reasoning summary. A
  helper (subagent) is one step, with its own calls under it. A step that a subagent made
  among the agent's own (Codex) says whose it is. A row of thoughts only, in a turn that failed
  or was stopped before the agent answered, says that the turn ended while it was thinking.
-->
<script lang="ts">
  import { store, type Seg, type SubAgent, type Thought } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { dur } from '../../lib/util/format';
  import { uniqueKeys } from '../../lib/util/keys';
  import { pastVerb, phrase, subagentTag } from '../../lib/util/labels';
  import { isPlanCall, planStep } from '../../lib/util/planRequest';
  import Icon from '../../components/Icon.svelte';
  import Provenance from '../../components/Provenance.svelte';
  import { splitHelpers } from '../helpers';
  import { endedBeforeAnswer } from '../turnEnd';
  import { workSummary } from '../workSummary';
  import HelperRow from './HelperRow.svelte';

  interface Props {
    segs: Seg[];
    thoughts: Thought[];
    thinking: string | null;
    /** The helpers (subagents) that calls of this row started. */
    helpers: SubAgent[];
  }
  let { segs, thoughts, thinking, helpers }: Props = $props();
  let open = $state(false);
  /** The agent's own steps, and each helper's calls apart from them. */
  const split = $derived(splitHelpers(segs, helpers));
  /** One key per step, its call id. A CLI may use an id again in a later turn of the same stretch: that step adds its place. */
  const ownKeys = $derived(uniqueKeys(split.own, (s) => s.callId));
  const helperOf = (s: Seg) => helpers.find((h) => h.callId === s.callId);
  const running = $derived(segs.some((s) => s.end == null) || thinking != null || helpers.some((h) => h.status === 'running'));
  /** Only the latest outcome counts: a failing run fixed later in the same stretch is not a failure. */
  const failed = $derived(split.own.filter((s) => s.ok != null).at(-1)?.ok === false);
  /** The CLI of the agent whose work this is: only a Claude agent's call of the plan tool is its plan. */
  const provider = $derived(store.agentMap.get(segs[0]?.agentId ?? '')?.provider ?? null);
  const done = $derived(workSummary(split.own, provider));
  /** Only thoughts, and the turn failed or was stopped before the agent answered: they were cut. */
  const cut = $derived.by(() => {
    if (done || thinking != null || !thoughts.length) return false;
    void store.rev;
    return endedBeforeAnswer(store.events, thoughts[0].eventId);
  });
  const summary = $derived(done || (thinking != null ? 'Thinking' : cut ? 'Was thinking when the turn ended' : thoughts.length ? 'Thought it through' : ''));
  const steps = $derived(split.own.length + thoughts.length);
</script>

{#snippet step(s: Seg)}
  {@const by = subagentTag(s.input)}
  <button class="step" onclick={() => ui.selectAgent(s.agentId, s.eventId)}>
    <!-- A Claude agent's plan shown to you is said as that: the name of its tool says nothing. -->
    <span class="verb">{isPlanCall(s.name, provider) ? planStep(s.end == null) : s.end == null ? phrase(s.activity, '') : pastVerb(s.activity)}</span>
    <code class="target truncate">{isPlanCall(s.name, provider) ? '' : s.label || s.name}</code>
    {#if by}<span class="by">{by}</span>{/if}
    {#if s.testsPassed != null}<span class="tests" class:bad={!!s.testsFailed}>{s.testsFailed ? `${s.testsFailed} failed` : `${s.testsPassed} passed`}</span>{/if}
    <span class="dur num">{s.end != null ? dur(s.end - s.start) : '…'}</span>
    <Provenance kind={s.confidence === 'observed' ? 'observed' : 'inferred'} compact />
  </button>
{/snippet}

{#if summary}
  <div class="work" class:running>
    <button class="line" onclick={() => (open = !open)} aria-expanded={open}>
      <span class="ic" class:live={running} class:bad={failed && !running}>
        {#if running}<span class="pulse"></span>{:else}<Icon name={failed ? 'alert' : cut ? 'pause' : 'check'} size={12} stroke={2} />{/if}
      </span>
      <span class="sum truncate">{summary}</span>
      {#if steps > 1}<span class="n num">{steps} steps</span>{/if}
      <span class="tri" class:open><Icon name="chevron-right" size={11} stroke={2} /></span>
    </button>
    {#if open}
      <ol class="steps">
        {#each split.own as s, si (ownKeys[si])}
          {@const helper = helperOf(s)}
          <li>
            {#if helper}
              <HelperRow {helper} call={s} steps={split.under.get(s.callId) ?? []} {step} />
            {:else}
              {@render step(s)}
            {/if}
          </li>
        {/each}
        {#each thoughts as t (t.itemId)}
          <li class="thought"><Provenance kind="model" compact /><p class="selectable">{t.text}</p></li>
        {/each}
        {#if thinking}<li class="thought live"><Provenance kind="model" compact /><p class="selectable">{thinking}</p></li>{/if}
      </ol>
    {/if}
  </div>
{/if}

<style>
  .work { margin: -4px 0; }
  .line { display: inline-flex; align-items: center; gap: 8px; max-width: 100%; height: 26px; padding: 0 10px 0 6px; margin-left: -6px; border-radius: var(--r-full); font-size: var(--t-sm); color: var(--ink-3); }
  .line:hover { background: var(--surface-1); color: var(--ink-2); }
  .ic { display: grid; place-items: center; width: 14px; height: 14px; color: var(--ink-3); }
  .ic.bad { color: var(--bad); }
  .pulse { width: 7px; height: 7px; border-radius: 50%; background: var(--aurora); box-shadow: 0 0 0 3px var(--live-soft); }
  .running .sum { color: var(--ink-2); }
  .n { color: var(--ink-4); }
  .tri { display: inline-grid; color: var(--ink-4); transition: transform var(--d-2) var(--ease-out); }
  .tri.open { transform: rotate(90deg); }
  .steps { margin: 6px 0 4px 6px; padding-left: 12px; box-shadow: inset 1px 0 0 var(--line-2); display: flex; flex-direction: column; gap: 1px; animation: fade-in var(--d-2) var(--ease-out); }
  .step { display: flex; align-items: center; gap: 9px; width: 100%; height: 26px; padding: 0 8px; border-radius: var(--r-sm); font-size: var(--t-sm); text-align: left; }
  .step:hover { background: var(--surface-1); }
  .verb { color: var(--ink-2); white-space: nowrap; }
  .target { flex: 1; font-family: var(--font-mono); font-size: 11.5px; color: var(--ink-1); }
  .by { color: var(--ink-3); white-space: nowrap; }
  .tests { color: var(--ok); white-space: nowrap; }
  .tests.bad { color: var(--bad); }
  .dur { color: var(--ink-3); white-space: nowrap; }
  .thought { display: flex; gap: 8px; align-items: flex-start; padding: 6px 8px; }
  .thought p { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); font-style: italic; max-width: 60ch; }
  .thought.live p { color: var(--ink-2); }
</style>
