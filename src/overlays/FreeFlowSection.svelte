<!--
  Settings → Free Flow: dictate into the composer. Off until you turn it on and add a Groq key.
  The key is stored in the macOS Keychain and never shown again: this screen only knows
  "key saved". Audio is sent to Groq only while you record, and nowhere else.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import type { DictationStatus } from '../lib/gen/DictationStatus';
  import { freeflow } from '../lib/state/freeflow.svelte';
  import { toast } from '../lib/state/toasts.svelte';

  const st = $derived(freeflow.status);
  let key = $state('');
  let saving = $state(false);

  onMount(() => void freeflow.load());

  /** Run one dictation command and keep the shared status in step; errors become a toast. */
  async function run(f: () => Promise<DictationStatus>, failed: string): Promise<boolean> {
    try {
      freeflow.set(await f());
      return true;
    } catch (e) {
      toast({ kind: 'error', title: failed, body: String((e as Error)?.message ?? e) });
      return false;
    }
  }

  async function saveKey() {
    if (!key.trim()) return;
    saving = true;
    if (await run(() => api.dictationSaveKey(key), 'Could not save the key')) key = ''; // gone from the screen at once
    saving = false;
  }
</script>

<section>
  <h3 class="eyebrow">Free Flow</h3>
  <p class="desc">Dictate into the message box: click the mic, or hold ⌥ while it is focused. Your clip goes to Groq’s Whisper with your own key, and the transcript lands in the draft for you to review. It is never sent for you.</p>
  {#if st}
    <label class="row toggle">
      <div class="r-text"><span class="r-title">Dictation</span><span class="desc">The mic appears in the message box once a key is saved.</span></div>
      <input type="checkbox" class="switch" checked={st.enabled} onchange={(e) => run(() => api.dictationConfigure(e.currentTarget.checked, null), 'Could not change Free Flow')} />
    </label>

    <div class="row">
      <div class="r-text">
        <span class="r-title">Groq API key</span>
        <span class="desc">
          {#if !st.secure_storage}Secure key storage is only available on macOS in this build.
          {:else if st.key_saved}<span class="saved">Key saved</span> in the macOS Keychain. It is never shown again.
          {:else}Create one at console.groq.com. It is kept in the macOS Keychain, not in a file.{/if}
        </span>
      </div>
      {#if st.key_saved}
        <button class="btn" onclick={() => run(() => api.dictationClearKey(), 'Could not remove the key')}>Remove</button>
      {/if}
    </div>
    {#if st.secure_storage}
      <form class="row" onsubmit={(e) => { e.preventDefault(); void saveKey(); }}>
        <input class="field wide" type="password" bind:value={key} placeholder={st.key_saved ? 'Replace the saved key' : 'gsk_…'} spellcheck="false" autocomplete="off" aria-label="Groq API key" />
        <button class="btn" type="submit" disabled={!key.trim() || saving}>{saving ? 'Saving…' : 'Save key'}</button>
      </form>
    {/if}

    <div class="row">
      <div class="r-text"><span class="r-title">Model</span><span class="desc">Turbo is fast and cheap; the full model is a little more accurate.</span></div>
      <select class="field" value={st.model} aria-label="Whisper model" onchange={(e) => run(() => api.dictationConfigure(null, e.currentTarget.value), 'Could not change the model')}>
        {#each st.models as m (m)}<option value={m}>{m}</option>{/each}
      </select>
    </div>
    <p class="desc">Clips are capped at two minutes, recorded only while you hold or toggle the mic, and sent only to api.groq.com.</p>
  {:else}
    <p class="desc">Not available in this build.</p>
  {/if}
</section>

<style>
  section { padding: 20px 0; box-shadow: inset 0 -1px 0 var(--line); display: flex; flex-direction: column; gap: 12px; }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .row { display: flex; align-items: center; gap: 16px; min-height: 42px; }
  .r-text { flex: 1; display: flex; flex-direction: column; gap: 2px; }
  .r-title { font-weight: var(--w-med); }
  .saved { color: var(--ink-1); font-weight: var(--w-med); }
  .field { width: 180px; height: 32px; padding: 0 10px; border-radius: var(--r-md); background: var(--surface-1); color: var(--ink-1); box-shadow: inset 0 0 0 1px var(--line); font: inherit; font-size: var(--t-sm); outline: none; }
  .field.wide { flex: 1; width: auto; }
  .field:focus { box-shadow: inset 0 0 0 1px var(--ink-3); }
  .btn { height: 32px; padding: 0 14px; border-radius: var(--r-full); background: var(--surface-2); font-weight: var(--w-med); font-size: var(--t-sm); }
  .btn:hover:not(:disabled) { background: var(--surface-3); }
  .btn:disabled { opacity: 0.5; }
</style>
