<!--
  Settings → Developer: cheats for testing the game (docs/design/PROGRESSION.md §3), behind the
  "Developer mode" switch; the core refuses them while it is off. Coins and XP from here are not
  from real work, so the first one flags the wallet for good. Resetting the office only empties
  it of furniture, refunded.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { Wallet } from '../lib/gen/Wallet';
  import { api } from '../lib/api';
  import { updateSettings } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { rankLabel } from '../lib/util/career';
  import { coins } from '../lib/util/format';

  const on = $derived(store.settings.developer_mode);
  let wallet = $state.raw<Wallet | null>(null);
  const load = () => api.wallet().then((w) => (wallet = w)).catch(() => {});
  onMount(() => void load());

  // The agent selected in the office, unless you pick another here.
  const workers = $derived(store.agents.filter((a) => a.role !== 'lead' && !a.archived));
  let picked = $state<string | null>(null);
  const agentId = $derived(
    workers.find((a) => a.id === picked)?.id ?? workers.find((a) => a.id === ui.selectedAgentId)?.id ?? workers[0]?.id ?? null,
  );

  const fail = (title: string) => (e: unknown) => toast({ kind: 'error', title, body: e instanceof Error ? e.message : String(e) });
  function grant(n: number) {
    api.devGrant(n).then((w) => {
      wallet = w;
      toast({ kind: 'success', title: `+${coins(n)}`, body: `${coins(w.balance)} to spend` });
    }, fail('Couldn’t add coins'));
  }
  function giveXp(xp: number) {
    if (!agentId) return;
    api.devGiveXp(agentId, xp).then((a) => {
      void load();
      toast({ kind: 'success', title: `${a.name}: +${xp} XP`, body: `${rankLabel(a.career.rank)} · ${a.career.xp} XP` });
    }, fail('Couldn’t give XP'));
  }
  let confirmReset = $state(false);
  function reset() {
    confirmReset = false;
    api.devResetOffice().then((s) => {
      wallet = s.wallet;
      toast({ kind: 'success', title: 'Office reset', body: 'Its furniture is gone and refunded.' });
    }, fail('Couldn’t reset the office'));
  }
</script>

<section>
  <h3 class="eyebrow">Developer</h3>
  <label class="row toggle">
    <div class="r-text"><span class="r-title">Developer mode</span><span class="desc">Cheats for testing the game: coins, XP, a fresh office. Coins and XP from here aren’t from real work, so using them marks this office as cheated, for good.</span></div>
    <input type="checkbox" class="switch" checked={on} onchange={(e) => updateSettings({ developer_mode: e.currentTarget.checked })} />
  </label>
  {#if wallet?.cheated}
    <p class="desc flag">This office has used cheats.</p>
  {/if}

  {#if on}
    <div class="row">
      <div class="r-text">
        <span class="r-title">Coins</span>
        <span class="desc">{#if wallet}<b>{coins(wallet.balance)}</b> to spend: {coins(wallet.earned)} earned{wallet.granted ? `, ${coins(wallet.granted)} given` : ''}, {coins(wallet.spent)} in furniture.{:else}The wallet, across every repo.{/if}</span>
      </div>
      <button class="btn sm" onclick={() => grant(1000)}>Add {coins(1000)}</button>
      <button class="btn sm" onclick={() => grant(10000)}>Add {coins(10000)}</button>
    </div>

    <div class="row">
      <div class="r-text"><span class="r-title">Give XP</span><span class="desc">Its rank follows (Junior 60, Senior 250, Staff 800), with a “Cheat” milestone.</span></div>
      {#if workers.length}
        <select class="input" aria-label="Agent" value={agentId} onchange={(e) => (picked = e.currentTarget.value)}>
          {#each workers as a (a.id)}<option value={a.id}>{a.name} · {rankLabel(a.career.rank)}</option>{/each}
        </select>
        {#each [60, 250, 800] as xp (xp)}
          <button class="btn sm" onclick={() => giveXp(xp)}>+{xp} XP</button>
        {/each}
      {:else}
        <span class="desc">No agents yet.</span>
      {/if}
    </div>

    <!-- Energy and time (PROGRESSION.md §3, with the energy work): fill or drain the agent's
         energy, skip the home and caffeine timers. Their rows go here. -->

    <div class="row">
      <div class="r-text"><span class="r-title">Reset this office</span><span class="desc">Removes every piece of furniture in this repo’s office; what it cost goes back to the wallet.</span></div>
      {#if confirmReset}
        <button class="btn sm danger" onclick={reset}>Reset</button>
        <button class="btn sm ghost" onclick={() => (confirmReset = false)}>Keep</button>
      {:else}
        <button class="btn sm" disabled={!store.workspace} onclick={() => (confirmReset = true)}>Reset…</button>
      {/if}
    </div>
  {/if}
</section>

<style>
  section { padding: 20px 0; box-shadow: inset 0 -1px 0 var(--line); display: flex; flex-direction: column; gap: 12px; }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .desc b { color: var(--ink-2); font-weight: var(--w-med); }
  .flag { color: var(--amber-ink); }
  .row { display: flex; align-items: center; gap: 8px; min-height: 42px; }
  .r-text { flex: 1; display: flex; flex-direction: column; gap: 2px; margin-right: 8px; }
  .r-title { font-weight: var(--w-med); }
  select { width: auto; max-width: 160px; height: 26px; font-size: var(--t-sm); }
</style>
