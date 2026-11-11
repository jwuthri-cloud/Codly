<!--
  Switch a 1:1 agent to another model. The list is the CLI's own catalog (OpenCode lists every
  provider you have a key for, Codex its models, Claude its aliases); any other name can be typed.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import { setAgentModel } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { PROVIDER_LABEL } from '../lib/util/labels';
  import Dialog from '../components/Dialog.svelte';

  const { agentId, provider } = ui.modelDialog!;
  const agent = store.agents.find((a) => a.id === agentId) ?? null;
  let models = $state<string[]>([]);
  let loading = $state(true);
  let problem = $state<string | null>(null);
  let model = $state(agent?.model ?? '');
  let busy = $state(false);

  api
    .listModels(provider)
    .then((list) => (models = list))
    .catch((e) => (problem = String(e)))
    .finally(() => (loading = false));

  const close = () => (ui.modelDialog = null);
  // The agent is gone (let go while its menu was open): there is nothing to switch.
  onMount(() => {
    if (!agent) close();
  });

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (!agent) return;
    busy = true;
    const done = await setAgentModel(agent, model.trim() || null);
    busy = false;
    if (done) close();
  }
</script>

{#if agent}
  <Dialog label="Change {agent.name}’s model" width={480} onclose={close}>
    <form onsubmit={submit}>
      <div class="body">
        <p class="desc">{PROVIDER_LABEL[provider]}. The next message resumes the same conversation on the new model.</p>
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
        <button type="submit" class="btn sm primary" disabled={busy}>Switch model</button>
      </footer>
    </form>
  </Dialog>
{/if}

<style>
  .body { display: flex; flex-direction: column; gap: 14px; padding: 22px 26px; }
  .desc { color: var(--ink-2); font-size: var(--t-md); }
  .field { display: flex; flex-direction: column; gap: 8px; }
  .muted { color: var(--ink-3); font-weight: var(--w-reg); }
  footer { display: flex; justify-content: flex-end; gap: 8px; padding: 0 26px 22px; }
</style>
