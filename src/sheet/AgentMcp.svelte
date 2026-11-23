<!--
  The MCP servers of the agent's live run, as Claude Code reports them: connected, still
  connecting, did not start (with Claude Code's reason), needs a sign-in, switched off. A quiet
  list: closed, the section's line says how many are in each state. "Reconnect" asks Claude
  Code to try again (orchestrator/live_session.rs), which can take as long as the server takes
  to start. When the server still does not connect, the answer shows under the server, beside
  the server's reason, until the server reports again.
-->
<script lang="ts">
  import type { Agent } from '../lib/gen/Agent';
  import { MCP_HINT, MCP_LABEL, canReconnect, mcpSummary, ofLiveRun, reconnectAnswer, reconnectMcp, type McpServer, type ReconnectAnswer } from '../lib/state/session';
  import { store } from '../lib/state/store.svelte';
  import { clean } from '../lib/util/sanitize';
  import Provenance from '../components/Provenance.svelte';
  import Section from '../components/Section.svelte';

  interface Props {
    agent: Agent;
  }
  let { agent }: Props = $props();

  const servers = $derived.by(() => {
    void store.revBy.get(agent.id);
    return ofLiveRun(store.live[agent.id]?.mcp ?? [], agent.run_id);
  });
  /** A server of this run (the sheet shows one agent after another). */
  const key = (name: string) => `${agent.run_id}:${name}`;
  /** The server a reconnect is on its way for. */
  let busy = $state<string | null>(null);
  /** What the last reconnect of a server came back with, when the server still did not connect. */
  let answers = $state<Record<string, ReconnectAnswer>>({});
  async function reconnect(s: McpServer) {
    const k = key(s.name);
    const seen = s.ts;
    busy = k;
    const text = await reconnectMcp(agent.id, s.name);
    const { [k]: _old, ...rest } = answers;
    answers = text == null ? rest : { ...rest, [k]: { text, seen } };
    busy = null;
  }
</script>

{#if servers.length}
  <Section id="mcp" title="MCP servers" summary={mcpSummary(servers)}>
    <ul class="servers">
      {#each servers as s (s.name)}
        {@const answer = reconnectAnswer(s, answers[key(s.name)])}
        <li class="st-{s.status}">
          <div class="row">
            <span class="dot" aria-hidden="true"></span>
            <span class="name truncate" data-tip={s.name}>{s.name}</span>
            <span class="state" data-tip={MCP_HINT[s.status]}>{MCP_LABEL[s.status]}</span>
            {#if canReconnect(s)}
              <button class="btn sm ghost" disabled={busy != null} onclick={() => reconnect(s)}>{busy === key(s.name) ? 'Reconnecting…' : 'Reconnect'}</button>
            {/if}
          </div>
          {#if s.status === 'needs_sign_in'}<p class="why">Sign in to this server in Claude Code, then reconnect.</p>{/if}
          {#if s.error}<p class="why selectable">{clean(s.error)}</p>{/if}
          {#if answer}<p class="why selectable">{clean(answer)}</p>{/if}
        </li>
      {/each}
    </ul>
    <p class="src"><Provenance kind="observed" /> as reported by Claude Code</p>
  </Section>
{/if}

<style>
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
