<!--
  One crew member: portrait inside its orbit (todo progress), name, provider, and ONE sentence
  of narration. Amber when it needs you. Hover cross-highlights its task, lane and files.
  Subagents ride along as small companions. A Lead is compact: its name, and on hover the
  request it serves; clicking it opens that request.
-->
<script lang="ts">
  import type { Agent } from '../lib/gen/Agent';
  import { accounts } from '../lib/state/accounts.svelte';
  import { narrate } from '../lib/state/narration';
  import { requests } from '../lib/state/requests.svelte';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { idVar, PROVIDER_LABEL } from '../lib/util/labels';
  import Orbit from '../components/Orbit.svelte';
  import Portrait from '../components/Portrait.svelte';
  import Say from '../components/Say.svelte';

  interface Props {
    agent: Agent;
    compact?: boolean;
  }
  let { agent, compact = false }: Props = $props();

  const n = $derived(narrate(agent.id));
  const tone = $derived(n?.tone ?? 'idle');
  const subs = $derived.by(() => {
    void store.rev;
    return (store.live[agent.id]?.subagents ?? []).filter((s) => s.status === 'running');
  });
  const lit = $derived.by(() => {
    const h = ui.hover;
    if (h.agentId === agent.id) return true;
    if (h.taskId && store.taskMap.get(h.taskId)?.assignee === agent.id) return true;
    if (h.path) {
      void store.rev;
      return !!store.live[agent.id]?.files[h.path];
    }
    return false;
  });
  const selected = $derived(ui.selectedAgentId === agent.id);
  const isLead = $derived(agent.role === 'lead');
  /** The request this agent works for (a Lead's own; a worker's through its task's plan). */
  const req = $derived(requests.get(isLead ? (agent.thread ?? 'lead') : store.taskThread(agent.task_id)));
  const many = $derived(requests.list.length > 1);
  /** "Claude Code · work" when its CLI has several sign-ins in use. */
  const acct = $derived(accounts.tag(agent));
  const tip = $derived(
    isLead ? `${agent.name}${acct ? ` (${acct})` : ''} · ${req?.title ?? 'New request'} — ${req?.status ?? n?.text ?? ''}`
    : compact ? `${agent.name}${acct ? ` · ${accounts.who(agent)}` : ''}: ${n?.text ?? ''}`
    : [acct && accounts.who(agent), many && agent.task_id && req && `For: ${req.title}`].filter(Boolean).join(' — ') || undefined,
  );
  const here = $derived(isLead && ui.room === 'home' && ui.thread === (agent.thread ?? 'lead'));
  const amber = $derived(isLead && !!req?.needsYou);

  function open(e: MouseEvent) {
    if (isLead) ui.openRequest(agent.thread ?? 'lead');
    else if (e.altKey) ui.openThread(store.threadOf(agent.id));
    else ui.selectAgent(selected ? null : agent.id);
  }
  function openSub(e: MouseEvent) {
    e.stopPropagation();
    ui.sheetFocus = 'subagents';
    ui.selectAgent(agent.id);
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="pill t-{tone}"
  class:lit
  class:selected={selected || here}
  class:compact
  class:lead={isLead}
  class:amber
  style:--hue={idVar(agent.color)}
  onpointerenter={() => ui.setHover({ agentId: agent.id })}
  onpointerleave={() => ui.hover.agentId === agent.id && ui.setHover(null)}
>
  <button class="hit" onclick={open} aria-pressed={selected || here} aria-label="{agent.name}, {isLead ? `Lead of “${req?.title ?? 'a new request'}”` : accounts.who(agent)}: {n?.key ?? ''}" data-tip={tip}></button>
  <span class="face">
    <Orbit size={36} progress={n?.progress ?? null} {tone}><Portrait {agent} size={26} dim={tone === 'off'} /></Orbit>
    {#if subs.length}
      <span class="subs">
        {#each subs.slice(0, 3) as s (s.callId)}
          <button class="sub" aria-label="Subagent: {s.description}" data-tip="Subagent · {s.description}" onclick={openSub}></button>
        {/each}
      </span>
    {/if}
  </span>
  {#if isLead}
    <span class="ln">{agent.name}</span>
  {:else if !compact}
    <span class="txt">
      <span class="who"><span class="name">{agent.name}</span><span class="prov">{agent.consultant ? 'Consultant · ' : agent.member && agent.title ? `${agent.title} · ` : ''}{PROVIDER_LABEL[agent.provider]}</span></span>
      <span class="say truncate">{#if n}<Say parts={n.parts} />{/if}</span>
    </span>
  {/if}
</div>

<style>
  .pill { position: relative; display: flex; align-items: center; gap: 10px; height: 48px; padding: 0 16px 0 6px; min-width: 0; flex: 0 1 280px; max-width: 290px; border-radius: var(--r-full); background: var(--glass); -webkit-backdrop-filter: var(--glass-blur); backdrop-filter: var(--glass-blur); box-shadow: var(--glass-edge), 0 10px 30px -18px rgba(0, 0, 0, 0.6); transition: box-shadow var(--d-3) var(--ease-out), background-color var(--d-3) var(--ease-out), transform var(--d-3) var(--spring); }
  .pill.compact { flex: 0 0 auto; padding: 0 6px; }
  .pill.lead { padding: 0 14px 0 6px; gap: 8px; }
  .ln { position: relative; z-index: 1; pointer-events: none; font-size: var(--t-sm); font-weight: var(--w-semi); color: var(--ink-2); white-space: nowrap; }
  .pill.lead.selected .ln { color: var(--ink-1); }
  .pill.amber { background: color-mix(in srgb, var(--amber) 16%, var(--glass)); box-shadow: var(--glass-edge), inset 0 0 0 1px color-mix(in srgb, var(--amber) 55%, transparent); }
  .pill.amber .ln { color: var(--amber-ink); }
  .hit { position: absolute; inset: 0; border-radius: inherit; z-index: 0; }
  .hit:focus-visible { outline-offset: 1px; border-radius: var(--r-full); }
  .face { position: relative; z-index: 1; pointer-events: none; }
  .txt { position: relative; z-index: 1; pointer-events: none; display: flex; flex-direction: column; gap: 1px; min-width: 0; flex: 1; }
  .who { display: flex; align-items: baseline; gap: 6px; line-height: 16px; }
  .name { font-weight: var(--w-semi); font-size: var(--t-md); color: var(--ink-1); }
  .prov { font-size: var(--t-xs); color: var(--ink-3); white-space: nowrap; }
  .say { display: block; font-size: var(--t-sm); line-height: 17px; color: var(--ink-2); }
  .pill:hover, .pill.lit { box-shadow: var(--glass-edge), inset 0 0 0 1px color-mix(in srgb, var(--hue) 55%, transparent), 0 10px 30px -18px rgba(0, 0, 0, 0.6); }
  .pill.selected { background: color-mix(in srgb, var(--hue) 14%, var(--glass)); box-shadow: var(--glass-edge), inset 0 0 0 1.5px color-mix(in srgb, var(--hue) 75%, transparent); }
  .t-wait { background: color-mix(in srgb, var(--amber) 16%, var(--glass)); box-shadow: var(--glass-edge), inset 0 0 0 1px color-mix(in srgb, var(--amber) 55%, transparent), 0 0 24px -6px color-mix(in srgb, var(--amber) 45%, transparent); }
  .t-wait .say { color: var(--amber-ink); }
  .t-done .say, .t-idle .say, .t-off .say { color: var(--ink-3); }
  .t-blocked .say { color: var(--bad); }
  .subs { position: absolute; right: -4px; bottom: -2px; display: flex; gap: 2px; pointer-events: auto; }
  .sub { width: 9px; height: 9px; border-radius: 50%; background: color-mix(in srgb, var(--hue) 75%, white); box-shadow: 0 0 0 2px var(--glass-solid), 0 0 8px var(--hue); }
</style>
