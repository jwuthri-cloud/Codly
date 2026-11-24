<!--
  Something went wrong in the agent's turn, said where it happened: what (in plain words), the
  agent's own reason in full, and what you can do: send your message again, copy the reason,
  or open the agent's history at that event. A CLI that is not signed in gets "Sign in…": the
  sign-in terminal of the first-run screen, under the row, with Codly's own command for that
  CLI. A turn you stopped yourself gets "Continue" while nothing ran after it (troubleHelp.ts).
-->
<script lang="ts">
  import { api } from '../../lib/api';
  import { accounts } from '../../lib/state/accounts.svelte';
  import { continueAgent, copyText } from '../../lib/state/actions';
  import { yourStops } from '../../lib/state/stops';
  import { store } from '../../lib/state/store.svelte';
  import { toast } from '../../lib/state/toasts.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { clock } from '../../lib/util/format';
  import { clean } from '../../lib/util/sanitize';
  import Icon from '../../components/Icon.svelte';
  import InstallTerminal from '../../onboarding/InstallTerminal.svelte';
  import { troubleTitle, type TroubleKind } from '../outcomes';
  import { canContinue, signInCommand } from '../troubleHelp';

  interface Props {
    agentId: string;
    eventId: number;
    ts: number;
    what: TroubleKind;
    reason: string | null;
    /** Send your message again (absent: nothing to send again here). */
    onretry?: () => void;
  }
  let { agentId, eventId, ts, what, reason, onretry }: Props = $props();
  const name = $derived(store.agentMap.get(agentId)?.name ?? 'The agent');
  const why = $derived(clean(reason).trim());
  const tone = $derived(what === 'failed' || what === 'stopped' ? 'bad' : what === 'interrupted' ? 'quiet' : 'warn');

  /** The sign-in the reason asks for, with a command Codly knows for that CLI. */
  const signIn = $derived(
    signInCommand(why, (provider) => [store.clis.find((c) => c.provider === provider)?.install.login ?? null, ...accounts.list.filter((a) => a.provider === provider).map((a) => a.login_command)]),
  );
  const continues = $derived.by(() => {
    void store.rev;
    return canContinue(what, ts, store.live[agentId]?.turns ?? [], yourStops(agentId));
  });

  /** The sign-in terminal under the row (one at a time; it closes with the row). */
  let run = $state<{ ptyId: string; command: string } | null>(null);
  function setRun(next: typeof run) {
    if (run) void api.ptyClose(run.ptyId).catch(() => {}); // already gone is fine
    run = next;
  }
  $effect(() => () => setRun(null));
  async function startSignIn(command: string) {
    setRun(null);
    try {
      const id = await api.ptyOpen(null);
      setRun({ ptyId: id, command });
      await api.ptyWrite(id, command + '\n');
    } catch (e) {
      toast({ kind: 'error', title: 'Could not open a terminal', body: e instanceof Error ? e.message : String(e) });
    }
  }
</script>

<div class="trouble t-{tone}">
  <Icon name={what === 'interrupted' ? 'pause' : 'alert'} size={14} />
  <div class="body">
    <p class="head"><b>{troubleTitle(what, name)}</b><time class="num">{clock(ts)}</time></p>
    {#if why}<p class="why selectable">{why}</p>{/if}
    <div class="acts">
      {#if signIn}{@const command = signIn.command}<button class="act" onclick={() => startSignIn(command)} data-tip={command}>Sign in…</button>{/if}
      {#if onretry}<button class="act" onclick={onretry}>Retry</button>{/if}
      {#if continues}<button class="act" onclick={() => continueAgent(agentId)} data-tip="Sends “Continue from where you stopped.”">Continue</button>{/if}
      {#if why}<button class="act" onclick={() => copyText(why, 'Error copied')}>Copy</button>{/if}
      <button class="act" onclick={() => ui.selectAgent(agentId, eventId)}>Details</button>
    </div>
    {#if run}
      <InstallTerminal ptyId={run.ptyId} />
      <p class="fine"><code>{run.command}</code> · When it says you are signed in, send your message again.</p>
    {/if}
  </div>
</div>

<style>
  .trouble { display: flex; align-items: flex-start; gap: 10px; padding: 10px 14px; border-radius: var(--r-md); background: var(--surface-2); box-shadow: inset 0 0 0 1px var(--line); color: var(--ink-3); }
  .trouble > :global(svg) { flex-shrink: 0; margin-top: 2px; }
  .t-bad { background: var(--bad-soft); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--bad) 40%, transparent); color: var(--bad); }
  .t-warn { color: var(--amber-ink); }
  .body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .head { display: flex; align-items: baseline; gap: 8px; font-size: var(--t-md); }
  .head b { font-weight: var(--w-semi); color: var(--ink-1); }
  .head time { font-size: var(--t-xs); color: var(--ink-3); }
  .why { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-2); white-space: pre-wrap; overflow-wrap: anywhere; }
  .acts { display: flex; gap: 4px; margin: 2px 0 0 -8px; }
  .act { height: 24px; padding: 0 8px; border-radius: var(--r-full); font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-2); }
  .act:hover { background: var(--surface-3); color: var(--ink-1); }
  .fine { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .fine code { font-size: 11px; padding: 1px 5px; border-radius: 4px; background: var(--surface-2); color: var(--ink-1); }
</style>
