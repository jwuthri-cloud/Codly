<!--
  A small chip in the corner while the Lead hires a consultant: the title, applicants and a
  countdown. Click it to choose who gets the job; when it runs out, the Lead's pick (the first)
  is hired and walks in through the front door.
-->
<script lang="ts">
  import type { JobOffer } from '../lib/gen/JobOffer';
  import { modal } from '../lib/util/modal';
  import ApplicantPicker from './ApplicantPicker.svelte';

  interface Props {
    offers: JobOffer[];
    insets: { top: number; right: number; bottom: number; left: number };
    onpick?: (offerId: string, index: number) => void;
  }
  let { offers, insets, onpick }: Props = $props();

  let now = $state(Date.now());
  let open = $state<string | null>(null);
  $effect(() => {
    if (!offers.length) return;
    const t = setInterval(() => (now = Date.now()), 500);
    return () => clearInterval(t);
  });
  const shown = $derived(offers[0] ?? null);
  const dialog = $derived(offers.find((o) => o.id === open && o.hired == null) ?? null);
  const left = (o: JobOffer) => Math.max(0, Math.ceil((o.closes_at - now) / 1000));
  const hiredName = (o: JobOffer) => (o.hired != null ? o.applicants[o.hired]?.name : null);
</script>

{#if shown}
  {@const hired = hiredName(shown)}
  <button
    class="job"
    class:hired
    style:top={`${insets.top + 8}px`}
    style:right={`${insets.right + 12}px`}
    disabled={!!hired || !onpick}
    onclick={() => (open = shown.id)}
    aria-label={hired ? `${hired} was hired` : `Job offer: ${shown.title ?? 'Consultant'}. Choose who gets the job.`}
  >
    {#if hired}
      <b>{hired}</b> hired as {shown.title ?? 'Consultant'}
    {:else}
      Hiring <b>{shown.title ?? 'Consultant'}</b> · {shown.applicants.length} applicants · {left(shown)}s
      {#if onpick}<span class="cta">Choose</span>{/if}
    {/if}
  </button>
{/if}

{#if dialog && onpick}
  <div class="scrim" role="presentation" onclick={() => (open = null)}></div>
  <div class="dlg" role="dialog" aria-modal="true" aria-label="Choose who gets the job" use:modal={() => (open = null)}>
    <header>
      <div>
        <div class="kicker">Now hiring · closes in {left(dialog)}s</div>
        <h3>{dialog.title ?? 'Consultant'} <small>{dialog.provider}{dialog.model ? ` · ${dialog.model}` : ''}</small></h3>
      </div>
      <button class="x" aria-label="Close" onclick={() => (open = null)}>✕</button>
    </header>
    <ApplicantPicker
      applicants={dialog.applicants}
      provider={dialog.provider}
      picked={0}
      firstNote="The Lead's pick"
      onpick={(i) => {
        onpick(dialog.id, i);
        open = null;
      }}
    />
    <p class="note">Green traits change how the agent works; the others are personality.</p>
  </div>
{/if}

<style>
  .job { position: absolute; z-index: 400; display: flex; align-items: center; gap: 6px; padding: 6px 8px 6px 12px; border: 0; border-radius: 999px; background: rgba(22, 26, 36, 0.92); color: #eef1f7; box-shadow: 0 8px 20px -10px rgba(0, 0, 0, 0.6), inset 0 0 0 1px rgba(255, 255, 255, 0.1); font: 500 12px/1.3 'Inter Variable', Inter, system-ui, sans-serif; cursor: pointer; animation: pin 0.25s ease-out both; }
  .job:hover:not(:disabled) { background: rgba(34, 40, 54, 0.96); }
  .job:disabled { cursor: default; }
  .kicker { font: 700 10px/1 ui-monospace, monospace; letter-spacing: 0.08em; text-transform: uppercase; opacity: 0.6; }
  .cta { padding: 2px 10px; border-radius: 999px; background: #eef1f7; color: #161a24; font-weight: 600; }
  .scrim { position: absolute; inset: 0; z-index: 900; background: rgba(8, 10, 16, 0.45); }
  .dlg { position: absolute; z-index: 901; left: 50%; top: 50%; transform: translate(-50%, -50%); width: min(620px, calc(100% - 32px)); padding: 16px; border-radius: 18px; background: rgba(22, 26, 36, 0.96); color: #eef1f7; box-shadow: 0 30px 80px -30px rgba(0, 0, 0, 0.8), inset 0 0 0 1px rgba(255, 255, 255, 0.08); font: 500 13px 'Inter Variable', Inter, system-ui, sans-serif; }
  .dlg header { display: flex; justify-content: space-between; align-items: start; margin-bottom: 12px; }
  .dlg h3 { margin: 4px 0 0; font-size: 18px; }
  .dlg small { font-size: 12px; opacity: 0.6; font-weight: 500; }
  .x { border: 0; background: none; color: #9aa2b6; font-size: 16px; cursor: pointer; }
  .note { margin: 10px 0 0; font-size: 11.5px; opacity: 0.6; }
  @keyframes pin { from { opacity: 0; transform: translateY(-8px); } }
  @media (prefers-reduced-motion: reduce) { .job { animation: none; } }
</style>
