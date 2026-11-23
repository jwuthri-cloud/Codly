<!--
  Settings → Safety: the repo's required checks, `.codly/checks`. One command on each line; Codly
  runs them for every task after the plan's own verify commands, for every finished plan, and
  from an agent's Git tab (Check the work). The file is text from the repo: off until you turn
  it on here, where each command and what the approval rules see in it are shown. Turning it on
  is for that exact text. When the file is not that text any more, two buttons replace the
  switch: turn the new text on, or turn the checks off. Either choice lets the tasks that waited
  for it go on: the core checks them again.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import type { RequiredChecks } from '../lib/gen/RequiredChecks';
  import { Loader } from '../lib/state/loader.svelte';
  import { store } from '../lib/state/store.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import { setupChoices } from '../lib/util/worktree';
  import LoadError from '../components/LoadError.svelte';

  const checks = new Loader<RequiredChecks>(() => api.requiredChecks());
  const s = $derived(checks.value);
  onMount(() => {
    if (store.workspace) void checks.load();
  });

  /** Turn the checks on for the text shown (`allow` = its fingerprint) or off (null). The
   *  section shows what the core saved; a refusal (the file changed meanwhile) reloads it. */
  async function turn(allow: string | null) {
    try {
      checks.set(await api.requiredChecks({ allow }));
    } catch (e) {
      toast({ kind: 'error', title: 'Couldn’t change the required checks', body: e instanceof Error ? e.message : String(e) });
      void checks.load();
    }
  }
</script>

{#if store.workspace}
  <section>
    <h3 class="eyebrow">Required checks</h3>
    <p class="desc">
      The Lead writes the checks of each task. Required checks are yours: commands that must pass for every task and every finished plan in
      {store.workspace.name}, whatever the plan says. Write them in a <code>.codly/checks</code> file at the top of the repo, one command on each line
      (a line that starts with <code>#</code> is a comment), for example <code>pnpm check</code> and <code>pnpm test</code>.
    </p>
    {#if s}
      {@const can = setupChoices(s)}
      <label class="row toggle">
        <div class="r-text">
          <span class="r-title">Run the repo’s required checks</span>
          <span class="desc">
            {#if s.problem}
              Codly does not read this repo’s checks file: {s.problem}.
            {:else if s.text === null}
              This repo has no <code>.codly/checks</code> with a command in it.
            {:else}
              Codly reads <code>.codly/checks</code> from your checkout and runs each command in the task’s worktree, after the task’s own checks, and again on
              the finished plan. A command that fails goes back to the worker like any failed check. For an agent you chat with, its Git tab has Check the work.
              Each command has the time limit for checks above.
            {/if}
          </span>
        </div>
        <input
          type="checkbox"
          class="switch"
          checked={s.on}
          disabled={can.ask || (!can.on && !can.off)}
          aria-label="Run the repo’s required checks"
          onchange={(e) => turn(e.currentTarget.checked ? can.on : null)}
        />
      </label>
      {#if can.ask}
        <div class="changed" role="status">
          <p>
            {#if s.text === null}
              You turned the required checks on, and Codly cannot read their file now. No task passes its checks until you fix the file or turn the checks off.
            {:else}
              The checks file changed since you turned it on. Codly does not run it, and no task passes its checks, until you read it and turn it on again, or turn it off.
            {/if}
            When you turn the checks on or off here, Codly checks the tasks that waited again by itself.
          </p>
          <div class="choices">
            {#if can.on}<button class="btn sm" onclick={() => turn(can.on)}>Turn on this text</button>{/if}
            <button class="btn sm" onclick={() => turn(null)}>Turn off</button>
          </div>
        </div>
      {/if}
      {#if s.commands.length}
        <ul class="cmds" aria-label="The required checks">
          {#each s.commands as c (c.command)}
            <li><code>{c.command}</code>{#if c.risks.length}<span class="risk">{c.risks.join(', ')}</span>{/if}</li>
          {/each}
        </ul>
        <p class="desc">
          These commands come from the repo, and once they are on they run without asking again, also the ones with a risk shown beside them.
          Turning them on is for this text only: if the file changes, Codly asks you again.
        </p>
      {/if}
    {:else if checks.error}
      <LoadError what="the required checks" error={checks.error} loading={checks.loading} onretry={() => checks.load()} />
    {:else}
      <p class="desc">Loading…</p>
    {/if}
  </section>
{/if}

<style>
  section { padding: 20px 0; box-shadow: inset 0 -1px 0 var(--line); display: flex; flex-direction: column; gap: 12px; }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .row { display: flex; align-items: center; gap: 16px; min-height: 42px; }
  .r-text { flex: 1; display: flex; flex-direction: column; gap: 2px; }
  .r-title { font-weight: var(--w-med); }
  .changed { display: flex; flex-direction: column; gap: 8px; font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--bad); }
  .choices { display: flex; gap: 8px; }
  .cmds { display: flex; flex-direction: column; gap: 4px; padding: 10px 12px; border-radius: var(--r-md); background: var(--surface-1); font-size: var(--t-sm); line-height: var(--lh-sm); }
  .cmds li { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; }
  .cmds code { overflow-wrap: anywhere; }
  .risk { flex: none; color: var(--amber-ink); }
</style>
