<!--
  A small, safe preview of an artifact: images and SVGs as <img> (no scripts run in an image),
  HTML in a sandboxed iframe with no permissions, Markdown rendered and sanitized, CSV as a
  table. Blob URLs are released when the thumbnail unmounts.
-->
<script lang="ts">
  import type { Artifact } from '../lib/gen/Artifact';
  import { acquireArtifactUrl, artifactText, KIND_ICON, parseCsv, releaseArtifactUrl } from '../lib/state/artifacts';
  import { renderMarkdown } from '../lib/util/markdown';
  import Icon from './Icon.svelte';

  interface Props {
    artifact: Artifact;
  }
  let { artifact }: Props = $props();

  let url = $state<string | null>(null);
  let text = $state<string | null>(null);
  let failed = $state(false);

  $effect(() => {
    const a = artifact;
    let alive = true;
    url = null;
    text = null;
    failed = false;
    if (a.kind === 'image' || a.kind === 'svg' || a.kind === 'html') {
      acquireArtifactUrl(a).then((u) => {
        if (alive) url = u;
        else releaseArtifactUrl(a.id);
      }).catch(() => alive && (failed = true));
      return () => {
        if (alive && url) releaseArtifactUrl(a.id);
        alive = false;
      };
    }
    if (a.kind === 'markdown' || a.kind === 'csv') artifactText(a).then((t) => alive && (text = t)).catch(() => alive && (failed = true));
    return () => {
      alive = false;
    };
  });
  const rows = $derived(artifact.kind === 'csv' && text ? parseCsv(text, 6) : []);
</script>

<div class="thumb k-{artifact.kind}">
  {#if failed}
    <span class="ph"><Icon name="alert" size={18} /></span>
  {:else if (artifact.kind === 'image' || artifact.kind === 'svg') && url}
    <img src={url} alt={artifact.title} draggable="false" />
  {:else if artifact.kind === 'html' && url}
    <iframe src={url} sandbox="" title={artifact.title} tabindex="-1" loading="lazy" referrerpolicy="no-referrer"></iframe>
  {:else if artifact.kind === 'markdown' && text}
    <div class="md-mini">{@html renderMarkdown(text.slice(0, 900))}</div>
  {:else if artifact.kind === 'csv' && rows.length}
    <table class="csv-mini">
      <tbody>{#each rows as r, i (i)}<tr>{#each r.slice(0, 4) as c, j (j)}{#if i === 0}<th>{c}</th>{:else}<td>{c}</td>{/if}{/each}</tr>{/each}</tbody>
    </table>
  {:else}
    <span class="ph"><Icon name={KIND_ICON[artifact.kind]} size={20} /></span>
  {/if}
</div>

<style>
  .thumb { position: relative; width: 100%; height: 100%; overflow: hidden; background: var(--surface-2); display: grid; place-items: center; }
  .k-svg, .k-html, .k-markdown, .k-csv { background: #fbfaf7; color: #1d2230; }
  img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .k-svg img { object-fit: contain; padding: 6%; }
  iframe { position: absolute; left: 0; top: 0; width: 400%; height: 400%; border: 0; transform: scale(0.25); transform-origin: 0 0; pointer-events: none; background: #fff; }
  .md-mini { position: absolute; inset: 0; padding: 10% 9%; transform-origin: 0 0; font-size: 7px; line-height: 1.45; color: #2a2f3c; overflow: hidden; pointer-events: none; }
  .md-mini :global(h1) { font-family: var(--font-display); font-size: 15px; font-weight: 400; margin: 0 0 4px; }
  .md-mini :global(h2) { font-size: 9px; margin: 6px 0 2px; }
  .md-mini :global(p), .md-mini :global(table), .md-mini :global(pre) { margin: 0 0 4px; }
  .md-mini :global(code) { font-size: 0.9em; }
  .md-mini :global(td), .md-mini :global(th) { padding: 1px 3px; border-bottom: 0.5px solid #e2e2e2; }
  .csv-mini { position: absolute; inset: 10% 8% auto; border-collapse: collapse; font-size: 7.5px; line-height: 1.6; color: #2a2f3c; pointer-events: none; }
  .csv-mini th { text-align: left; font-weight: 600; border-bottom: 0.5px solid #c9ccd4; padding: 1px 4px 1px 0; }
  .csv-mini td { padding: 1px 4px 1px 0; border-bottom: 0.5px solid #eceef2; }
  .ph { color: var(--ink-3); }
</style>
