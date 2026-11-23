<!--
  Level 1: every CLI Codly can drive, what it is missing, and the one click that fixes it:
  install, sign in or update. The commands come from Codly (CliInfo.install); a run opens a live
  terminal under the card.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import type { CliInfo } from '../lib/gen/CliInfo';
  import type { InstallRung } from '../lib/gen/InstallRung';
  import { store } from '../lib/state/store.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import { cliFix, cliOk, cliRank, cliState, codeParts, drone, installPlan, updateNote } from '../lib/util/cli';
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
  const NODE_URL = 'https://nodejs.org';

  /** One terminal at a time: `next` replaces the one before it, whose shell would otherwise stay alive until Codly quits. */
  function setRun(next: typeof run) {
    if (run) void api.ptyClose(run.ptyId).catch(() => {}); // already gone is fine
    run = next;
  }
  // The terminal finishes on its own clock: look again when you come back and every few seconds meanwhile.
  onMount(() => {
    const t = setInterval(() => run && void again(), 4000);
    const f = () => void again();
    window.addEventListener('focus', f);
    return () => {
      clearInterval(t);
      window.removeEventListener('focus', f);
      setRun(null);
    };
  });

  /** Install or update: the core types its own line for (provider, rung) into a fresh shell. */
  async function install(c: CliInfo, rung: InstallRung) {
    setRun(null);
    try {
      const r = await api.installCli(c.provider, rung);
      setRun({ provider: c.provider, ptyId: r.pty_id, command: r.command });
    } catch (e) {
      toast({ kind: 'error', title: rung === 'update' ? 'Could not start the update' : 'Could not start the install', body: e instanceof Error ? e.message : String(e) });
    }
  }
  /** Sign in: Codly's own login command, typed into a fresh shell for you to finish. */
  async function signIn(c: CliInfo) {
    const cmd = c.install.login;
    if (!cmd) return;
    setRun(null);
    try {
      const id = await api.ptyOpen(null);
      setRun({ provider: c.provider, ptyId: id, command: cmd });
      await api.ptyWrite(id, cmd + '\n');
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
          {@const plan = installPlan(c.install)}
          {@const first = plan.primary}
          {@const second = plan.other}
          {@const npmLine = c.install.npm}
          <div class="acts">
            {#if first}
              <button class="btn sm primary" onclick={() => install(c, first.rung)}><Icon name="play" size={12} />Install for me</button>
              <button class="btn sm ghost" onclick={() => copy(first.command)}><Icon name="copy" size={12} />Copy command</button>
              {#if second}<button class="btn sm ghost" data-tip={second.command} onclick={() => install(c, second.rung)}>Use npm instead</button>{/if}
            {:else}
              <button class="btn sm primary" onclick={() => openExternal(NODE_URL)}><Icon name="external" size={12} />Get Node.js</button>
              {#if npmLine}<button class="btn sm ghost" onclick={() => copy(npmLine)}><Icon name="copy" size={12} />Copy command</button>{/if}
            {/if}
            <button class="btn sm ghost" onclick={() => openExternal(c.install.docs)}><Icon name="external" size={12} />Docs</button>
          </div>
          {#if first}
            <code class="cmd selectable">{first.command}</code>
          {:else}
            <!-- "No npm" can be wrong (a Node.js that Codly does not look for): the line stays in reach. -->
            <p class="fix">{PROVIDER_LABEL[c.provider]} installs with <code>npm</code>, which comes with Node.js, and Codly found no <code>npm</code> on this Mac. Install Node.js, then check again.{#if npmLine} If you have <code>npm</code> already, run this line in a terminal:{/if}</p>
            {#if npmLine}<code class="cmd selectable">{npmLine}</code>{/if}
          {/if}
        {:else}
          {@const fix = cliFix(c)}
          {@const login = fix.login}
          {@const update = fix.update}
          {#if login || update}
            <div class="acts">
              {#if login}
                <button class="btn sm primary" onclick={() => signIn(c)}><Icon name="play" size={12} />Sign in</button>
                <button class="btn sm ghost" onclick={() => copy(login)}><Icon name="copy" size={12} />Copy command</button>
              {/if}
              {#if update}
                {#if update.command}<button class="btn sm" class:primary={!login} class:ghost={!!login} onclick={() => install(c, 'update')}><Icon name="refresh" size={12} />Update</button>{/if}
                <button class="btn sm ghost" onclick={() => openExternal(c.install.docs)}><Icon name="external" size={12} />Docs</button>
              {/if}
            </div>
          {/if}
          {#if fix.text}<p class="fix">{#each codeParts(fix.text) as p, i (i)}{#if p.code}<code>{p.t}</code>{:else}{p.t}{/if}{/each}</p>{/if}
          {#if update}
            <p class="fix warn">{updateNote(c)}</p>
            {#if update.command}<code class="cmd selectable">{update.command}</code>{/if}
            {#if update.how}<p class="fix">{update.how}</p>{/if}
          {/if}
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
  <p class="fine">Codly checks each sign-in with the CLI's own command or settings file. It does not read your CLIs' sign-in tokens, in the Keychain or on disk, unless you turn on “Show Claude usage” in Settings → Team → Accounts. One ready agent is enough to continue.</p>
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
  .fix.warn { color: var(--amber-ink); }
  .fix code, .fine code { font-size: 11px; padding: 1px 5px; border-radius: 4px; background: var(--surface-2); color: var(--ink-1); }
  .row { display: flex; align-items: center; gap: 14px; margin-top: 4px; }
  .fine { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
</style>
