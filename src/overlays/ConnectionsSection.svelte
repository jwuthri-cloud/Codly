<!--
  Settings → Connections: the MCP servers new agents get by default. Read-only ones are on;
  ones that need a key (GitHub, a database, email and calendar) stay off until you add it.
  Keys go to the macOS Keychain and are never shown again. A change applies to agents that
  start from now on; agents already running keep the servers they started with.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import type { Connection } from '../lib/gen/Connection';
  import { toast } from '../lib/state/toasts.svelte';
  import Icon from '../components/Icon.svelte';

  let list = $state.raw<Connection[]>([]);
  let busy = $state<string | null>(null);
  /** Key being typed, by "<server>:<ENV>". Never kept after saving. */
  let draft = $state<Record<string, string>>({});
  let open = $state<string | null>(null);

  onMount(() => void api.listConnections().then((l) => (list = l)).catch(() => {}));

  async function run(id: string, p: Promise<Connection[]>) {
    busy = id;
    try {
      list = await p;
    } catch (e) {
      toast({ kind: 'error', title: 'Could not update the connection', body: e instanceof Error ? e.message : String(e) });
    } finally {
      busy = null;
    }
  }
  const toggle = (c: Connection) => run(c.id, api.updateConnection(c.id, !c.enabled));
  async function saveKey(c: Connection, env: string) {
    const k = `${c.id}:${env}`;
    const value = (draft[k] ?? '').trim();
    if (!value) return;
    await run(c.id, api.updateConnection(c.id, null, { env, value }));
    draft = { ...draft, [k]: '' };
  }
  const removeKey = (c: Connection, env: string) => run(c.id, api.updateConnection(c.id, null, { env, value: null }));

  const safe = $derived(list.filter((c) => c.tier === 'safe_readonly'));
  const keyed = $derived(list.filter((c) => c.tier === 'credentialed'));
  const stateOf = (c: Connection) => (c.active ? 'On for new agents' : c.enabled ? 'Add the key to turn it on' : 'Off');
</script>

<section>
  <h3 class="eyebrow">Connections</h3>
  <p class="desc">Tools every new agent starts with, through MCP. They apply to agents that start from now on; running agents are not changed.</p>

  {#snippet row(c: Connection)}
    <li class:on={c.active}>
      <div class="line">
        <div class="txt">
          <span class="name">{c.label}</span>
          <span class="desc">{c.blurb}</span>
        </div>
        <span class="state" class:warn={c.enabled && !c.active}>{stateOf(c)}</span>
        {#if c.secrets.length}
          <button class="btn ghost sm" onclick={() => (open = open === c.id ? null : c.id)} aria-expanded={open === c.id}>
            <Icon name="chevron-down" size={12} />{c.secrets.every((s) => s.set) ? 'Key added' : 'Add key'}
          </button>
        {/if}
        <input type="checkbox" class="switch" checked={c.enabled} disabled={busy === c.id} onchange={() => toggle(c)} aria-label={c.label} />
      </div>
      {#if c.secrets.length && open === c.id}
        <div class="keys">
          {#each c.secrets as s (s.env)}
            {@const k = `${c.id}:${s.env}`}
            <div class="key">
              <span class="klabel">{s.label}</span>
              {#if s.set}
                <span class="desc grow">Stored in your Keychain</span>
                <button class="btn ghost sm" onclick={() => removeKey(c, s.env)} disabled={busy === c.id}>Remove</button>
              {:else}
                <input class="field grow" type="password" autocomplete="off" spellcheck="false" placeholder={s.env} bind:value={draft[k]} onkeydown={(e) => e.key === 'Enter' && saveKey(c, s.env)} />
                <button class="btn sm" onclick={() => saveKey(c, s.env)} disabled={busy === c.id || !(draft[k] ?? '').trim()}>Save</button>
              {/if}
            </div>
          {/each}
          <p class="desc">Saved to the macOS Keychain, not to Codly’s settings. It is handed to the server when an agent starts.</p>
        </div>
      {/if}
    </li>
  {/snippet}

  <h4>Read-only · on by default</h4>
  <ul class="list">{#each safe as c (c.id)}{@render row(c)}{/each}</ul>
  <h4>Needs a key · off until you add one</h4>
  <ul class="list">{#each keyed as c (c.id)}{@render row(c)}{/each}</ul>
</section>

<style>
  section { padding: 20px 0; box-shadow: inset 0 -1px 0 var(--line); display: flex; flex-direction: column; gap: 12px; }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  h4 { font-size: var(--t-sm); font-weight: var(--w-semi); color: var(--ink-2); margin-top: 4px; }
  .list { display: flex; flex-direction: column; border-radius: var(--r-md); background: var(--surface-1); }
  li + li { box-shadow: inset 0 1px 0 var(--line); }
  .line { display: flex; align-items: center; gap: 12px; min-height: 52px; padding: 6px 12px 6px 14px; }
  .txt { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .name { font-weight: var(--w-med); }
  .state { font-size: var(--t-sm); color: var(--ink-4); white-space: nowrap; }
  li.on .state { color: var(--ok); }
  .state.warn { color: var(--amber-ink); }
  .keys { display: flex; flex-direction: column; gap: 8px; padding: 4px 14px 14px; }
  .key { display: flex; align-items: center; gap: 10px; }
  .klabel { width: 150px; flex-shrink: 0; font-size: var(--t-sm); color: var(--ink-2); }
  .grow { flex: 1; min-width: 0; }
  .field { height: 30px; padding: 0 10px; border-radius: var(--r-md); background: var(--surface-2); color: var(--ink-1); box-shadow: inset 0 0 0 1px var(--line); font: inherit; font-size: var(--t-sm); outline: none; }
  .field:focus { box-shadow: inset 0 0 0 1px var(--ink-3); }
</style>
