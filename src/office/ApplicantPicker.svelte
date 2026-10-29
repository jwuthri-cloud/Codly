<!--
  Applicants for a job, side by side (Create-a-Sim style): portrait, name, pitch and traits.
  A trait that changes how the agent works is marked and says how. Used by the office's job
  offer and the add-agent dialog.
-->
<script lang="ts">
  import type { Applicant } from '../lib/gen/Applicant';
  import type { Provider } from '../lib/gen/Provider';
  import type { Role } from '../lib/gen/Role';
  import { TRAITS } from '../lib/util/career';
  import { agentPortrait } from './portrait';

  interface Props {
    applicants: Applicant[];
    provider: Provider;
    role?: Role;
    /** Index of the highlighted one (e.g. the Lead's pick, or the current choice). */
    picked?: number | null;
    /** Label under the first card. */
    firstNote?: string;
    onpick: (index: number) => void;
  }
  let { applicants, provider, role = 'worker', picked = null, firstNote, onpick }: Props = $props();
  const trait = (id: string) => TRAITS.find((t) => t.id === id);
</script>

<div class="apps" role="listbox" aria-label="Applicants">
  {#each applicants as a, i (a.name)}
    <button class="app" class:on={picked === i} role="option" aria-selected={picked === i} onclick={() => onpick(i)}>
      <img src={agentPortrait({ id: a.name, color: a.color, look_seed: a.avatar, provider, role }, 72)} alt="" width="72" height="72" />
      <b>{a.name}</b>
      <q>{a.pitch}</q>
      <span class="traits">
        {#each a.traits as t (t)}
          {@const tr = trait(t)}
          <span class:work={tr?.rule} title={tr?.rule ? `Changes how it works: ${tr.rule}` : 'Personality only'}>{tr?.label ?? t}</span>
        {/each}
      </span>
      {#if i === 0 && firstNote}<small>{firstNote}</small>{/if}
    </button>
  {/each}
</div>

<style>
  .apps { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 10px; }
  .app { display: flex; flex-direction: column; align-items: center; gap: 6px; padding: 12px 10px; border: 0; border-radius: 14px; background: var(--surface-2, rgba(255, 255, 255, 0.05)); box-shadow: inset 0 0 0 1px var(--line, rgba(255, 255, 255, 0.1)); color: inherit; font: inherit; text-align: center; cursor: pointer; transition: transform 0.12s, box-shadow 0.12s; }
  .app:hover { transform: translateY(-2px); }
  .app.on { box-shadow: inset 0 0 0 2px #e9b03a; }
  .app img { border-radius: 14px; background: rgba(0, 0, 0, 0.15); }
  .app b { font-size: 14px; }
  .app q { font-size: 12px; opacity: 0.75; quotes: '“' '”'; }
  .traits { display: flex; flex-wrap: wrap; gap: 4px; justify-content: center; }
  .traits span { padding: 1px 7px; border-radius: 999px; background: rgba(98, 182, 247, 0.14); color: #6aa7e0; font-size: 11px; }
  .traits span.work { background: rgba(79, 220, 123, 0.14); color: #2f9e57; box-shadow: inset 0 0 0 1px rgba(79, 220, 123, 0.45); }
  small { font-size: 11px; opacity: 0.6; }
  @media (prefers-reduced-motion: reduce) { .app { transition: none; } }
</style>
