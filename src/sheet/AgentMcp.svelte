<!--
  The MCP servers of the agent's live run, as Claude Code reports them: connected, still
  connecting, did not start (with Claude Code's reason), needs a sign-in, switched off. A quiet
  list: closed, the section's line says how many are in each state. "Reconnect" asks Claude
  Code to try again (orchestrator/live_session.rs), which can take as long as the server takes
  to start. When the server still does not connect, the answer shows under the server, beside
  the server's reason, until the server reports again. "Restart" on a connected server sends the
  same request, which starts it again. "Turn off" on a connected server and "Turn on" on a
  switched-off one go to the agent's live run. Not recorded: whether Claude Code saves the
  switch, so the button's hint promises nothing about the agent's next run.
  A button reads "Reconnecting…", "Restarting…", "Turning off…" or "Turning on…" until the
  server's next status row arrives (or 10 s pass), and the row's buttons stay off for a moment
  after its state changed: Reconnect and Restart sit last in the row, where the other takes the
  place of the one you pressed, so a press too many never turns a server off.
  A run that started without the folder's own CLI configuration says so on top.
-->
<script lang="ts">
  import type { Agent } from '../lib/gen/Agent';
  import { MCP_BUSY, MCP_HINT, MCP_LABEL, WITHOUT_FOLDER_CONFIG_LINE, canReconnect, canRestart, mcpAskOnItsWay, mcpRowLocked, mcpSummary, mcpSwitch, mcpSwitchHint, ofLiveRun, reconnectAnswer, reconnectMcp, restartHint, restartMcp, signInSteps, startedWithoutFolderConfig, switchMcp, type McpAsk, type McpServer, type ReconnectAnswer } from '../lib/state/session';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { clean } from '../lib/util/sanitize';
  import Provenance from '../components/Provenance.svelte';
  import Section from '../components/Section.svelte';

  interface Props {
    agent: Agent;
  }
  let { agent }: Props = $props();

  const servers = $derived.by(() => {
    void store.revBy.get(agent.id);
    // (a server that stands where an earlier run recorded it comes as a live event, in no log)
    void store.liveRevBy.get(agent.id);
    return ofLiveRun(store.live[agent.id]?.mcp ?? [], agent.run_id);
  });
  const withoutFolderConfig = $derived.by(() => {
    void store.revBy.get(agent.id);
    return startedWithoutFolderConfig(store.events, agent.id, agent.run_id);
  });
  /** A server of this run (the sheet shows one agent after another). */
  const key = (name: string) => `${agent.run_id}:${name}`;
  /** The last request about each server: until its next status row it is on its way (`mcpAskOnItsWay`). */
  let asks = $state<Record<string, McpAsk>>({});
  /** What the last reconnect of a server came back with, when the server still did not connect. */
  let answers = $state<Record<string, ReconnectAnswer>>({});
  const ask = (s: McpServer, what: McpAsk['what']) => (asks = { ...asks, [key(s.name)]: { what, seen: s.ts, at: ui.now } });
  /** A request that came back without a change: the button is there again at once. */
  function settled(s: McpServer) {
    const { [key(s.name)]: _old, ...rest } = asks;
    asks = rest;
  }
  async function reconnect(s: McpServer) {
    const k = key(s.name);
    ask(s, 'reconnect');
    const text = await reconnectMcp(agent.id, s.name);
    const { [k]: _old, ...rest } = answers;
    answers = text == null ? rest : { ...rest, [k]: { text, seen: s.ts } };
    if (text != null) settled(s);
  }
  async function restart(s: McpServer) {
    ask(s, 'restart');
    if (!(await restartMcp(agent.id, s.name))) settled(s);
  }
  /** Turn the server off or on. Its row changes when the server reports where it stands. */
  async function flip(s: McpServer, enabled: boolean) {
    ask(s, enabled ? 'on' : 'off');
    if (!(await switchMcp(agent.id, s.name, enabled))) settled(s);
  }
</script>

{#if servers.length}
  <Section id="mcp" title="MCP servers" summary={mcpSummary(servers)}>
    {#if withoutFolderConfig}<p class="note">{WITHOUT_FOLDER_CONFIG_LINE}</p>{/if}
    <ul class="servers">
      {#each servers as s (s.name)}
        {@const answer = reconnectAnswer(s, answers[key(s.name)])}
        {@const flips = mcpSwitch(s)}
        {@const going = mcpAskOnItsWay(s, asks[key(s.name)], ui.now)}
        {@const locked = mcpRowLocked(s, asks[key(s.name)], ui.now)}
        <li class="st-{s.status}">
          <div class="row">
            <span class="dot" aria-hidden="true"></span>
            <span class="name truncate" data-tip={s.name}>{s.name}</span>
            <span class="state" data-tip={MCP_HINT[s.status]}>{MCP_LABEL[s.status]}</span>
            {#if flips}
              <button class="btn sm ghost" disabled={locked} aria-label="{flips.label} {s.name}" data-tip={mcpSwitchHint(clean(agent.name))} onclick={() => flip(s, flips.enabled)}>{going?.what === (flips.enabled ? 'on' : 'off') ? flips.busy : flips.label}</button>
            {/if}
            {#if canReconnect(s)}
              <button class="btn sm ghost" disabled={locked} aria-label="Reconnect {s.name}" onclick={() => reconnect(s)}>{going?.what === 'reconnect' ? MCP_BUSY.reconnect : 'Reconnect'}</button>
            {:else if canRestart(s)}
              <button class="btn sm ghost" disabled={locked} aria-label="Restart {s.name}" data-tip={restartHint(clean(agent.name))} onclick={() => restart(s)}>{going?.what === 'restart' ? MCP_BUSY.restart : 'Restart'}</button>
            {/if}
          </div>
          {#if s.status === 'needs_sign_in'}<p class="why">{signInSteps(clean(s.name))}</p>{/if}
          {#if s.error}<p class="why selectable">{clean(s.error)}</p>{/if}
          {#if answer}<p class="why selectable">{clean(answer)}</p>{/if}
        </li>
      {/each}
    </ul>
    <p class="src"><Provenance kind="observed" /> as reported by Claude Code</p>
  </Section>
{/if}

<style>
  .note { margin-bottom: 10px; font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--amber-ink); }
  .servers { display: flex; flex-direction: column; gap: 9px; }
  .row { display: flex; align-items: center; gap: 10px; min-width: 0; min-height: 24px; }
  .dot { flex: none; width: 8px; height: 8px; border-radius: 50%; background: var(--ink-4); }
  .st-connected .dot { background: var(--ok); }
  .st-needs_sign_in .dot { background: var(--amber); }
  .st-failed .dot { background: var(--bad); }
  .name { flex: 1; min-width: 0; font-size: var(--t-md); color: var(--ink-1); }
  .state { font-size: var(--t-xs); color: var(--ink-3); white-space: nowrap; }
  .st-needs_sign_in .state { color: var(--amber-ink); }
  .why { margin: 1px 0 0 18px; font-size: var(--t-xs); line-height: 16px; color: var(--ink-3); overflow-wrap: anywhere; }
  .src { margin-top: 12px; display: flex; align-items: center; gap: 6px; font-size: var(--t-xs); color: var(--ink-3); }
</style>
