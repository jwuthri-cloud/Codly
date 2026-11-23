<!--
  Settings → API keys: your own credentials per vendor, each handed to one CLI only
  (Anthropic → Claude Code, OpenAI → Codex, OpenRouter → OpenCode). A source is off, a saved key,
  or a command that prints a short-lived token (workload identity federation); Claude and Codex
  re-run the command when the token expires. A key goes in once and never comes back: this
  screen only knows "key saved". A command is not secret, so it is shown.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import type { ByokSource } from '../lib/gen/ByokSource';
  import type { ByokStatus } from '../lib/gen/ByokStatus';
  import type { Vendor } from '../lib/gen/Vendor';
  import { Loader } from '../lib/state/loader.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import LoadError from '../components/LoadError.svelte';

  const VENDORS: { id: Vendor; label: string; cli: string; placeholder: string }[] = [
    { id: 'anthropic', label: 'Anthropic', cli: 'Claude Code', placeholder: 'sk-ant-…' },
    { id: 'openai', label: 'OpenAI', cli: 'Codex', placeholder: 'sk-proj-…' },
    { id: 'openrouter', label: 'OpenRouter', cli: 'OpenCode', placeholder: 'sk-or-…' },
  ];
  const KINDS: { kind: ByokSource['kind']; label: string }[] = [
    { kind: 'off', label: 'Off' },
    { kind: 'key', label: 'Key' },
    { kind: 'command', label: 'Token command' },
  ];

  const byok = new Loader<ByokStatus>(() => api.byokStatus());
  const st = $derived(byok.value);
  let busy = $state(false);
  /** What the user picked but has not saved yet (Key before a key is saved, Command before its line). */
  let pick = $state<Partial<Record<Vendor, ByokSource['kind']>>>({});
  let keys = $state<Record<Vendor, string>>({ anthropic: '', openai: '', openrouter: '' });
  let commands = $state<Record<Vendor, string>>({ anthropic: '', openai: '', openrouter: '' });

  async function refresh() {
    if ((await byok.load()) && byok.value) showCommands(byok.value);
  }
  onMount(() => void refresh());

  /** A command is not secret: the saved one shows in its field. */
  function showCommands(s: ByokStatus) {
    for (const v of s.vendors) if (v.source.kind === 'command') commands[v.vendor] = v.source.command;
  }
  function load(s: ByokStatus) {
    byok.set(s);
    showCommands(s);
  }

  const of = (v: Vendor) => st?.vendors.find((x) => x.vendor === v);
  const shown = (v: Vendor) => pick[v] ?? of(v)?.source.kind ?? 'off';

  async function run(f: () => Promise<ByokStatus>, ok?: string): Promise<boolean> {
    busy = true;
    try {
      load(await f());
      if (ok) toast({ kind: 'success', title: ok, duration: 3000 });
      return true;
    } catch (e) {
      toast({ kind: 'error', title: 'Could not save', body: e instanceof Error ? e.message : String(e) });
      return false;
    } finally {
      busy = false;
    }
  }

  /** Off applies at once; Key applies at once when a key is saved; Command waits for its line. */
  async function choose(v: Vendor, kind: ByokSource['kind']) {
    pick[v] = kind;
    if (kind === 'off' || (kind === 'key' && of(v)?.key_saved)) {
      if (await run(() => api.byokConfigure(v, { kind } as ByokSource))) delete pick[v];
    }
  }

  async function saveKey(v: Vendor) {
    const key = keys[v].trim();
    if (!key) return;
    if (!(await run(() => api.byokSaveKey(v, key), 'Key saved'))) return;
    keys[v] = ''; // gone from the screen at once
    if (await run(() => api.byokConfigure(v, { kind: 'key' }))) delete pick[v];
  }

  async function saveCommand(v: Vendor) {
    const command = commands[v].trim();
    if (!command) return;
    if (await run(() => api.byokConfigure(v, { kind: 'command', command }), 'Token command saved')) delete pick[v];
  }
</script>

<section>
  <h3 class="eyebrow">API keys</h3>
  <p class="desc">Off by default: agents use your CLI sign-ins. Turn a vendor on to bill its API instead, with a saved key or a command that prints a token (for workload identity federation). Each credential reaches one CLI only, and applies to agents started after the change.</p>
  {#if st}
    {#each VENDORS as v (v.id)}
      {@const cur = of(v.id)}
      <div class="vendor">
        <div class="row">
          <div class="r-text">
            <span class="r-title">{v.label} <span class="cli">→ {v.cli}</span></span>
            <span class="desc">
              {#if cur?.source.kind === 'key'}<span class="on">Using your key.</span>
              {:else if cur?.source.kind === 'command'}<span class="on">Using the token command.</span>
              {:else}Subscription only.{/if}
              {#if cur?.key_saved} A key is saved; it is never shown again.{/if}
            </span>
          </div>
          <div class="seg" role="radiogroup" aria-label="{v.label} credential">
            {#each KINDS as k (k.kind)}
              <button role="radio" aria-checked={shown(v.id) === k.kind} class:on={shown(v.id) === k.kind} disabled={busy} onclick={() => choose(v.id, k.kind)}>{k.label}</button>
            {/each}
          </div>
        </div>
        {#if shown(v.id) === 'key'}
          <form class="row" onsubmit={(e) => { e.preventDefault(); void saveKey(v.id); }}>
            <input class="field wide" type="password" bind:value={keys[v.id]} placeholder={cur?.key_saved ? 'Replace the saved key' : v.placeholder} spellcheck="false" autocomplete="off" aria-label="{v.label} API key" disabled={busy} />
            <button class="btn" type="submit" disabled={busy || !keys[v.id].trim()}>Save key</button>
            {#if cur?.key_saved}<button class="btn" type="button" disabled={busy} onclick={() => run(() => api.byokClear(v.id), 'Key removed')}>Remove</button>{/if}
          </form>
        {:else if shown(v.id) === 'command'}
          <form class="row" onsubmit={(e) => { e.preventDefault(); void saveCommand(v.id); }}>
            <input class="field wide mono" type="text" bind:value={commands[v.id]} placeholder="a command that prints one token, e.g. ~/bin/get-token {v.id}" spellcheck="false" autocomplete="off" aria-label="{v.label} token command" disabled={busy} />
            <button class="btn" type="submit" disabled={busy || !commands[v.id].trim()}>Save</button>
          </form>
          <p class="desc">Runs with <code>/bin/sh -c</code> in your login environment, no input, and must print only the token. {v.id === 'openrouter' ? 'Codly runs it once per agent start.' : `${v.cli} runs it again before the token expires.`}</p>
        {/if}
      </div>
    {/each}
  {:else if byok.error}
    <LoadError what="your API keys" error={byok.error} loading={byok.loading} onretry={refresh} />
  {:else}
    <p class="desc">Loading…</p>
  {/if}
</section>

<style>
  section { padding: 20px 0; box-shadow: inset 0 -1px 0 var(--line); display: flex; flex-direction: column; gap: 12px; }
  .vendor { display: flex; flex-direction: column; gap: 6px; padding: 10px 0; box-shadow: inset 0 1px 0 var(--line); }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .row { display: flex; align-items: center; gap: 16px; min-height: 42px; }
  .r-text { flex: 1; display: flex; flex-direction: column; gap: 2px; }
  .r-title { font-weight: var(--w-med); }
  .cli { color: var(--ink-3); font-weight: normal; }
  .on { color: var(--ink-1); font-weight: var(--w-med); }
  .seg { display: flex; gap: 2px; padding: 2px; border-radius: var(--r-full); background: var(--surface-2); }
  .seg button { height: 28px; padding: 0 12px; border-radius: var(--r-full); font-size: var(--t-sm); color: var(--ink-2); }
  .seg button.on { background: var(--surface-3); color: var(--ink-1); font-weight: var(--w-med); }
  .seg button:disabled { opacity: 0.6; }
  .field { font-size: var(--t-sm); }
  .field.wide { flex: 1; width: auto; }
  .mono { font-family: var(--font-mono); }
  code { font-size: 11px; padding: 1px 5px; border-radius: 4px; background: var(--surface-2); color: var(--ink-1); }
  .btn { height: 32px; padding: 0 14px; border-radius: var(--r-full); background: var(--surface-2); font-weight: var(--w-med); font-size: var(--t-sm); }
  .btn:hover:not(:disabled) { background: var(--surface-3); }
  .btn:disabled { opacity: 0.5; }
</style>
