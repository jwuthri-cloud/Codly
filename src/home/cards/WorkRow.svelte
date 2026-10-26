<!--
  The agent's own work between two messages, folded into one quiet line ("Read 6 files ·
  searched twice"). Open it for each step (with provenance) and the reasoning summary.
-->
<script lang="ts">
  import type { Seg, Thought } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { dur } from '../../lib/util/format';
  import { pastVerb, phrase } from '../../lib/util/labels';
  import Icon from '../../components/Icon.svelte';
  import Provenance from '../../components/Provenance.svelte';
  import { workSummary } from '../workSummary';

  interface Props {
    segs: Seg[];
    thoughts: Thought[];
    thinking: string | null;
  }
  let { segs, thoughts, thinking }: Props = $props();
  let open = $state(false);
  const running = $derived(segs.some((s) => s.end == null) || thinking != null);
  /** Only the latest outcome counts: a failing run fixed later in the same stretch is not a failure. */
  const failed = $derived(segs.filter((s) => s.ok != null).at(-1)?.ok === false);
  const summary = $derived(workSummary(segs) || (thinking != null ? 'Thinking' : thoughts.length ? 'Thought it through' : ''));
  const steps = $derived(segs.length + thoughts.length);
</script>

{#if summary}
  <div class="work" class:running>
    <button class="line" onclick={() => (open = !open)} aria-expanded={open}>
      <span class="ic" class:live={running} class:bad={failed && !running}>
        {#if running}<span class="pulse"></span>{:else}<Icon name={failed ? 'alert' : 'check'} size={12} stroke={2} />{/if}
      </span>
      <span class="sum truncate">{summary}</span>
      {#if steps > 1}<span class="n num">{steps} steps</span>{/if}
      <span class="tri" class:open><Icon name="chevron-right" size={11} stroke={2} /></span>
    </button>
    {#if open}
      <ol class="steps">
        {#each segs as s (s.callId)}
          <li>
            <button class="step" onclick={() => ui.selectAgent(s.agentId, s.eventId)}>
              <span class="verb">{s.end == null ? phrase(s.activity, '') : pastVerb(s.activity)}</span>
              <code class="target truncate">{s.label || s.name}</code>
              {#if s.testsPassed != null}<span class="tests" class:bad={!!s.testsFailed}>{s.testsFailed ? `${s.testsFailed} failed` : `${s.testsPassed} passed`}</span>{/if}
              <span class="dur num">{s.end != null ? dur(s.end - s.start) : '…'}</span>
              <Provenance kind={s.confidence === 'observed' ? 'observed' : 'inferred'} compact />
            </button>
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
  .tests { color: var(--ok); white-space: nowrap; }
  .tests.bad { color: var(--bad); }
  .dur { color: var(--ink-3); white-space: nowrap; }
  .thought { display: flex; gap: 8px; align-items: flex-start; padding: 6px 8px; }
  .thought p { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); font-style: italic; max-width: 60ch; }
  .thought.live p { color: var(--ink-2); }
</style>
