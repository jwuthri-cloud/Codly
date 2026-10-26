<!--
  Pick the model for a new agent, or switch a 1:1 agent to another one. The list is the CLI's own
  catalog (OpenCode lists every provider you have a key for, Codex its models, Claude its aliases);
  any other name can be typed.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import { hireAgent, setAgentModel } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { COMPANY_LABEL, PROVIDER_LABEL } from '../lib/util/labels';
  import Icon from '../components/Icon.svelte';

  const { provider, agentId } = ui.modelDialog!;
  const agent = agentId ? (store.agents.find((a) => a.id === agentId) ?? null) : null;
  let models = $state<string[]>([]);
  let loading = $state(true);
  let problem = $state<string | null>(null);
  let model = $state(agent?.model ?? '');
  let busy = $state(false);
  let el: HTMLDivElement | undefined = $state();

  onMount(() => {
    el?.focus();
    api
      .listModels(provider)
      .then((list) => (models = list))
      .catch((e) => (problem = String(e)))
      .finally(() => (loading = false));
  });

  const close = () => (ui.modelDialog = null);

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    const next = model.trim() || null;
    const done = agent ? await setAgentModel(agent, next) : await hireAgent(provider, next);
    busy = false;
    if (done) close();
  }
</script>

<div class="scrim" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && close()}>
  <div
    class="sheet"
    role="dialog"
    aria-modal="true"
    aria-label={agent ? `Change ${agent.name}’s model` : 'Start an agent'}
    tabindex="-1"
    bind:this={el}
    onkeydown={(e) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        close();
      }
    }}
  >
    <form onsubmit={submit}>
      <header>
        <h2>{agent ? `Change ${agent.name}’s model` : `Start a ${PROVIDER_LABEL[provider]} agent`}</h2>
        <button type="button" class="icon-btn" aria-label="Close" onclick={close}><Icon name="x" /></button>
      </header>
      <div class="body">
        <p class="desc">
          {#if agent}
            {PROVIDER_LABEL[provider]}. The next message resumes the same conversation on the new model.
          {:else}
            {COMPANY_LABEL[provider]}. Pick a model, or type any name the CLI accepts. Empty means the CLI’s default.
          {/if}
        </p>
        <label class="field">
          <span class="eyebrow">Model</span>
          <input list="model-options" bind:value={model} placeholder={loading ? 'Loading models…' : 'CLI default'} spellcheck="false" autocomplete="off" />
          <datalist id="model-options">
            {#each models as m (m)}<option value={m}></option>{/each}
          </datalist>
        </label>
        {#if problem}<p class="muted small">Couldn’t load the model list ({problem}). You can still type a name.</p>{/if}
        {#if !loading && !problem && !models.length}<p class="muted small">This CLI doesn’t list its models. Type a name, or leave it empty.</p>{/if}
      </div>
      <footer>
        <button type="button" class="btn sm ghost" onclick={close}>Cancel</button>
        <button type="submit" class="btn sm primary" disabled={busy}>{agent ? 'Switch model' : 'Start agent'}</button>
      </footer>
    </form>
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; z-index: var(--z-modal); background: var(--scrim); display: grid; place-items: center; animation: fade-in var(--d-3) var(--ease-out); }
  .sheet { width: min(480px, calc(100vw - 32px)); border-radius: var(--r-xl); background: var(--pop); box-shadow: var(--pop-shadow), 0 40px 100px -30px rgba(0, 0, 0, 0.6); outline: none; animation: pop-in var(--d-4) var(--spring); }
  header { display: flex; align-items: center; justify-content: space-between; height: 64px; padding: 0 14px 0 26px; box-shadow: inset 0 -1px 0 var(--line); }
  h2 { font-family: var(--font-display); font-weight: 400; font-size: var(--d-sm); }
  .body { display: flex; flex-direction: column; gap: 14px; padding: 22px 26px; }
  .desc { color: var(--ink-2); font-size: var(--t-md); }
  .field { display: flex; flex-direction: column; gap: 8px; }
  input { height: 36px; padding: 0 12px; border-radius: var(--r-md); background: var(--surface-2); color: var(--ink-1); box-shadow: inset 0 0 0 1px var(--line); font: inherit; font-size: var(--t-md); outline: none; }
  input:focus { box-shadow: inset 0 0 0 1px var(--ink-3); }
  footer { display: flex; justify-content: flex-end; gap: 8px; padding: 0 26px 22px; }
</style>
