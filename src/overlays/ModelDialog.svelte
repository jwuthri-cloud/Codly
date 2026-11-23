<!--
  Switch a 1:1 agent to another model and thinking effort. The list is the CLI's catalog
  (`api.modelCatalog`): Codex's own models with the efforts each one takes, what OpenCode
  prints, Claude's aliases. It opens in view with the current model marked and the keyboard in
  an empty field: typing narrows the list, ↑ ↓ move, ↵ switches to the highlighted one. Any other
  name can be typed (its row is the last one). A switch that fails says why here, and the dialog
  stays open.
-->
<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api } from '../lib/api';
  import { switchAgentModel } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import type { ModelCatalog } from '../lib/gen/ModelCatalog';
  import { PROVIDER_LABEL } from '../lib/util/labels';
  import { switchNote } from '../lib/util/modelNotes';
  import { firstRow, pickRows, type PickRow } from '../lib/util/modelPick';
  import { effortLabel, effortsFor, keptEffort, optionOf, sourceNote } from '../lib/util/models';
  import { joinModel, splitModel } from '../lib/util/team';
  import Dialog from '../components/Dialog.svelte';
  import Icon from '../components/Icon.svelte';

  const { agentId, provider } = ui.modelDialog!;
  const agent = store.agents.find((a) => a.id === agentId) ?? null;
  let catalog = $state<ModelCatalog | null>(null);
  let loading = $state(true);
  let problem = $state<string | null>(null);
  const current = splitModel(agent?.model ?? null);
  /** What "Switch model" switches to ('' = the CLI's default). */
  let model = $state(current.name);
  let query = $state('');
  let hi = $state(0);
  let effort = $state(current.effort);
  let busy = $state(false);
  /** Why the last switch did not happen. */
  let failure = $state<string | null>(null);
  let field: HTMLInputElement | undefined = $state();
  let list: HTMLUListElement | undefined = $state();

  const models = $derived(catalog?.models ?? []);
  const rows = $derived(pickRows(models, query, current.name));
  const efforts = $derived(effortsFor(catalog, provider, model.trim()));
  const defaultEffort = $derived(optionOf(catalog, model.trim())?.default_effort ?? null);
  const note = $derived(sourceNote(catalog, PROVIDER_LABEL[provider]));

  // A new list (typed words, the catalog arrived): the current model is highlighted, or the first row.
  $effect(() => {
    hi = firstRow(rows, !!query.trim());
  });
  $effect(() => {
    void hi;
    tick().then(() => list?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' }));
  });

  api
    .modelCatalog(provider)
    .then((c) => (catalog = c))
    .catch((e) => (problem = String(e)))
    .finally(() => (loading = false));

  const close = () => {
    ui.modelDialog = null;
  };
  // The agent is gone (let go while its menu was open): there is nothing to switch.
  onMount(() => {
    if (!agent) close();
    else requestAnimationFrame(() => field?.focus());
  });

  function choose(r: PickRow) {
    model = r.id;
    failure = null;
  }
  async function submit() {
    if (!agent || busy) return;
    busy = true;
    failure = null;
    // An effort the chosen model does not take is not sent: the model runs with its default.
    const why = await switchAgentModel(agent, joinModel(model, keptEffort(catalog, provider, model.trim(), effort)));
    busy = false;
    if (why) failure = why;
    else close();
  }
  function onKey(e: KeyboardEvent) {
    if (e.isComposing) return;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      if (rows.length) hi = (hi + (e.key === 'ArrowDown' ? 1 : rows.length - 1)) % rows.length;
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (e.repeat) return;
      if (rows[hi]) choose(rows[hi]);
      void submit();
    }
  }
</script>

{#if agent}
  <Dialog label="Change {agent.name}’s model" width={480} onclose={close}>
    <form onsubmit={(e) => { e.preventDefault(); void submit(); }}>
      <div class="body">
        <p class="desc">{PROVIDER_LABEL[provider]}. {switchNote(store.agentMap.get(agent.id) ?? agent)}</p>
        <div class="field">
          <label class="eyebrow" for="model-find">Model</label>
          <input id="model-find" bind:this={field} bind:value={query} role="combobox" aria-expanded="true" aria-controls="model-list" aria-autocomplete="list"
            aria-activedescendant={rows[hi] ? `model-row-${hi}` : undefined} placeholder={loading ? 'Loading models…' : 'Find a model, or type its name'} spellcheck="false" autocomplete="off" onkeydown={onKey} />
          <ul class="list" id="model-list" role="listbox" aria-label="Models" bind:this={list}>
            {#each rows as r, i (r.id + r.typed)}
              <li id="model-row-{i}" role="option" aria-selected={i === hi} class:active={i === hi} class:picked={r.id === model}
                onpointerenter={() => (hi = i)} onmousedown={(e) => { e.preventDefault(); choose(r); }}>
                <span class="mark">{#if r.id === model}<Icon name="check" size={13} />{/if}</span>
                <span class="ml truncate">{r.label}</span>
                {#if r.current}<span class="cur">current</span>{/if}
              </li>
            {/each}
          </ul>
        </div>
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
        {#if !loading && !problem && !models.length}<p class="muted small">This CLI doesn’t list its models. Type a name, or keep its default.</p>{/if}
        {#if failure}<p class="err" role="alert">Could not switch: {failure}</p>{/if}
      </div>
      <footer>
        <button type="button" class="btn sm ghost" onclick={close}>Cancel</button>
        <button type="submit" class="btn sm primary" disabled={busy}>{busy ? 'Switching…' : 'Switch model'}</button>
      </footer>
    </form>
  </Dialog>
{/if}

<style>
  .body { display: flex; flex-direction: column; gap: 14px; padding: 22px 26px; }
  .desc { color: var(--ink-2); font-size: var(--t-md); }
  .field { display: flex; flex-direction: column; gap: 8px; }
  .muted { color: var(--ink-3); font-weight: var(--w-reg); }
  .list { max-height: 216px; overflow-y: auto; padding: 4px; border-radius: var(--r-md); box-shadow: inset 0 0 0 1px var(--line); }
  .list li { display: flex; align-items: center; gap: 8px; height: 34px; padding: 0 10px 0 6px; border-radius: var(--r-sm); font-size: var(--t-md); cursor: default; }
  .list li.active { background: var(--surface-3); }
  .mark { width: 14px; display: grid; place-items: center; color: var(--ink-1); flex-shrink: 0; }
  .ml { flex: 1; min-width: 0; }
  .picked .ml { font-weight: var(--w-semi); }
  .cur { font-size: var(--t-sm); color: var(--ink-3); }
  .err { font-size: var(--t-sm); color: var(--bad); }
  footer { display: flex; justify-content: flex-end; gap: 8px; padding: 0 26px 22px; }
</style>
