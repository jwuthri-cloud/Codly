<!--
  One artifact, full size. Images and SVGs as images; HTML in an iframe with `sandbox=""` (no
  scripts, no same-origin) unless you turn on "Run scripts" (adds allow-scripts only); Markdown
  rendered and sanitized; PDF embedded; CSV as a table. Who made it, in which task, and where it
  lives. Blob URLs are revoked when the viewer closes.
-->
<script lang="ts">
  import type { Artifact } from '../../lib/gen/Artifact';
  import { copyText, revealInFinder } from '../../lib/state/actions';
  import { acquireArtifactUrl, artifactText, KIND_LABEL, parseCsv, prettyBytes, releaseArtifactUrl } from '../../lib/state/artifacts';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { ago, clock } from '../../lib/util/format';
  import { idVar, PROVIDER_LABEL } from '../../lib/util/labels';
  import Icon from '../../components/Icon.svelte';
  import Markdown from '../../components/Markdown.svelte';
  import Portrait from '../../components/Portrait.svelte';
  import Provenance from '../../components/Provenance.svelte';

  interface Props {
    artifact: Artifact;
    list: Artifact[];
  }
  let { artifact, list }: Props = $props();

  const agent = $derived(artifact.agent_id ? (store.agentMap.get(artifact.agent_id) ?? null) : null);
  const task = $derived(artifact.task_id ? (store.taskMap.get(artifact.task_id) ?? null) : null);
  let url = $state<string | null>(null);
  let text = $state<string | null>(null);
  let failed = $state<string | null>(null);
  let scripts = $state(false);

  $effect(() => {
    const a = artifact;
    let alive = true;
    let held = false;
    url = null;
    text = null;
    failed = null;
    scripts = false;
    if (a.kind === 'markdown' || a.kind === 'csv') artifactText(a).then((t) => alive && (text = t)).catch((e) => alive && (failed = String(e?.message ?? e)));
    else
      acquireArtifactUrl(a)
        .then((u) => {
          if (!alive) return releaseArtifactUrl(a.id);
          held = true;
          url = u;
        })
        .catch((e) => alive && (failed = String(e?.message ?? e)));
    return () => {
      alive = false;
      if (held) releaseArtifactUrl(a.id);
    };
  });
  const rows = $derived(artifact.kind === 'csv' && text ? parseCsv(text) : []);
  const idx = $derived(list.findIndex((x) => x.id === artifact.id));
  function go(d: number) {
    if (list.length < 2) return;
    ui.artifactOpen = list[(idx + d + list.length) % list.length].id;
  }
  function key(e: KeyboardEvent) {
    if ((e.target as HTMLElement).closest('input, textarea')) return;
    if (e.key === 'ArrowRight') go(1);
    else if (e.key === 'ArrowLeft') go(-1);
  }
</script>

<svelte:window onkeydown={key} />

<div class="viewer" role="dialog" aria-label={artifact.title} style:--hue={idVar(agent?.color ?? 0)}>
  <div class="stage k-{artifact.kind}">
    {#if failed}
      <p class="fail">Couldn’t open it: {failed}</p>
    {:else if (artifact.kind === 'image' || artifact.kind === 'svg') && url}
      <img src={url} alt={artifact.caption ?? artifact.title} />
    {:else if artifact.kind === 'html' && url}
      {#key scripts}
        <iframe src={url} title={artifact.title} sandbox={scripts ? 'allow-scripts' : ''} referrerpolicy="no-referrer"></iframe>
      {/key}
    {:else if artifact.kind === 'pdf' && url}
      <embed src={url} type="application/pdf" title={artifact.title} />
    {:else if artifact.kind === 'markdown' && text != null}
      <div class="doc"><Markdown {text} /></div>
    {:else if artifact.kind === 'csv' && rows.length}
      <div class="csv selectable">
        <table>
          <thead><tr>{#each rows[0] as c, i (i)}<th>{c}</th>{/each}</tr></thead>
          <tbody>{#each rows.slice(1) as r, i (i)}<tr>{#each r as c, j (j)}<td>{c}</td>{/each}</tr>{/each}</tbody>
        </table>
      </div>
    {:else}
      <p class="fail">Opening…</p>
    {/if}
    {#if list.length > 1}
      <button class="nav prev icon-btn" aria-label="Previous" onclick={() => go(-1)}><Icon name="chevron-left" /></button>
      <button class="nav next icon-btn" aria-label="Next" onclick={() => go(1)}><Icon name="chevron-right" /></button>
    {/if}
  </div>
  <aside class="meta">
    <button class="icon-btn close" aria-label="Close" data-tip-kbd="Esc" data-tip="Back to the gallery" onclick={() => (ui.artifactOpen = null)}><Icon name="x" /></button>
    <p class="eyebrow">{KIND_LABEL[artifact.kind]} · {prettyBytes(artifact.bytes)}</p>
    <h2 class="serif">{artifact.title}</h2>
    {#if artifact.caption}<p class="cap selectable"><Provenance kind="model" compact /><span>{artifact.caption}</span></p>{/if}
    <dl>
      {#if agent}
        <dt>Made by</dt>
        <dd><button class="who" onclick={() => ui.selectAgent(agent.id)}><Portrait {agent} size={20} /><b>{agent.name}</b><span>{PROVIDER_LABEL[agent.provider]}</span></button></dd>
      {/if}
      {#if task}<dt>Task</dt><dd><button class="link" onclick={() => ui.selectTask(task.id)}><code>{task.key}</code> {task.title}</button></dd>{/if}
      <dt>When</dt><dd class="num">{clock(artifact.created_at)} · {ago(artifact.created_at, ui.now)}</dd>
      <dt>Path</dt><dd><code class="path selectable">{artifact.path}</code></dd>
    </dl>
    <div class="acts">
      <button class="btn sm" onclick={() => revealInFinder(artifact.abs_path)}><Icon name="reveal" size={13} />Reveal in Finder</button>
      <button class="btn sm ghost" onclick={() => copyText(artifact.abs_path, 'Path copied')}><Icon name="copy" size={13} />Copy path</button>
    </div>
    {#if artifact.kind === 'html'}
      <label class="scripts">
        <input type="checkbox" class="switch" bind:checked={scripts} />
        <span><b>Run scripts</b><span class="d">Off by default. On, the page may run its JavaScript inside a sandbox with no access to Codly or your files. Turn on only for pages you trust.</span></span>
      </label>
    {/if}
  </aside>
</div>

<style>
  .viewer { position: absolute; inset: 0; z-index: 3; display: flex; background: var(--glass-solid); animation: fade-in var(--d-3) var(--ease-out); }
  .stage { position: relative; flex: 1; min-width: 0; display: grid; place-items: center; padding: 28px; background: radial-gradient(circle at 50% 40%, var(--surface-2), transparent 70%); }
  .stage.k-image { background-color: var(--night-2); }
  img { max-width: 100%; max-height: 100%; object-fit: contain; border-radius: var(--r-md); box-shadow: 0 20px 60px -24px rgba(0, 0, 0, 0.6); animation: rise-in var(--d-4) var(--spring); }
  .k-svg img { background: #fff; }
  iframe, embed { width: 100%; height: 100%; border: 0; border-radius: var(--r-md); background: #fff; box-shadow: 0 20px 60px -24px rgba(0, 0, 0, 0.6); }
  .doc { width: min(720px, 100%); height: 100%; overflow-y: auto; padding: 36px 44px; border-radius: var(--r-md); background: var(--surface-1); }
  .csv { width: 100%; height: 100%; overflow: auto; }
  table { border-collapse: collapse; font-size: var(--t-md); margin: 0 auto; }
  th { position: sticky; top: 0; background: var(--glass-solid); text-align: left; font-weight: var(--w-semi); color: var(--ink-2); padding: 8px 14px; box-shadow: inset 0 -1px 0 var(--line-2); }
  td { padding: 7px 14px; box-shadow: inset 0 -1px 0 var(--line); font-variant-numeric: tabular-nums; font-family: var(--font-mono); font-size: 12px; }
  .fail { color: var(--ink-3); }
  .nav { position: absolute; top: 50%; transform: translateY(-50%); background: var(--pop); box-shadow: var(--pop-shadow); }
  .prev { left: 12px; }
  .next { right: 12px; }
  .meta { position: relative; width: 320px; flex-shrink: 0; padding: 26px 24px; display: flex; flex-direction: column; gap: 12px; overflow-y: auto; box-shadow: inset 1px 0 0 var(--line); }
  .close { position: absolute; top: 14px; right: 14px; }
  h2 { font-size: var(--d-md); line-height: 1.1; padding-right: 26px; }
  .cap { display: flex; gap: 8px; align-items: flex-start; font-size: var(--t-md); line-height: var(--lh-md); color: var(--ink-2); }
  dl { display: grid; grid-template-columns: auto 1fr; gap: 8px 14px; margin: 4px 0 0; font-size: var(--t-sm); }
  dt { color: var(--ink-3); }
  dd { margin: 0; min-width: 0; color: var(--ink-1); }
  .who { display: inline-flex; align-items: center; gap: 6px; }
  .who span { color: var(--ink-3); }
  .link { text-align: left; }
  .link code { font-size: 11px; color: var(--ink-3); }
  .path { font-size: 11px; word-break: break-all; color: var(--ink-2); }
  .acts { display: flex; gap: 6px; flex-wrap: wrap; }
  .scripts { display: flex; gap: 12px; align-items: flex-start; padding: 12px; border-radius: var(--r-md); background: var(--surface-1); }
  .scripts > span { display: flex; flex-direction: column; gap: 3px; font-size: var(--t-sm); }
  .scripts .d { color: var(--ink-3); line-height: var(--lh-sm); }
</style>
