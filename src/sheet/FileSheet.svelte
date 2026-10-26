<!-- A file from a diff, the office or search: who touched it and why, and how it changed. -->
<script lang="ts">
  import { copyText } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { ago } from '../lib/util/format';
  import { basename, dirname } from '../lib/util/sanitize';
  import Icon from '../components/Icon.svelte';
  import Portrait from '../components/Portrait.svelte';
  import Provenance from '../components/Provenance.svelte';

  interface Props {
    path: string;
  }
  let { path }: Props = $props();
  const touches = $derived.by(() => {
    void store.rev;
    return store.agents
      .map((a) => ({ agent: a, f: store.live[a.id]?.files[path] ?? null }))
      .filter((x) => x.f)
      .sort((a, b) => b.f!.lastTs - a.f!.lastTs);
  });
  const tasks = $derived.by(() => {
    void store.rev;
    return store.tasks.filter((t) => (store.taskMarks[t.id] ?? []).some((m) => m.files?.includes(path)));
  });
  const kgs = $derived(store.triples.filter((t) => t.valid_to == null && (t.subject === `file:${path}` || t.object === `file:${path}`)).slice(0, 8));
  const label = (e: string) => {
    const [type, ...rest] = e.split(':');
    const v = rest.join(':');
    if (type === 'agent') return store.agentMap.get(v)?.name ?? 'agent';
    if (type === 'task') return store.taskMap.get(v)?.key ?? 'task';
    if (type === 'file') return basename(v);
    return v;
  };
</script>

<div class="fs">
  <header>
    <p class="eyebrow">File</p>
    <h2 class="serif">{basename(path)}</h2>
    <p class="dir mono">{dirname(path) || '/'}</p>
    <div class="acts">
      <button class="btn sm ghost" onclick={() => copyText(path, 'Path copied')}><Icon name="copy" size={13} />Copy path</button>
      <button class="btn sm ghost" onclick={() => ui.seedComposer(ui.request, `\`${path}\` `)}><Icon name="chat" size={13} />Ask the Lead about it</button>
    </div>
  </header>
  <section>
    <h3 class="eyebrow">Who touched it</h3>
    {#each touches as t (t.agent.id)}
      <button class="who" onclick={() => ui.selectAgent(t.agent.id)} onpointerenter={() => ui.setHover({ agentId: t.agent.id, path })} onpointerleave={() => ui.setHover(null)}>
        <Portrait agent={t.agent} size={24} />
        <span class="w"><b>{t.agent.name}</b> {t.f!.created ? 'created it' : t.f!.deleted ? 'deleted it' : t.f!.modified ? 'changed it' : t.f!.read ? 'read it' : 'was given it'} · {ago(t.f!.lastTs, ui.now)}
          {#each t.f!.reasons.slice(-2) as r, i (i)}<span class="r"><Provenance kind={r.prov} compact />{r.text}</span>{/each}
        </span>
      </button>
    {:else}
      <p class="small">No agent has touched it in this session.</p>
    {/each}
  </section>
  {#if tasks.length}
    <section>
      <h3 class="eyebrow">Changed in</h3>
      {#each tasks as t (t.id)}<button class="task" onclick={() => ui.openDiff({ kind: 'task', id: t.id })}><code>{t.key}</code>{t.title}<Icon name="arrow-right" size={12} /></button>{/each}
    </section>
  {/if}
  {#if kgs.length}
    <section>
      <h3 class="eyebrow">Known facts</h3>
      <ul class="kg">{#each kgs as k (k.id)}<li><span>{label(k.subject)}</span><em>{k.predicate.replace(/_/g, ' ')}</em><span>{label(k.object)}</span><Provenance kind={k.source === 'observed' ? 'observed' : 'model'} compact /></li>{/each}</ul>
    </section>
  {/if}
</div>

<style>
  .fs { display: flex; flex-direction: column; overflow-y: auto; height: 100%; }
  header { padding: 26px 22px 14px; display: flex; flex-direction: column; gap: 4px; }
  h2 { font-size: var(--d-md); line-height: 1.1; padding-right: 30px; word-break: break-all; }
  .dir { color: var(--ink-3); font-size: 11.5px; }
  .acts { display: flex; gap: 6px; margin-top: 8px; flex-wrap: wrap; }
  section { padding: 14px 22px; box-shadow: inset 0 1px 0 var(--line); display: flex; flex-direction: column; gap: 8px; }
  .who { display: flex; gap: 10px; align-items: flex-start; text-align: left; padding: 6px 8px; margin: 0 -8px; border-radius: var(--r-md); }
  .who:hover { background: var(--surface-1); }
  .w { display: flex; flex-direction: column; gap: 2px; font-size: var(--t-sm); color: var(--ink-2); }
  .w b { color: var(--ink-1); }
  .r { display: flex; align-items: center; gap: 5px; font-size: var(--t-xs); color: var(--ink-3); }
  .small { font-size: var(--t-sm); color: var(--ink-3); }
  .task { display: flex; align-items: center; gap: 8px; font-size: var(--t-md); text-align: left; }
  .task code { font-size: 11px; color: var(--ink-3); }
  .task:hover { color: var(--ink-1); text-decoration: underline; }
  .kg { display: flex; flex-direction: column; gap: 6px; font-size: var(--t-sm); }
  .kg li { display: flex; align-items: center; gap: 6px; color: var(--ink-1); }
  .kg em { color: var(--ink-3); font-style: normal; }
</style>
