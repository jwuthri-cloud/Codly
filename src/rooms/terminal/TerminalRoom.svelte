<!--
  Terminal room: a real shell (portable-pty in the core) in the repo, any agent's worktree, or
  a plan's integration worktree (Open in terminal on its card).
  An agent that is in your terminal ("Open in my terminal" in its sheet) has a tab of its own,
  where the core types its CLI's resume command. Sessions survive room switches; WebGL
  rendering only while visible.
-->
<script lang="ts">
  import '@xterm/xterm/css/xterm.css';
  import { onMount, tick, untrack } from 'svelte';
  import { api } from '../../lib/api';
  import { store } from '../../lib/state/store.svelte';
  import { takeovers } from '../../lib/state/takeover.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { idVar, PROVIDER_LABEL } from '../../lib/util/labels';
  import { tildify } from '../../lib/util/sanitize';
  import Icon from '../../components/Icon.svelte';
  import Portrait from '../../components/Portrait.svelte';
  import RoomHead from '../RoomHead.svelte';
  import { closeSession, disableWebgl, enableWebgl, openSession, sessions, takeoverKey, xtermTheme, type Session } from './sessions';

  /** `takeover`: the tab of an agent that is in your terminal, not a plain shell in a folder. */
  type Place = { key: string; cwd: string | null; label: string; agentId: string | null; takeover: boolean };
  const places = $derived.by((): Place[] => {
    const repo = store.workspace?.repo_path ?? null;
    const list: Place[] = [{ key: repo ?? '(repo)', cwd: repo, label: store.workspace?.name ?? 'Repository', agentId: null, takeover: false }];
    // Agents can share a worktree (or sit in the repo itself): one tab per path, keys must be unique.
    const seen = new Set(list.map((p) => p.key));
    for (const a of store.agents) {
      if (!a.worktree || seen.has(a.worktree)) continue;
      seen.add(a.worktree);
      list.push({ key: a.worktree, cwd: a.worktree, label: `${a.name}’s worktree`, agentId: a.id, takeover: false });
    }
    for (const t of takeovers.active) {
      const a = store.agentMap.get(t.agent_id);
      if (a) list.push({ key: takeoverKey(a.id), cwd: t.cwd, label: `${a.name} in ${PROVIDER_LABEL[a.provider]}`, agentId: a.id, takeover: true });
    }
    // A plan's integration worktree: when its card sent you here, and while a shell is open there.
    for (const p of store.plans) {
      const wt = p.integration_worktree;
      if (!wt || seen.has(wt) || (wt !== ui.terminalCwd && !openKeys.includes(wt))) continue;
      seen.add(wt);
      list.push({ key: wt, cwd: wt, label: `Plan: ${p.summary}`, agentId: null });
    }
    return list;
  });
  const current = $derived(
    places.find((p) => p.takeover && p.agentId === ui.terminalTakeover) ?? places.find((p) => !p.takeover && p.cwd === ui.terminalCwd) ?? places[0],
  );
  function pick(p: Place) {
    ui.terminalTakeover = p.takeover ? p.agentId : null;
    if (!p.takeover) ui.terminalCwd = p.cwd;
  }

  let host: HTMLDivElement | undefined = $state();
  let session = $state<Session | null>(null);
  let error = $state<string | null>(null);
  let openKeys = $state<string[]>([...sessions.keys()]);

  async function attach(p: Place) {
    if (!host) return;
    error = null;
    try {
      // An agent's own tab: the core opens the terminal and types the resume command, not run.
      const agentId = p.takeover ? p.agentId : null;
      const s = await openSession(p.cwd, p.label, agentId ? { key: p.key, open: () => api.openTakeoverTerminal(agentId) } : undefined);
      if (session && session !== s) {
        disableWebgl(session);
        session.el.remove();
      }
      session = s;
      openKeys = [...sessions.keys()];
      host.replaceChildren(s.el);
      await tick();
      await enableWebgl(s);
      refit();
      s.term.focus();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }
  function refit() {
    const s = session;
    if (!s || !host || host.clientWidth < 20) return;
    try {
      s.fit.fit();
      api.ptyResize(s.ptyId, s.term.cols, s.term.rows).catch(() => {});
    } catch {
      /* not measurable yet */
    }
  }
  const currentKey = $derived(current?.key ?? null);
  $effect(() => {
    const key = currentKey;
    if (!key || !host) return;
    untrack(() => current && attach(current));
  });
  $effect(() => {
    void store.settings.theme;
    void ui.systemDark;
    tick().then(() => {
      const t = xtermTheme();
      for (const s of sessions.values()) s.term.options.theme = t;
    });
  });
  onMount(() => {
    let raf = 0;
    const ro = new ResizeObserver(() => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(refit);
    });
    if (host) ro.observe(host);
    return () => {
      ro.disconnect();
      cancelAnimationFrame(raf);
      if (session) {
        disableWebgl(session);
        session.el.remove();
      }
    };
  });
  async function close() {
    if (!session) return;
    const key = session.key;
    session.el.remove();
    session = null;
    await closeSession(key);
    openKeys = [...sessions.keys()];
    if (current) attach(current);
  }
</script>

<RoomHead eyebrow="Terminal" title={current?.label ?? 'Terminal'}>
  {#snippet sub()}<code>{tildify(session?.cwd ?? current?.cwd ?? '')}</code>{/snippet}
  {#snippet right()}
    <button class="icon-btn" aria-label="Close this shell" data-tip="Close this shell" onclick={close} disabled={!session}><Icon name="x" size={15} /></button>
  {/snippet}
</RoomHead>
<div class="places" role="tablist" aria-label="Where">
  {#each places as p (p.key)}
    {@const a = p.agentId ? store.agentMap.get(p.agentId) : null}
    <button class="place" role="tab" aria-selected={current?.key === p.key} class:on={current?.key === p.key} style:--hue={a ? idVar(a.color) : 'var(--ink-3)'} onclick={() => pick(p)} data-tip={tildify(p.cwd)}>
      {#if a}<Portrait agent={a} size={18} />{:else}<Icon name="repo" size={14} />{/if}
      <span class="truncate">{p.label}</span>
      {#if openKeys.includes(p.key)}<span class="alive" aria-label="shell open"></span>{/if}
    </button>
  {/each}
</div>
<div class="screen">
  {#if error}<p class="err">Couldn’t open a shell: {error}</p>{/if}
  <div class="host" bind:this={host}></div>
</div>

<style>
  code { font-size: 11.5px; color: var(--ink-3); }
  .places { display: flex; gap: 4px; padding: 0 28px 12px; overflow-x: auto; scrollbar-width: none; flex-shrink: 0; }
  .place { display: inline-flex; align-items: center; gap: 7px; height: 30px; padding: 0 12px 0 8px; border-radius: var(--r-full); color: var(--ink-3); font-size: var(--t-sm); font-weight: var(--w-med); white-space: nowrap; max-width: 220px; }
  .place:hover { background: var(--surface-1); color: var(--ink-1); }
  .place.on { background: var(--surface-3); color: var(--ink-1); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--hue) 40%, transparent); }
  .alive { width: 6px; height: 6px; border-radius: 50%; background: var(--ok); }
  .screen { flex: 1; min-height: 0; position: relative; margin: 0 16px 16px; padding: 14px 6px 8px 16px; border-radius: var(--r-lg); background: var(--glass-solid); box-shadow: inset 0 0 0 1px var(--line); }
  .host { width: 100%; height: 100%; }
  .host :global(.xterm) { height: 100%; }
  .host :global(.xterm-viewport) { background: transparent !important; }
  .err { position: absolute; top: 14px; left: 18px; color: var(--bad); font-size: var(--t-sm); }
</style>
