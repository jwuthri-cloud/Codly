<!--
  "From issue…" / "From pull request…": asks for a GitHub link or a number, and for what you
  want to add in your own words. Codly reads it with your own gh sign-in and gives it to a
  request's Lead; the text from GitHub is quoted for the Lead as data. When it cannot be read,
  the dialog stays open and says why, and nothing is sent. While the read runs the dialog cannot
  be closed: the read cannot be stopped, and its end is told here or in the request it opens.
-->
<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { intakeUi, startFromGithub } from '../lib/state/intake.svelte';
  import Dialog from '../components/Dialog.svelte';

  const kind = intakeUi.dialog!;
  const pull = kind === 'pull_request';
  let reference = $state('');
  let note = $state('');
  const busy = $derived(intakeUi.busy);
  let problem = $state<string | null>(null);
  let field: HTMLInputElement | undefined = $state();
  const close = () => intakeUi.close();
  onMount(() => field?.focus());

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (busy || !reference.trim()) return;
    problem = await startFromGithub(kind, reference, note);
    if (!problem) return close();
    // The field was disabled while the read ran, and a disabled field loses the focus: it goes
    // back there, where the reference is corrected.
    await tick();
    field?.focus();
  }
</script>

<Dialog label={pull ? 'Start from a pull request' : 'Start from an issue'} width={540} onclose={close}>
  <form onsubmit={submit}>
    <div class="body">
      <p class="desc">
        Codly reads the {pull ? 'pull request' : 'issue'} with your own gh sign-in and gives it to a request’s Lead. The Lead gets the text
        from GitHub as quoted data, not as your instructions. Nothing is written to GitHub.
      </p>
      <label class="field">
        <span class="eyebrow">Link or number</span>
        <input bind:this={field} bind:value={reference} placeholder={pull ? 'https://github.com/owner/repo/pull/12, or 12' : 'https://github.com/owner/repo/issues/12, or 12'} spellcheck="false" autocomplete="off" disabled={busy} />
      </label>
      <label class="field">
        <span class="eyebrow">Anything to add (optional)</span>
        <textarea bind:value={note} rows="3" placeholder="Your own words for the Lead" disabled={busy}></textarea>
      </label>
      {#if pull}
        <p class="hint">
          The Lead also gets the reviews, the comments on the code and the failing checks. A plan starts from the branch you have checked out: to
          work on top of the pull request, check out its branch first.
        </p>
      {/if}
      {#if problem}<p class="note" role="alert">{problem}</p>{/if}
    </div>
    <footer>
      <button type="button" class="btn sm ghost" disabled={busy} onclick={close}>Cancel</button>
      <button type="submit" class="btn sm primary" disabled={busy || !reference.trim()}>{busy ? 'Reading from GitHub…' : 'Start'}</button>
    </footer>
  </form>
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: 14px; padding: 22px 26px; }
  .desc { color: var(--ink-2); font-size: var(--t-md); }
  .field { display: flex; flex-direction: column; gap: 8px; }
  .hint { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .note { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--amber-ink, var(--ink-2)); overflow-wrap: anywhere; }
  footer { display: flex; justify-content: flex-end; gap: 8px; padding: 0 26px 22px; }
</style>
