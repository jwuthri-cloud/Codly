<!--
  Live controls for one running agent. Enforced by the core (orchestrator/controls.rs), not by
  asking nicely: blocked tools are denied, a stop waits for the step to end, a steer note is an
  ordinary prompt tagged as context, and kill ends the process.
-->
<script lang="ts">
  import type { Agent } from '../lib/gen/Agent';
  import { stopAgent } from '../lib/state/actions';
  import { controls, resumeAgent, setBlockTools, steerAgent, stopAfterStep } from '../lib/state/control.svelte';
  import Icon from '../components/Icon.svelte';
  import Section from '../components/Section.svelte';

  interface Props {
    agent: Agent;
  }
  let { agent }: Props = $props();

  const c = $derived(controls.of(agent.id));
  const live = $derived(!!agent.run_id && agent.status !== 'offline');
  const isLead = $derived(agent.role === 'lead');
  const summary = $derived(c.paused ? 'paused' : c.stop_requested ? 'stopping…' : c.block_tools ? 'tools blocked' : 'none');

  let note = $state('');
  let sending = $state(false);
  let confirmKill = $state(false);

  async function send() {
    const text = note.trim();
    if (!text || sending) return;
    sending = true;
    if (await steerAgent(agent.id, text)) note = '';
    sending = false;
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void send();
    }
  }
</script>

<Section id="controls" title="Controls" {summary}>
  {#if !live}
    <p class="small">{agent.name} isn’t running. Controls appear while it has a live process.</p>
  {:else}
    <div class="ctl">
      <label class="row">
        <span class="txt">
          <b>Block tools</b>
          <span class="sub">{isLead ? 'The Lead needs its tools to plan.' : 'It can think and answer, but not run tools. Anything it tries is denied.'}</span>
        </span>
        <input class="switch" type="checkbox" role="switch" checked={c.block_tools} disabled={isLead} aria-label="Block tools" onchange={(e) => setBlockTools(agent.id, e.currentTarget.checked)} />
      </label>

      <div class="row">
        <span class="txt">
          <b>Stop after this step</b>
          {#if c.paused}
            <span class="sub state paused"><Icon name="pause" size={11} /> Paused{c.held ? ` · ${c.held} message${c.held === 1 ? '' : 's'} waiting` : ''}</span>
          {:else if c.stop_requested}
            <span class="sub state"><span class="pulse"></span> Stopping after this step…</span>
          {:else}
            <span class="sub">Lets it finish what it’s doing, then waits for you.</span>
          {/if}
        </span>
        {#if c.paused}
          <button class="btn sm primary" onclick={() => resumeAgent(agent.id)}><Icon name="play" size={12} />Resume</button>
        {:else if c.stop_requested}
          <button class="btn sm ghost" onclick={() => resumeAgent(agent.id)}>Cancel</button>
        {:else}
          <button class="btn sm" onclick={() => stopAfterStep(agent.id)}><Icon name="pause" size={12} />Stop after step</button>
        {/if}
      </div>

      <div class="steer">
        <b>Steer note</b>
        <textarea rows="2" placeholder="e.g. use the v2 client, not v1" bind:value={note} onkeydown={onKey} aria-label="Steer note"></textarea>
        <div class="steer-foot">
          <span class="sub">{c.paused || c.stop_requested ? 'Held until it resumes. ' : ''}Arrives as context on its next turn; nothing is typed into its terminal.</span>
          <button class="btn sm" disabled={!note.trim() || sending} onclick={send}><Icon name="send" size={12} />Send</button>
        </div>
      </div>

      {#if confirmKill}
        <div class="kill-confirm">
          <span>Kill {agent.name}? The process ends now; its worktree is kept.</span>
          <button class="btn sm danger" onclick={() => { confirmKill = false; stopAgent(agent.id); }}>Kill</button>
          <button class="btn sm ghost" onclick={() => (confirmKill = false)}>Keep</button>
        </div>
      {:else}
        <button class="btn sm danger kill" onclick={() => (confirmKill = true)}><Icon name="stop" size={12} />Kill agent…</button>
      {/if}
    </div>
  {/if}
</Section>

<style>
  .small { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .ctl { display: flex; flex-direction: column; gap: 14px; }
  .row { display: flex; align-items: center; gap: 12px; }
  .txt { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .txt b, .steer b { font-weight: var(--w-med); font-size: var(--t-md); color: var(--ink-1); }
  .sub { font-size: var(--t-xs); line-height: 16px; color: var(--ink-3); }
  .state { display: inline-flex; align-items: center; gap: 6px; color: var(--amber-ink); }
  .state.paused { color: var(--ink-2); }
  .pulse { width: 7px; height: 7px; border-radius: 50%; background: var(--amber); animation: pulse 1.2s ease-in-out infinite; }
  @keyframes pulse { 50% { opacity: 0.3; } }
  .steer { display: flex; flex-direction: column; gap: 6px; }
  .steer textarea { resize: vertical; min-height: 44px; padding: 8px 10px; border-radius: var(--r-md); background: var(--surface-1); font: inherit; font-size: var(--t-md); line-height: 19px; color: var(--ink-1); }
  .steer textarea:focus-visible { outline: 2px solid var(--live); outline-offset: 1px; }
  .steer-foot { display: flex; align-items: flex-end; gap: 10px; }
  .steer-foot .sub { flex: 1; }
  .kill { align-self: flex-start; }
  .kill-confirm { display: flex; align-items: center; gap: 6px; padding: 10px 12px; border-radius: var(--r-md); background: var(--bad-soft); font-size: var(--t-sm); }
  .kill-confirm span { flex: 1; }
  @media (prefers-reduced-motion: reduce) { .pulse { animation: none; } }
</style>
