<!--
  Level 3: every other model, through OpenCode and one OpenRouter key. The key is opt-in, lives in the
  Mac's Keychain, and is handed to OpenCode only (Codly otherwise scrubs API keys from agents).
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import type { ByokStatus } from '../lib/gen/ByokStatus';
  import { toast } from '../lib/state/toasts.svelte';
  import { openExternal } from '../lib/util/markdown';
  import Icon from '../components/Icon.svelte';
  import { catalog } from './catalog';

  let st = $state<ByokStatus>({ enabled: false, key_saved: false });
  let key = $state('');
  let busy = $state(false);
  onMount(() => {
    api.byokStatus().then((s) => (st = s)).catch(() => {});
  });

  async function run(f: () => Promise<ByokStatus | void>, ok?: string) {
    busy = true;
    try {
      const s = await f();
      if (s) st = s;
      if (ok) toast({ kind: 'success', title: ok, duration: 3000 });
    } catch (e) {
      toast({ kind: 'error', title: 'Could not save', body: e instanceof Error ? e.message : String(e) });
    } finally {
      busy = false;
    }
  }
  const save = () => run(async () => {
    const s = await api.byokSaveKey(key.trim());
    key = '';
    return s;
  }, 'OpenRouter key saved');
  const parts = (s: string) => s.split(/(`[^`]+`)/g).map((p) => ({ code: p.startsWith('`'), t: p.replace(/`/g, '') }));
</script>

<ol class="steps">
  {#each catalog.brains.steps as s, i (s.title)}
    <li>
      <span class="n">{i + 1}</span>
      <div>
        <b>{s.title}</b>
        <p>{#each parts(s.body) as p, j (j)}{#if p.code}<code>{p.t}</code>{:else}{p.t}{/if}{/each}</p>
        {#if s.url}<button class="btn sm ghost" onclick={() => openExternal(s.url!)}><Icon name="external" size={12} />Open openrouter.ai/keys</button>{/if}
        {#if i === 2}
          <div class="key">
            <input type="password" bind:value={key} placeholder={st.key_saved ? 'Replace the saved key' : 'sk-or-…'} autocomplete="off" spellcheck="false" aria-label="OpenRouter API key" disabled={busy} />
            <button class="btn sm primary" onclick={save} disabled={busy || !key.trim()}>Save</button>
            {#if st.key_saved}<button class="btn sm ghost" onclick={() => run(() => api.byokClear(), 'Key removed')} disabled={busy}>Remove</button>{/if}
          </div>
          <label class="use">
            <input type="checkbox" class="switch" checked={st.enabled} disabled={busy || !st.key_saved} onchange={(e) => run(() => api.byokConfigure(e.currentTarget.checked))} />
            <span>Let OpenCode use my OpenRouter key <em>· billed to your OpenRouter account</em></span>
          </label>
          <p class="state">{st.key_saved ? 'Key saved in your Mac’s Keychain. It is never shown again.' : 'No key saved. Skipping this is fine: nothing changes.'}</p>
        {/if}
      </div>
    </li>
  {/each}
</ol>
<p class="local">{#each parts(catalog.brains.local) as p, j (j)}{#if p.code}<code>{p.t}</code>{:else}{p.t}{/if}{/each}</p>

<style>
  .steps { display: flex; flex-direction: column; gap: 10px; }
  li { display: flex; gap: 14px; padding: 12px 14px; border-radius: var(--r-lg); background: var(--surface-1); box-shadow: inset 0 0 0 1px var(--line); }
  .n { flex: none; width: 26px; height: 26px; border-radius: 9px; display: grid; place-items: center; font-family: var(--font-mono); font-size: 12px; background: var(--surface-3); color: var(--ink-1); }
  li > div { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 6px; align-items: flex-start; }
  b { font-size: 14px; font-weight: var(--w-semi); }
  p { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-2); }
  code { font-size: 11px; padding: 1px 5px; border-radius: 4px; background: var(--surface-2); color: var(--ink-1); }
  .key { display: flex; gap: 6px; width: 100%; }
  .key input { flex: 1; }
  .use { display: flex; align-items: center; gap: 10px; font-size: var(--t-sm); color: var(--ink-2); }
  .use em { color: var(--ink-3); font-style: normal; }
  .state, .local { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
</style>
