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
  import { onMount, untrack } from 'svelte';
  import { api } from '../lib/api';
  import type { RequiredChecks } from '../lib/gen/RequiredChecks';
  import { Loader } from '../lib/state/loader.svelte';
  import { store } from '../lib/state/store.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import { checksTurnTrouble, setupChoices } from '../lib/util/worktree';
  import LoadError from '../components/LoadError.svelte';
  import MoreAbout from './MoreAbout.svelte';

  /** Called with each value the section shows, as it is read or saved: the line under the
   *  page's name in the list follows it. */
  let { onvalue }: { onvalue?: (checks: RequiredChecks) => void } = $props();

  const checks = new Loader<RequiredChecks>(() => api.requiredChecks());
  const s = $derived(checks.value);
  $effect(() => {
    const now = s;
    if (now) untrack(() => onvalue?.(now));
  });
  onMount(() => {
    if (store.workspace) void checks.load();
  });

  /** Your choice is on its way to the core: the switch and the buttons wait for its answer, so
   *  a second click does not send it twice. */
  let busy = $state(false);

  /** Turn the checks on for the text shown (`allow` = its fingerprint) or off (null). The
   *  section shows what the core saved. After an error the checks are read again, and the toast
   *  says what is true: a refusal (the file changed meanwhile) saved nothing, but an error that
   *  came after the choice was saved did change them. */
  async function turn(allow: string | null) {
    if (busy) return;
    busy = true;
    try {
      checks.set(await api.requiredChecks({ allow }));
    } catch (e) {
      const read = await checks.load();
      toast({ kind: 'error', title: checksTurnTrouble(allow, read ? checks.value : null), body: e instanceof Error ? e.message : String(e) });
    } finally {
      busy = false;
    }
  }

  /** The switch: it shows what the core has when the answer is there, also after a refusal
   *  (the click moved it, and nothing else would move it back). */
  async function flip(box: HTMLInputElement, on: string | null) {
    await turn(box.checked ? on : null);
    box.checked = checks.value?.on ?? false;
  }
</script>

{#if store.workspace}
  <section>
    <h3 class="eyebrow">Required checks</h3>
    <p class="desc">
      The Lead writes the checks of each task. Required checks are yours: commands that must pass for every task and every finished plan in
      {store.workspace.name}, whatever the plan says.
    </p>
    <MoreAbout id="required-checks">
      <p class="desc">
        Write them in a <code>.codly/checks</code> file at the top of the repo, one command on each line (a line that starts with <code>#</code> is a
        comment), for example <code>pnpm check</code> and <code>pnpm test</code>.
      </p>
    </MoreAbout>
    {#if s}
      {@const can = setupChoices(s)}
      <!-- Not a <label>: only the switch itself changes it, never a click on the text. -->
      <div class="row">
        <div class="r-text">
          <span class="r-title">Run the repo’s required checks</span>
          <span class="desc">
            {#if s.problem}
              Codly does not read this repo’s checks file: {s.problem}.
            {:else if s.text === null}
              This repo has no <code>.codly/checks</code> with a command in it.
            {:else}
              Codly reads <code>.codly/checks</code> from your checkout and runs each command in the task’s worktree, after the task’s own checks, and again on
              the finished plan. A command that fails goes back to the worker like any failed check.
            {/if}
          </span>
          {#if !s.problem && s.text !== null}
            <MoreAbout id="required-checks-run">
              <span class="desc">For an agent you chat with, its Git tab has Check the work. Each command has the time limit for checks above.</span>
            </MoreAbout>
          {/if}
        </div>
        {#if busy && !can.ask}<span class="busy" role="status">Saving…</span>{/if}
        <input
          type="checkbox"
          class="switch"
          checked={s.on}
          disabled={busy || can.ask || (!can.on && !can.off)}
          aria-label="Run the repo’s required checks"
          onchange={(e) => flip(e.currentTarget, can.on)}
        />
      </div>
      {#if can.ask}
        <div class="changed" role="status">
          <p>
            {#if s.text === null}
              You turned the required checks on, and Codly cannot read their file now. No task passes its checks until you fix the file or turn the checks off.
            {:else}
              The checks file changed since you turned it on. Codly does not run it, and no task passes its checks, until you read it and turn it on again, or turn it off.
            {/if}
            When you turn the checks on or off here, Codly checks the waiting tasks again by itself. A task whose own check needs your OK stays where it is.
          </p>
          <div class="choices">
            {#if can.on}<button class="btn sm" disabled={busy} onclick={() => turn(can.on)}>Turn on this text</button>{/if}
            <button class="btn sm" disabled={busy} onclick={() => turn(null)}>Turn off</button>
            {#if busy}<span class="busy">Saving…</span>{/if}
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
        </p>
        <MoreAbout id="required-checks-text">
          <p class="desc">You turn on these commands as they are written now: if the file changes, they are off until Codly has asked you again.</p>
        </MoreAbout>
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
  .choices { display: flex; align-items: center; gap: 8px; }
  .busy { flex: none; font-size: var(--t-sm); color: var(--ink-3); }
  .cmds { display: flex; flex-direction: column; gap: 4px; padding: 10px 12px; border-radius: var(--r-md); background: var(--surface-1); font-size: var(--t-sm); line-height: var(--lh-sm); }
  .cmds li { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; }
  .cmds code { overflow-wrap: anywhere; }
  .risk { flex: none; color: var(--amber-ink); }
</style>
