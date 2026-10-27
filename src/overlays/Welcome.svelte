<!--
  Welcome (no workspace yet): what Codly is in one sentence, your crew on this Mac (each CLI,
  how it is billed, what is missing and how to fix it), and the repositories you opened before.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api, isTauriApp } from '../lib/api';
  import type { CliInfo } from '../lib/gen/CliInfo';
  import type { RecentWorkspace } from '../lib/gen/RecentWorkspace';
  import { openWorkspace, redetect } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { ago } from '../lib/util/format';
  import { cliOk, cliRank, cliState, codeParts, drone } from '../lib/util/cli';
  import { PROVIDER_LABEL } from '../lib/util/labels';
  import { tildify } from '../lib/util/sanitize';
  import BillingBadge from '../components/BillingBadge.svelte';
  import Icon from '../components/Icon.svelte';
  import Kbd from '../components/Kbd.svelte';
  import Logo from '../components/Logo.svelte';
  import Portrait from '../components/Portrait.svelte';
  import StateMark from '../components/StateMark.svelte';
  import { quest } from '../onboarding/quest.svelte';

  let recent = $state<RecentWorkspace[]>([]);
  let detecting = $state(false);
  let opening = $state<string | null>(null);
  onMount(() => {
    api.recentWorkspaces().then((r) => (recent = r)).catch(() => {});
  });

  const ok = cliOk;
  const ready = $derived(store.clis.filter(ok));
  const sorted = $derived([...store.clis].sort((a, b) => cliRank(a) - cliRank(b)));
  const TRANSPORT: Record<string, string> = {
    'stream-json': 'Codly talks to Claude Code over its stream-json protocol, with real approvals.',
    'app-server': 'Codly runs one private Codex app-server and gives each agent its own thread.',
    acp: 'Agent Client Protocol: one adapter for Gemini CLI, OpenCode, Cursor and Copilot.',
  };
  async function again() {
    detecting = true;
    await redetect();
    detecting = false;
  }
  async function open(path?: string) {
    opening = path ?? 'picker';
    await openWorkspace(path);
    opening = null;
  }
  const parts = codeParts;
</script>

<div class="welcome">
  <div class="sky" aria-hidden="true"></div>
  <div class="drag" data-tauri-drag-region class:tauri={isTauriApp()}></div>
  <div class="scroll">
    <div class="page">
      <section class="hero">
        <p class="brand"><Logo size={26} /><span>Codly</span></p>
        <h1 class="serif">Your coding agents, working in <em>plain sight.</em></h1>
        <p class="lede">
          Codly runs the CLIs you already pay for — Claude Code, Codex, Gemini CLI, OpenCode — as one crew. The Lead plans, each
          agent works in its own worktree, and nothing lands on your branch until you say so.
        </p>
        <div class="cta">
          <button class="btn primary lg" onclick={() => open()} disabled={!!opening}>
            <Icon name="folder" size={15} />{opening === 'picker' ? 'Opening…' : 'Open a repository'}<span class="k">⌘O</span>
          </button>
          <span class="note">{ready.length ? `${ready.length} of ${store.clis.length} agents ready` : 'No agent CLI is ready yet'}</span>
        </div>

        {#if recent.length}
          <div class="recent">
            <p class="eyebrow">Recent</p>
            <ul>
              {#each recent as r (r.repo_path)}
                <li>
                  <button class="r-row" onclick={() => open(r.repo_path)} disabled={!!opening}>
                    <span class="r-name serif">{r.name}</span>
                    <span class="r-path truncate">{tildify(r.repo_path)}</span>
                    <span class="r-when num">{opening === r.repo_path ? 'Opening…' : ago(r.opened_at, ui.now)}</span>
                    <Icon name="arrow-right" size={14} />
                  </button>
                </li>
              {/each}
            </ul>
          </div>
        {/if}
      </section>

      <section class="crew">
        <div class="c-head">
          <p class="eyebrow">Your crew on this Mac</p>
          <button class="btn ghost sm" onclick={again} disabled={detecting}><Icon name="refresh" size={13} />{detecting ? 'Checking…' : 'Check again'}</button>
        </div>
        <ul class="clis">
          {#each sorted as c (c.provider)}
            {@const s = cliState(c)}
            <li class="cli glass" class:off={!c.path}>
              <Portrait agent={drone(c.provider)} size={38} dim={!c.path} />
              <div class="c-body">
                <p class="c-top"><b>{PROVIDER_LABEL[c.provider]}</b>{#if c.version}<span class="ver num">v{c.version}</span>{/if}</p>
                <p class="c-state"><StateMark tone={s.tone} size={8} />{s.text}</p>
                <div class="c-meta">
                  {#if c.path}<BillingBadge billing={c.billing} plan={c.plan} />{/if}
                  <span class="transport" data-tip={TRANSPORT[c.transport] ?? c.transport}>via {c.transport}</span>
                </div>
                {#if c.problem}
                  <p class="fix selectable" class:warn={!!c.path}>{#each parts(c.problem) as p, i (i)}{#if p.code}<code>{p.t}</code>{:else}{p.t}{/if}{/each}</p>
                {/if}
              </div>
            </li>
          {/each}
        </ul>
        <p class="fine">Codly never reads your tokens or keychain. Agents run the unmodified CLIs you signed in to; costs on a subscription are list-price estimates, included in your plan.</p>
      </section>
    </div>
    <footer class="foot">
      <span><Kbd keys="⌘K" /> search</span>
      <span><Kbd keys="?" /> shortcuts</span>
      <button class="btn ghost sm" onclick={() => quest.replay()}><Icon name="sparkle" size={13} />Replay onboarding</button>
      <button class="btn ghost sm" onclick={() => (ui.settingsOpen = true)}><Icon name="settings" size={13} />Settings</button>
    </footer>
  </div>
</div>

<style>
  .welcome { position: fixed; inset: 0; display: flex; flex-direction: column; background: var(--night); overflow: hidden; }
  .sky { position: absolute; inset: 0; pointer-events: none; background: radial-gradient(120% 80% at 50% 125%, var(--night-glow), transparent 70%); }
  .drag { position: relative; height: 40px; flex-shrink: 0; }
  .scroll { position: relative; flex: 1; overflow-y: auto; display: flex; flex-direction: column; }
  .page { flex: 1; width: min(1180px, calc(100vw - 64px)); margin: 0 auto; padding: 5vh 0 32px; display: grid; grid-template-columns: minmax(0, 1.05fr) minmax(0, 1fr); gap: 64px; align-items: start; }
  .hero { display: flex; flex-direction: column; gap: 22px; animation: rise-in var(--d-5) var(--spring) both; }
  .brand { display: inline-flex; align-items: center; gap: 10px; font-weight: var(--w-semi); font-size: 15px; letter-spacing: 0.01em; }
  h1 { font-size: clamp(40px, 5.2vw, 64px); line-height: 1.02; letter-spacing: -0.015em; max-width: 13ch; }
  h1 em { font-style: italic; color: var(--ink-2); }
  .lede { font-size: 16px; line-height: 26px; color: var(--ink-2); max-width: 52ch; }
  .cta { display: flex; align-items: center; gap: 16px; margin-top: 4px; }
  .note { font-size: var(--t-sm); color: var(--ink-3); }
  .recent { margin-top: 18px; }
  .recent .eyebrow { margin-bottom: 8px; }
  .recent ul { display: flex; flex-direction: column; }
  .r-row { display: flex; align-items: baseline; gap: 14px; width: 100%; padding: 12px 4px; text-align: left; color: var(--ink-3); box-shadow: inset 0 -1px 0 var(--line); }
  .r-row:hover { color: var(--ink-1); }
  .r-row :global(svg) { align-self: center; opacity: 0; transform: translateX(-4px); transition: opacity var(--d-2), transform var(--d-2) var(--spring); }
  .r-row:hover :global(svg) { opacity: 1; transform: none; }
  .r-name { font-size: 21px; color: var(--ink-1); font-style: italic; white-space: nowrap; }
  .r-path { flex: 1; font-family: var(--font-mono); font-size: 11.5px; }
  .r-when { font-size: var(--t-sm); white-space: nowrap; }
  .crew { display: flex; flex-direction: column; gap: 12px; animation: rise-in var(--d-5) var(--spring) both; animation-delay: 90ms; padding-top: 52px; }
  .c-head { display: flex; align-items: center; justify-content: space-between; }
  .clis { display: flex; flex-direction: column; gap: 8px; }
  .cli { display: flex; gap: 14px; align-items: flex-start; padding: 14px 16px; border-radius: var(--r-lg); }
  .cli.off { background: transparent; box-shadow: inset 0 0 0 1px var(--line); -webkit-backdrop-filter: none; backdrop-filter: none; }
  .c-body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .c-top { display: flex; align-items: baseline; gap: 8px; }
  .c-top b { font-size: 14.5px; font-weight: var(--w-semi); }
  .off .c-top b { color: var(--ink-2); }
  .ver { font-family: var(--font-mono); font-size: 11px; color: var(--ink-3); }
  .c-state { display: flex; align-items: center; gap: 7px; font-size: var(--t-sm); color: var(--ink-2); }
  .c-meta { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; margin-top: 2px; }
  .transport { font-size: var(--t-xs); color: var(--ink-3); font-family: var(--font-mono); }
  .fix { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); margin-top: 4px; }
  .fix.warn { color: var(--amber-ink); }
  .fix code { font-size: 11px; padding: 1px 5px; border-radius: 4px; background: var(--surface-2); color: var(--ink-1); }
  .fine { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); margin-top: 6px; }
  .foot { width: min(1180px, calc(100vw - 64px)); margin: 0 auto; display: flex; align-items: center; gap: 18px; padding: 18px 0 26px; color: var(--ink-3); font-size: var(--t-sm); }
  .foot span { display: inline-flex; gap: 6px; align-items: center; }
  .foot .btn:first-of-type { margin-left: auto; }
  @media (max-width: 1100px) { .page { grid-template-columns: 1fr; gap: 28px; } .crew { padding-top: 0; } }
</style>
