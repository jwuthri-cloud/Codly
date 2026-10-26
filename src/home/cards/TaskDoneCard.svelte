<!--
  A task landed: who did it, what it says it did (its own words), the diff it produced, the
  checks Codly ran (verify, review, merge), and what it made (artifacts).
-->
<script lang="ts">
  import { diffstats, statLine } from '../../lib/state/diffstats.svelte';
  import { store } from '../../lib/state/store.svelte';
  import { accounts } from '../../lib/state/accounts.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { clock } from '../../lib/util/format';
  import { idVar } from '../../lib/util/labels';
  import ArtifactThumb from '../../components/ArtifactThumb.svelte';
  import Icon from '../../components/Icon.svelte';
  import InlineText from '../../components/InlineText.svelte';
  import Portrait from '../../components/Portrait.svelte';
  import Provenance from '../../components/Provenance.svelte';

  interface Props {
    taskId: string;
  }
  let { taskId }: Props = $props();
  const task = $derived(store.taskMap.get(taskId) ?? null);
  const agent = $derived(task?.assignee ? (store.agentMap.get(task.assignee) ?? null) : null);
  const marks = $derived.by(() => {
    void store.rev;
    return store.taskMarks[taskId] ?? [];
  });
  const verify = $derived([...marks].reverse().find((m) => m.kind === 'verified') ?? null);
  const review = $derived([...marks].reverse().find((m) => m.kind === 'reviewed') ?? null);
  const merged = $derived([...marks].reverse().find((m) => m.kind === 'merged') ?? null);
  const stat = $derived(diffstats.task[taskId] ?? null);
  const arts = $derived(store.artifacts.filter((a) => a.task_id === taskId));
  const tests = $derived.by(() => {
    if (!agent) return null;
    void store.rev;
    const segs = store.live[agent.id]?.segments.filter((s) => s.taskId === taskId && s.activity === 'test' && s.end != null) ?? [];
    const last = segs[segs.length - 1];
    return last && last.testsPassed != null ? last : null;
  });
</script>

{#if task}
  <article class="card" style:--hue={idVar(agent?.color ?? 0)}>
    <header>
      {#if agent}<button class="who" onclick={() => ui.selectAgent(agent.id)} aria-label="Open {agent.name}"><Portrait {agent} size={28} /></button>{/if}
      <div class="h">
        <p class="line"><b>{agent?.name ?? 'An agent'}</b> finished <button class="key" onclick={() => ui.selectTask(task.id)}>{task.key}</button></p>
        <h3 class="serif">{task.title}</h3>
      </div>
      <time class="num">{clock(merged?.ts ?? task.updated_at)}</time>
    </header>

    {#if task.summary}<p class="summary selectable"><Provenance kind="model" compact /><span><InlineText text={task.summary} /></span></p>{/if}

    <ul class="facts">
      {#if stat}<li data-tip="From git, in the task’s worktree"><Icon name="diff" size={13} /><span class="num"><span class="add">+{stat.added}</span> <span class="del">−{stat.removed}</span> in {stat.files} {stat.files === 1 ? 'file' : 'files'}</span></li>{/if}
      {#if verify}<li class:bad={!verify.ok}><Icon name={verify.ok ? 'check' : 'x'} size={13} stroke={2} /><code>{verify.text}</code>{#if tests}<span class="num">· {tests.testsFailed ? `${tests.testsFailed} failed` : `${tests.testsPassed} passed`}</span>{/if}</li>{/if}
      {#if review}<li class:bad={!review.ok}><Icon name="eye" size={13} />{review.text}</li>{/if}
      {#if merged}<li><Icon name="merge" size={13} />merged into <code>{merged.text}</code></li>{/if}
    </ul>

    {#if arts.length}
      <div class="arts">
        {#each arts as a (a.id)}
          <button class="art" onclick={() => ui.openArtifact(a.id)} data-tip={a.title}>
            <ArtifactThumb artifact={a} />
          </button>
        {/each}
      </div>
    {/if}

    <footer>
      <button class="btn sm" onclick={() => ui.openDiff({ kind: 'task', id: task.id })}><Icon name="diff" size={13} />View changes</button>
      {#if agent}<span class="by">{accounts.who(agent)}</span>{/if}
    </footer>
  </article>
{/if}

<style>
  .card { position: relative; display: flex; flex-direction: column; gap: 12px; padding: 16px 20px 14px 22px; border-radius: var(--r-lg); background: var(--surface-1); box-shadow: inset 3px 0 0 var(--hue); }
  header { display: flex; align-items: flex-start; gap: 12px; }
  .who { display: grid; border-radius: 50%; margin-top: 2px; }
  .h { flex: 1; min-width: 0; }
  .line { font-size: var(--t-sm); color: var(--ink-3); }
  .line b { color: var(--ink-1); font-weight: var(--w-semi); }
  .key { font-family: var(--font-mono); font-size: 11.5px; color: var(--ink-2); padding: 0 4px; border-radius: 4px; background: var(--surface-2); }
  .key:hover { color: var(--ink-1); }
  h3 { font-size: var(--d-sm); line-height: 1.15; color: var(--ink-1); margin-top: 2px; }
  time { font-size: var(--t-xs); color: var(--ink-3); }
  .summary { display: flex; gap: 8px; align-items: flex-start; font-size: var(--t-md); line-height: var(--lh-md); color: var(--ink-2); }
  .summary :global(.pv) { margin-top: 1px; }
  .facts { display: flex; flex-wrap: wrap; gap: 6px 16px; font-size: var(--t-sm); color: var(--ink-2); }
  .facts li { display: inline-flex; align-items: center; gap: 6px; }
  .facts li :global(svg) { color: var(--ok); }
  .facts li:first-child :global(svg) { color: var(--ink-3); }
  .facts li.bad :global(svg) { color: var(--bad); }
  .facts code { font-size: 11px; color: var(--ink-2); }
  .add { color: var(--ok); }
  .del { color: var(--bad); }
  .arts { display: flex; gap: 8px; flex-wrap: wrap; }
  .art { width: 116px; height: 74px; border-radius: var(--r-md); overflow: hidden; box-shadow: inset 0 0 0 1px var(--line); transition: transform var(--d-2) var(--spring); }
  .art:hover { transform: translateY(-2px); }
  footer { display: flex; align-items: center; gap: 10px; }
  .by { margin-left: auto; font-size: var(--t-xs); color: var(--ink-3); }
</style>
