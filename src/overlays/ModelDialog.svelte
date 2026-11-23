<!--
  Switch a 1:1 agent to another model and thinking effort. The list is the CLI's catalog
  (`api.modelCatalog`): Codex's own models with the efforts each one takes, what OpenCode
  prints, Claude's aliases. A line says where the list comes from; any other name can be typed.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import { setAgentModel } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import type { ModelCatalog } from '../lib/gen/ModelCatalog';
  import { PROVIDER_LABEL } from '../lib/util/labels';
  import { switchNote } from '../lib/util/modelNotes';
  import { effortLabel, effortsFor, keptEffort, optionLabel, optionOf, sourceNote } from '../lib/util/models';
  import { joinModel, splitModel } from '../lib/util/team';
  import Dialog from '../components/Dialog.svelte';

  const { agentId, provider } = ui.modelDialog!;
  const agent = store.agents.find((a) => a.id === agentId) ?? null;
  let catalog = $state<ModelCatalog | null>(null);
  let loading = $state(true);
  let problem = $state<string | null>(null);
  const current = splitModel(agent?.model ?? null);
  let model = $state(current.name);
  let effort = $state(current.effort);
  let busy = $state(false);

  const models = $derived(catalog?.models ?? []);
  const efforts = $derived(effortsFor(catalog, provider, model.trim()));
  const defaultEffort = $derived(optionOf(catalog, model.trim())?.default_effort ?? null);
  const note = $derived(sourceNote(catalog, PROVIDER_LABEL[provider]));

  api
    .modelCatalog(provider)
    .then((c) => (catalog = c))
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
    // An effort the chosen model does not take is not sent: the model runs with its default.
    const done = await setAgentModel(agent, joinModel(model, keptEffort(catalog, provider, model.trim(), effort)));
    busy = false;
    if (done) close();
  }
</script>

{#if agent}
  <Dialog label="Change {agent.name}’s model" width={480} onclose={close}>
    <form onsubmit={submit}>
      <div class="body">
        <p class="desc">{PROVIDER_LABEL[provider]}. {switchNote(store.agentMap.get(agent.id) ?? agent)}</p>
        <label class="field">
          <span class="eyebrow">Model</span>
          <input list="model-options" bind:value={model} placeholder={loading ? 'Loading models…' : 'CLI default'} spellcheck="false" autocomplete="off" />
          <datalist id="model-options">
            {#each models as m, i (i)}<option value={m.id}>{optionLabel(m)}</option>{/each}
          </datalist>
        </label>
        {#if efforts.length}
          <label class="field">
            <span class="eyebrow">Thinking effort</span>
            <select value={keptEffort(catalog, provider, model.trim(), effort)} onchange={(e) => (effort = e.currentTarget.value)}>
              <option value="">{defaultEffort ? `Default (${effortLabel(defaultEffort)})` : 'Default'}</option>
              {#each efforts as e (e)}<option value={e}>{effortLabel(e)}</option>{/each}
            </select>
          </label>
        {/if}
        {#if problem}<p class="muted small">Couldn’t load the model list ({problem}). You can still type a name.</p>{/if}
        {#if note}<p class="muted small">{note}</p>{/if}
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
