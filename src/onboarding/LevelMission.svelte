<!-- Level 5: pick a repository and a first request; the request lands in the Lead's composer, unsent. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import type { RecentWorkspace } from '../lib/gen/RecentWorkspace';
  import { openWorkspace } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { ago } from '../lib/util/format';
  import { tildify } from '../lib/util/sanitize';
  import Icon from '../components/Icon.svelte';
  import { catalog } from './catalog';

  interface Props {
    finish: () => void;
  }
  let { finish }: Props = $props();
  let recent = $state<RecentWorkspace[]>([]);
  let mission = $state(catalog.missions[0].id);
  let opening = $state<string | null>(null);
  onMount(() => {
    api.recentWorkspaces().then((r) => (recent = r.slice(0, 3))).catch(() => {});
  });

  async function go(path?: string) {
    opening = path ?? 'picker';
    const text = catalog.missions.find((m) => m.id === mission)?.text ?? '';
    await openWorkspace(path);
    opening = null;
    if (!store.workspace) return; // cancelled the picker
    finish();
    ui.seedComposer('lead', text); // unsent: you read it, edit it, press ↵
  }
</script>

<div class="missions" role="radiogroup" aria-label="First request">
  {#each catalog.missions as m (m.id)}
    <button class="m" class:on={mission === m.id} role="radio" aria-checked={mission === m.id} onclick={() => (mission = m.id)}>
      <b>{m.title}</b>
      <span>{m.text}</span>
    </button>
  {/each}
</div>

<div class="repos">
  <button class="btn primary" onclick={() => go()} disabled={!!opening}><Icon name="folder" size={14} />{opening === 'picker' ? 'Opening…' : 'Open a repository'}</button>
  {#each recent as r (r.repo_path)}
    <button class="r" onclick={() => go(r.repo_path)} disabled={!!opening}>
      <span class="r-name serif">{r.name}</span>
      <span class="r-path truncate">{tildify(r.repo_path)}</span>
      <span class="r-when num">{opening === r.repo_path ? 'Opening…' : ago(r.opened_at, ui.now)}</span>
    </button>
  {/each}
</div>
<p class="fine">Nothing is sent: the request waits in the Lead's composer for you to read and press ↵. On macOS, a repository in Documents may ask for folder access the first time.</p>

<style>
  .missions { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 10px; }
  .m { display: flex; flex-direction: column; gap: 6px; padding: 14px; text-align: left; border-radius: var(--r-lg); background: var(--surface-1); box-shadow: inset 0 0 0 1px var(--line); transition: box-shadow var(--d-3), transform var(--d-3) var(--spring); }
  .m:hover { transform: translateY(-2px); }
  .m.on { box-shadow: inset 0 0 0 1.5px var(--aurora-m), 0 6px 20px color-mix(in srgb, var(--aurora-m) 22%, transparent); }
  .m b { font-size: 14.5px; font-weight: var(--w-semi); }
  .m span { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .repos { display: flex; flex-direction: column; gap: 4px; margin-top: 6px; align-items: flex-start; }
  .repos .btn { margin-bottom: 8px; }
  .r { display: flex; align-items: baseline; gap: 14px; width: 100%; padding: 10px 4px; text-align: left; color: var(--ink-3); box-shadow: inset 0 -1px 0 var(--line); }
  .r:hover { color: var(--ink-1); }
  .r-name { font-size: 19px; color: var(--ink-1); font-style: italic; white-space: nowrap; }
  .r-path { flex: 1; font-family: var(--font-mono); font-size: 11.5px; }
  .r-when { font-size: var(--t-sm); white-space: nowrap; }
  .fine { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
</style>
