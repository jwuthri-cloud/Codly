<!--
  Start a 1:1 agent (harness, model and job title), or switch a 1:1 agent to another model. The list is the CLI's own
  catalog (OpenCode lists every provider you have a key for, Codex its models, Claude its aliases);
  any other name can be typed.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { Provider } from '../lib/gen/Provider';
  import { api } from '../lib/api';
  import { hireAgent, setAgentModel } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { COMPANY_LABEL, PROVIDER_LABEL } from '../lib/util/labels';
  import { ROLE_TITLES } from '../lib/util/team';
  import Icon from '../components/Icon.svelte';

  const { agentId } = ui.modelDialog!;
  const agent = agentId ? (store.agents.find((a) => a.id === agentId) ?? null) : null;
  /** CLIs a new agent can run on (installed, no known problem). */
  const ready = store.clis.filter((c) => c.path && !c.problem && c.logged_in !== false).map((c) => c.provider as Provider);
  let provider = $state<Provider>(ui.modelDialog!.provider);
  let models = $state<string[]>([]);
  let loading = $state(true);
  let problem = $state<string | null>(null);
  let model = $state(agent?.model ?? '');
  let title = $state('');
  let busy = $state(false);
  let el: HTMLDivElement | undefined = $state();

  onMount(() => el?.focus());
  $effect(() => {
    const p = provider;
    loading = true;
    problem = null;
    models = [];
    api
      .listModels(p)
      .then((list) => p === provider && (models = list))
      .catch((e) => p === provider && (problem = String(e)))
      .finally(() => p === provider && (loading = false));
  });
  function pick(p: Provider) {
    if (p !== provider) model = ''; // a model belongs to its CLI
    provider = p;
  }

  const close = () => (ui.modelDialog = null);

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    const next = model.trim() || null;
    const done = agent ? await setAgentModel(agent, next) : await hireAgent(provider, next, title || null);
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
        <h2>{agent ? `Change ${agent.name}’s model` : 'Start an agent'}</h2>
        <button type="button" class="icon-btn" aria-label="Close" onclick={close}><Icon name="x" /></button>
      </header>
      <div class="body">
        <p class="desc">
          {#if agent}
            {PROVIDER_LABEL[provider]}. The next message resumes the same conversation on the new model.
          {:else}
            An agent you chat with 1:1, in its own worktree. For agents the Lead delegates to,
            <button type="button" class="link" onclick={() => { close(); ui.settingsOpen = true; }}>add team members</button>.
          {/if}
        </p>
        {#if !agent}
          <div class="field">
            <span class="eyebrow">Harness</span>
            <div class="choices" role="radiogroup" aria-label="Harness">
              {#each ready as p (p)}
                <button type="button" class="choice" role="radio" aria-checked={provider === p} class:on={provider === p} onclick={() => pick(p)}>
                  <span class="radio"></span>{PROVIDER_LABEL[p]} <span class="muted small">{COMPANY_LABEL[p]}</span>
                </button>
              {/each}
            </div>
          </div>
          <label class="field">
            <span class="eyebrow">Title</span>
            <select bind:value={title}>
              <option value="">None</option>
              {#each ROLE_TITLES as t (t)}<option value={t}>{t}</option>{/each}
            </select>
          </label>
        {/if}
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
  .choices { display: flex; flex-wrap: wrap; gap: 6px; }
  .choice { display: inline-flex; align-items: center; gap: 8px; height: 30px; padding: 0 12px 0 10px; border-radius: var(--r-full); background: var(--surface-2); font-weight: var(--w-med); font-size: var(--t-sm); }
  .choice:hover { background: var(--surface-3); }
  .choice.on { background: var(--surface-3); box-shadow: inset 0 0 0 1.5px var(--ink-2); }
  .radio { width: 12px; height: 12px; border-radius: 50%; box-shadow: inset 0 0 0 1.5px var(--line-3); flex-shrink: 0; }
  .on .radio { box-shadow: inset 0 0 0 4px var(--ink-1); }
  .link { color: var(--ink-1); font-weight: var(--w-med); text-decoration: underline; text-decoration-color: var(--line-3); text-underline-offset: 3px; }
  .muted { color: var(--ink-3); font-weight: var(--w-reg); }
  footer { display: flex; justify-content: flex-end; gap: 8px; padding: 0 26px 22px; }
</style>
