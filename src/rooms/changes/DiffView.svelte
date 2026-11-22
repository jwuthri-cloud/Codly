<!-- Unified diff for one file: hunk headers, old/new line numbers, +/− gutters, and the hue of
     the agent that wrote it along the edge. Dense on purpose: this is where scanning matters.
     In the Changes room it is also where you review: a comment on a line or on the file, a
     "viewed" tick, and the file in the editor. A comment whose line the worker changed since
     stays under the file's name, where you can read and remove it. A large diff stays fast: a
     file builds its lines only when it comes near the visible part, and at most LINE_STEP of
     them at a time. -->
<script lang="ts">
  import type { DiffFile } from '../../lib/util/diff';
  import { ui } from '../../lib/state/ui.svelte';
  import { fileAnchor, FILE_PLACE, firstLines, lineAnchor, lineCount, linePlace, linesToDraw, LINE_STEP, placeComments, placeId, type Anchor, type ReviewComment } from '../../lib/util/review';
  import Icon from '../../components/Icon.svelte';

  interface Props {
    file: DiffFile;
    hue: string;
    who?: string;
    collapsed?: boolean;
    ontoggle?: () => void;
    /** Why the file opened collapsed ("Lockfile", "Large"…), beside its counts. */
    kind?: string;
    /** The scroller the file sits in (default: the window): its lines are built once it is near. */
    root?: HTMLElement | null;
    /** Your comments on this file that are not sent yet. */
    comments?: ReviewComment[];
    /** Save the comment at a place (empty text removes it). Given: the file takes comments. */
    oncomment?: (at: Anchor, text: string) => void;
    viewed?: boolean;
    /** Given: the file has a "Viewed" tick. */
    onviewed?: (on: boolean) => void;
    /** Given: the file has "Open in editor", and this says where it opens, or why it cannot. */
    openTip?: string;
    /** Open the file in the editor. Absent with `openTip`: the button is there, switched off. */
    onopen?: () => void;
  }
  let { file, hue, who, collapsed = false, ontoggle, kind, root = null, comments = [], oncomment, viewed = false, onviewed, openTip, onopen }: Props = $props();
  const STATUS: Record<string, string> = { added: 'New', deleted: 'Deleted', modified: '', renamed: 'Renamed', binary: 'Binary' };
  const dir = $derived(file.path.includes('/') ? file.path.slice(0, file.path.lastIndexOf('/') + 1) : '');
  const base = $derived(file.path.slice(dir.length));

  let section = $state<HTMLElement | null>(null);
  /** The file came within one screen of the visible part once: its lines are built, and stay. */
  let near = $state(false);
  $effect(() => {
    if (near || !section) return;
    const io = new IntersectionObserver((seen) => seen.some((e) => e.isIntersecting) && (near = true), { root, rootMargin: '100% 0px' });
    io.observe(section);
    return () => io.disconnect();
  });
  const sorted = $derived(placeComments(file, comments));
  /** The comments that sit on the file or on a line of its diff, by place. */
  const notes = $derived(sorted.placed);
  /** The comments whose line left the diff or reads differently by now: listed under the name. */
  const lost = $derived(sorted.lost);
  let limit = $state(LINE_STEP);
  const total = $derived(lineCount(file));
  const drawn = $derived(linesToDraw(file, limit, notes));
  const shown = $derived(firstLines(file, drawn));
  const rest = $derived(Math.max(0, total - drawn));

  /** The place whose comment box is open, and what is typed in it. */
  let editing = $state.raw<Anchor | null>(null);
  let draft = $state('');
  const editingId = $derived(editing ? placeId(editing) : null);
  const onFile = $derived(fileAnchor(file.path));
  function edit(at: Anchor) {
    editing = at;
    draft = notes.get(placeId(at))?.text ?? '';
  }
  function save() {
    if (editing) oncomment?.(editing, draft);
    editing = null;
  }
  function boxKey(e: KeyboardEvent) {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      save();
    } else if (e.key === 'Escape') {
      e.stopPropagation();
      editing = null;
    }
  }
  const focus = (node: HTMLTextAreaElement) => node.focus();
</script>

{#snippet note(at: Anchor)}
  {@const c = notes.get(placeId(at))}
  <div class="note" class:of-file={at.line == null}>
    {#if editingId === placeId(at)}
      <textarea rows={3} bind:value={draft} placeholder={at.line == null ? 'What should change in this file?' : 'What should change here?'} aria-label={at.line == null ? `Comment on ${file.path}` : `Comment on line ${at.line}`} onkeydown={boxKey} use:focus></textarea>
      <div class="note-acts">
        <button class="btn sm ghost" onclick={() => (editing = null)}>Cancel</button>
        <button class="btn sm" disabled={!draft.trim() && !c} onclick={save} data-tip="Kept on the diff until you send your comments back" data-tip-kbd="⌘↵">{c ? 'Save' : 'Add comment'}</button>
      </div>
    {:else if c}
      <p class="note-t">{c.text}</p>
      <div class="note-acts">
        <button class="btn sm ghost" onclick={() => edit(at)}>Edit</button>
        <button class="btn sm ghost" onclick={() => oncomment?.(at, '')}>Remove</button>
      </div>
    {/if}
  </div>
{/snippet}

<section class="file" bind:this={section} id="diff-{file.path}" role="group" aria-label={file.path} style:--hue={hue} onpointerenter={() => ui.setHover({ path: file.path })} onpointerleave={() => ui.setHover(null)}>
  <header>
    <button class="fh" onclick={ontoggle} aria-expanded={!collapsed}>
      <span class="chev" class:open={!collapsed}><Icon name="chevron-right" size={12} stroke={2} /></span>
      <span class="path truncate"><span class="dir">{dir}</span>{base}</span>
    </button>
    {#if STATUS[file.status]}<span class="status s-{file.status}">{STATUS[file.status]}</span>{/if}
    {#if kind}<span class="status">{kind}</span>{/if}
    {#if who}<span class="who">{who}</span>{/if}
    {#if comments.length}<span class="cc">{comments.length} {comments.length === 1 ? 'comment' : 'comments'}</span>{/if}
    <span class="stat num"><span class="add">+{file.added}</span><span class="del">−{file.removed}</span></span>
    {#if oncomment}<button class="icon-btn sm" aria-label="Comment on the file" data-tip="Comment on the whole file" onclick={() => edit(onFile)}><Icon name="chat" size={13} /></button>{/if}
    {#if openTip}<button class="icon-btn sm" aria-label="Open in editor" aria-disabled={!onopen} data-tip="Open in editor. {openTip}" onclick={() => onopen?.()}><Icon name="code" size={13} /></button>{/if}
    {#if onviewed}<label class="viewed" data-tip="Tick the files you have read. A file that changes later is unticked."><input type="checkbox" checked={viewed} onchange={(e) => onviewed?.(e.currentTarget.checked)} />Viewed</label>{/if}
  </header>
  {#if editingId === FILE_PLACE || notes.has(FILE_PLACE)}{@render note(onFile)}{/if}
  {#each lost as c (placeId(c) + c.quote)}
    <div class="note of-file">
      <p class="note-w">Line {c.line} changed or left the diff after you wrote this. Send back still sends your comment, with the line as it was:</p>
      <p class="note-q">{c.quote}</p>
      <p class="note-t">{c.text}</p>
      <div class="note-acts"><button class="btn sm ghost" onclick={() => oncomment?.(c, '')}>Remove</button></div>
    </div>
  {/each}
  {#if !collapsed}
    {#if file.status === 'binary'}
      <p class="binary">Binary file, not shown.</p>
    {:else if !near}
      <div class="ph" style:height="calc({Math.min(total, drawn)} * var(--lh-mono) + {shown.length} * (var(--lh-mono) + 8px) + 4px)"></div>
    {:else}
      <div class="code selectable" role="table" aria-label="Diff of {file.path}">
        {#each shown as s, hi (hi)}
          <div class="hunk" role="row"><span>{s.hunk.header}</span></div>
          {#each s.lines as l, li (li)}
            {@const place = oncomment ? linePlace(l) : null}
            <div class="ln k-{l.kind}" role="row">
              <span class="no">{l.oldNo ?? ''}</span>
              <span class="no">{l.newNo ?? ''}</span>
              <span class="sign">{l.kind === 'add' ? '+' : l.kind === 'del' ? '−' : ''}</span>
              <span class="txt">{l.text || ' '}</span>
              {#if place}<button class="cm" tabindex="-1" aria-label="Comment on line {l.newNo ?? l.oldNo}" data-tip="Comment on this line" onclick={() => edit(lineAnchor(file.path, l))}>+</button>{/if}
            </div>
            {#if place && (place === editingId || notes.has(place))}{@render note(lineAnchor(file.path, l))}{/if}
          {/each}
        {/each}
        {#if rest}
          <div class="more">
            <button class="btn sm" onclick={() => (limit = drawn + LINE_STEP)}>Show {Math.min(rest, LINE_STEP)} more lines</button>
            <span>{rest} of {total} lines not shown yet</span>
          </div>
        {/if}
      </div>
    {/if}
  {/if}
</section>

<style>
  .file { flex-shrink: 0; border-radius: var(--r-md); background: var(--surface-1); overflow: clip; box-shadow: inset 3px 0 0 var(--hue); }
  header { position: sticky; top: 0; z-index: 1; display: flex; align-items: center; gap: 10px; height: 38px; padding: 0 14px 0 8px; background: var(--glass-solid); box-shadow: inset 3px 0 0 var(--hue), inset 0 -1px 0 var(--line); }
  .fh { display: flex; align-items: center; gap: 6px; flex: 1; min-width: 0; height: 30px; padding: 0 6px; border-radius: var(--r-sm); text-align: left; }
  .chev { color: var(--ink-3); transition: transform var(--d-2) var(--ease-out); }
  .chev.open { transform: rotate(90deg); }
  .path { font-family: var(--font-mono); font-size: 12px; color: var(--ink-1); }
  .dir { color: var(--ink-3); }
  .status { font-size: var(--t-xs); color: var(--ink-3); }
  .s-added { color: var(--ok); }
  .s-deleted { color: var(--bad); }
  .who { font-size: var(--t-xs); color: var(--hue); }
  .cc { font-size: var(--t-xs); color: var(--amber-ink); white-space: nowrap; }
  .stat { display: inline-flex; gap: 6px; font-size: var(--t-sm); font-family: var(--font-mono); }
  .add { color: var(--ok); }
  .del { color: var(--bad); }
  .icon-btn[aria-disabled='true'] { opacity: 0.4; cursor: default; }
  .viewed { display: inline-flex; align-items: center; gap: 5px; font-size: var(--t-xs); color: var(--ink-2); white-space: nowrap; cursor: pointer; }
  .ph { flex-shrink: 0; }
  .code { position: relative; font-family: var(--font-mono); font-size: var(--mono); line-height: var(--lh-mono); overflow-x: auto; padding-bottom: 4px; font-variant-ligatures: none; font-feature-settings: 'calt' 0, 'liga' 0; }
  .hunk { padding: 4px 14px; background: var(--surface-1); color: var(--ink-3); white-space: pre; font-size: 11.5px; }
  .ln { position: relative; display: grid; grid-template-columns: 46px 46px 20px 1fr; min-width: max-content; }
  .no { padding: 0 8px; text-align: right; color: var(--ink-4); user-select: none; -webkit-user-select: none; font-variant-numeric: tabular-nums; }
  .sign { text-align: center; color: var(--ink-3); user-select: none; -webkit-user-select: none; }
  .txt { white-space: pre; padding-right: 20px; color: var(--ink-1); }
  .k-ctx .txt { color: var(--ink-2); }
  .k-add { background: var(--diff-add); }
  .k-add .no { background: var(--diff-add-strong); color: var(--ok); }
  .k-add .sign { color: var(--ok); }
  .k-del { background: var(--diff-del); }
  .k-del .no { background: var(--diff-del-strong); color: var(--bad); }
  .k-del .sign { color: var(--bad); }
  .k-meta .txt { color: var(--ink-3); font-style: italic; }
  /* The "+" that opens a line's comment box: over the start of the gutter, shown on the line you point at. */
  .cm { position: absolute; left: 2px; top: 1px; width: 17px; height: 17px; border-radius: 5px; background: var(--amber); color: #2a1a04; font-family: var(--font-ui); font-size: 13px; font-weight: var(--w-semi); line-height: 17px; text-align: center; opacity: 0; user-select: none; -webkit-user-select: none; }
  .ln:hover .cm, .cm:focus-visible { opacity: 1; }
  .note { position: sticky; left: 0; box-sizing: border-box; width: 100%; max-width: 720px; margin: 4px 0 6px; padding: 8px 12px; display: flex; flex-direction: column; gap: 6px; border-radius: var(--r-sm); background: var(--amber-soft); box-shadow: inset 2px 0 0 var(--amber); font-family: var(--font-ui); font-size: var(--t-sm); line-height: var(--lh-sm); }
  .note.of-file { margin: 8px 14px; width: auto; }
  .note textarea { width: 100%; font-size: var(--t-sm); line-height: var(--lh-sm); }
  .note-t { white-space: pre-wrap; overflow-wrap: anywhere; color: var(--ink-1); }
  .note-w { color: var(--ink-2); }
  .note-q { font-family: var(--font-mono); font-size: 11.5px; white-space: pre; overflow-x: auto; color: var(--ink-2); }
  .note-acts { display: flex; justify-content: flex-end; gap: 6px; }
  .more { position: sticky; left: 0; display: flex; align-items: center; gap: 10px; padding: 8px 14px; font-family: var(--font-ui); font-size: var(--t-sm); color: var(--ink-3); }
  .binary { padding: 14px; color: var(--ink-3); font-size: var(--t-sm); }
</style>
