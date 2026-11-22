<!--
  Settings → Safety: the rules saved from approval cards ("Always allow in this repo"). Codly
  keeps them in its own store and follows them for every agent of the open repo. Removing a
  rule makes Codly ask again. A rule is only ever added from a card, never typed here.
-->
<script lang="ts">
  import type { ApprovalRule } from '../lib/gen/ApprovalRule';
  import { api } from '../lib/api';
  import { store } from '../lib/state/store.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import Icon from '../components/Icon.svelte';

  let rules = $state<ApprovalRule[] | null>(null);

  async function load() {
    try {
      rules = await api.approvalRules();
    } catch {
      rules = null; // no repo open: the section stays hidden
    }
  }
  // The rules are the open repo's: read them again when you switch repos.
  $effect(() => {
    void store.workspace?.id;
    void load();
  });

  async function remove(rule: ApprovalRule) {
    try {
      await api.removeApprovalRule(rule.id);
    } catch (e) {
      toast({ kind: 'error', title: 'Couldn’t remove the rule', body: e instanceof Error ? e.message : String(e) });
    }
    await load();
  }
</script>

{#if rules}
  <section>
    <h3 class="eyebrow">Always allowed in this repo</h3>
    <p class="desc">
      Commands you saved from an approval card with “Always allow in this repo”. Codly approves them for every agent in
      {store.workspace?.name ?? 'this repo'} without asking. It still asks about deletes, <code>git push</code>, <code>sudo</code>, secrets and
      anything outside the agent’s worktree, and about everything in Strict mode. Remove a rule and Codly asks again.
    </p>
    {#if rules.length}
      <ul class="rules">
        {#each rules as r (r.id)}
          <li>
            <span class="kind">{r.kind === 'prefix' ? 'Starts with' : 'Exactly'}</span>
            <code class="selectable">{r.command}</code>
            <button class="btn sm ghost rm" aria-label={`Remove the rule for ${r.command}`} onclick={() => remove(r)}><Icon name="trash" size={13} />Remove</button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="desc">No rules yet. An approval card offers “Always allow in this repo” when its command can be saved.</p>
    {/if}
  </section>
{/if}

<style>
  section { padding: 20px 0; box-shadow: inset 0 -1px 0 var(--line); display: flex; flex-direction: column; gap: 12px; }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .desc code { font-family: var(--font-mono); font-size: 11.5px; color: var(--ink-2); }
  .rules { display: flex; flex-direction: column; border-radius: var(--r-md); background: var(--surface-1); }
  .rules li { display: flex; align-items: center; gap: 10px; min-height: 42px; padding: 8px 14px; min-width: 0; }
  .rules li + li { box-shadow: inset 0 1px 0 var(--line); }
  .kind { flex-shrink: 0; width: 76px; font-size: var(--t-sm); color: var(--ink-3); }
  .rules code { flex: 1; min-width: 0; font-family: var(--font-mono); font-size: 12px; line-height: 18px; color: var(--ink-1); white-space: pre-wrap; word-break: break-all; }
</style>
