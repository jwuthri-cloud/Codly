<!--
  Level 1: every CLI Codly can drive, what it is missing, and the one click that fixes it.
  Install commands come from Codly (CliInfo.install); a run opens a live terminal under the card.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import type { CliInfo } from '../lib/gen/CliInfo';
  import { store } from '../lib/state/store.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import { cliOk, cliRank, cliState, codeParts, drone } from '../lib/util/cli';
  import { PROVIDER_LABEL } from '../lib/util/labels';
  import { openExternal } from '../lib/util/markdown';
  import BillingBadge from '../components/BillingBadge.svelte';
  import Icon from '../components/Icon.svelte';
  import Portrait from '../components/Portrait.svelte';
  import StateMark from '../components/StateMark.svelte';
  import InstallTerminal from './InstallTerminal.svelte';

  interface Props {
    again: () => Promise<void>;
  }
  let { again }: Props = $props();

  const sorted = $derived([...store.clis].sort((a, b) => cliRank(a) - cliRank(b)));
  let run = $state<{ provider: string; ptyId: string; command: string } | null>(null);
  let checking = $state(false);

  async function recheck() {
    checking = true;
    await again();
    checking = false;
  }
  // The terminal finishes on its own clock: look again when you come back and every few seconds meanwhile.
  onMount(() => {
    const t = setInterval(() => run && void again(), 4000);
    const f = () => void again();
    window.addEventListener('focus', f);
    return () => {
      clearInterval(t);
      window.removeEventListener('focus', f);
    };
  });

  async function install(c: CliInfo, rung: 'npm' | 'native') {
    try {
      const r = await api.installCli(c.provider, rung);
      run = { provider: c.provider, ptyId: r.pty_id, command: r.command };
    } catch (e) {
      toast({ kind: 'error', title: `Could not start the install`, body: e instanceof Error ? e.message : String(e) });
    }
  }
  /** Sign in: Codly's own login command, typed into a fresh shell for you to finish. */
  async function login(c: CliInfo) {
    const cmd = c.install.login;
    if (!cmd) return;
    try {
      const id = await api.ptyOpen(null);
      await api.ptyWrite(id, cmd + '\n');
      run = { provider: c.provider, ptyId: id, command: cmd };
    } catch (e) {
      toast({ kind: 'error', title: 'Could not open a terminal', body: e instanceof Error ? e.message : String(e) });
    }
  }
  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      toast({ kind: 'info', title: 'Copied', body: text, duration: 2500 });
    } catch {
      /* clipboard blocked */
    }
  }
</script>

<ul class="clis">
  {#each sorted as c (c.provider)}
    {@const s = cliState(c)}
    <li class="cli" class:off={!c.path} class:ready={cliOk(c)}>
      <Portrait agent={drone(c.provider)} size={40} dim={!c.path} />
      <div class="c-body">
        <p class="c-top"><b>{PROVIDER_LABEL[c.provider]}</b>{#if c.version}<span class="ver num">v{c.version}</span>{/if}{#if cliOk(c)}<span class="ok"><Icon name="check" size={13} />Ready</span>{/if}</p>
        <p class="c-state"><StateMark tone={s.tone} size={8} />{s.text}{#if c.path}<BillingBadge billing={c.billing} plan={c.plan} />{/if}</p>
        {#if !c.path}
          <div class="acts">
            {#if c.install.npm}
              <button class="btn sm primary" onclick={() => install(c, 'npm')}><Icon name="play" size={12} />Install for me</button>
              <button class="btn sm ghost" onclick={() => copy(c.install.npm!)}><Icon name="copy" size={12} />Copy command</button>
            {:else if c.install.native}
              <button class="btn sm primary" onclick={() => install(c, 'native')}><Icon name="play" size={12} />Run the installer</button>
              <button class="btn sm ghost" onclick={() => copy(c.install.native!)}><Icon name="copy" size={12} />Copy command</button>
            {/if}
            <button class="btn sm ghost" onclick={() => openExternal(c.install.docs)}><Icon name="external" size={12} />Docs</button>
          </div>
          {#if c.install.npm}<code class="cmd selectable">{c.install.npm}</code>{/if}
        {:else if c.logged_in === false}
          <div class="acts">
            {#if c.install.login}
              <button class="btn sm primary" onclick={() => login(c)}><Icon name="play" size={12} />Sign in</button>
              <button class="btn sm ghost" onclick={() => copy(c.install.login!)}><Icon name="copy" size={12} />Copy command</button>
            {/if}
          </div>
          {#if c.problem}<p class="fix">{#each codeParts(c.problem) as p, i (i)}{#if p.code}<code>{p.t}</code>{:else}{p.t}{/if}{/each}</p>{/if}
        {/if}
        {#if run?.provider === c.provider}
          <InstallTerminal ptyId={run.ptyId} />
          <p class="fine"><code>{run.command}</code> · I'll notice when it finishes.</p>
        {/if}
      </div>
    </li>
  {/each}
</ul>
<div class="row">
  <button class="btn ghost sm" onclick={recheck} disabled={checking}><Icon name="refresh" size={13} />{checking ? 'Checking…' : 'Check again'}</button>
  <p class="fine">Codly never reads your tokens or keychain: it runs each CLI's own commands. One ready agent is enough to continue.</p>
</div>

<style>
  .clis { display: flex; flex-direction: column; gap: 8px; }
  .cli { display: flex; gap: 14px; align-items: flex-start; padding: 12px 14px; border-radius: var(--r-lg); background: var(--surface-1); box-shadow: inset 0 0 0 1px var(--line); transition: box-shadow var(--d-3); }
  .cli.ready { box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--ok) 45%, transparent); }
  .cli.off { background: transparent; }
  .c-body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 5px; }
  .c-top { display: flex; align-items: baseline; gap: 8px; }
  .c-top b { font-size: 14.5px; font-weight: var(--w-semi); }
  .ver { font-family: var(--font-mono); font-size: 11px; color: var(--ink-3); }
  .ok { display: inline-flex; align-items: center; gap: 4px; margin-left: auto; font-size: var(--t-sm); color: var(--ok); animation: pop-in var(--d-4) var(--spring) both; }
  .c-state { display: flex; align-items: center; gap: 7px; font-size: var(--t-sm); color: var(--ink-2); }
  .acts { display: flex; gap: 6px; flex-wrap: wrap; margin-top: 2px; }
  .cmd { font-size: 11.5px; padding: 5px 8px; border-radius: 6px; background: var(--surface-2); color: var(--ink-2); width: fit-content; max-width: 100%; overflow-x: auto; white-space: nowrap; }
  .fix { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .fix code, .fine code { font-size: 11px; padding: 1px 5px; border-radius: 4px; background: var(--surface-2); color: var(--ink-1); }
  .row { display: flex; align-items: center; gap: 14px; margin-top: 4px; }
  .fine { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
</style>
