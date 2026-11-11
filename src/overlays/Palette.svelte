<!--
  ⌘K, everywhere. Scopes: > commands · @ agents · # tasks · / files · + new task · ? help.
  ↵ runs the primary action, ⌘↵ the secondary. Esc pops the scope, then closes. Recents first;
  "Ask the Lead…" is always the last row, so a query never dead-ends.
  ponytail: no Raycast-style ⌘K action panel yet — ⌘↵ runs the row's one secondary action.
-->
<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api } from '../lib/api';
  import type { Citation } from '../lib/gen/Citation';
  import type { Provider } from '../lib/gen/Provider';
  import type { SearchResult } from '../lib/gen/SearchResult';
  import {
    cancelPlan, createTask, interruptAgent, mergePlan, openWorkspace, redetect, resolveApproval, runPlan, sendChat, updateSettings,
  } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import { narrate } from '../lib/state/narration';
  import { ROOMS, ui } from '../lib/state/ui.svelte';
  import { fuzzy, highlight } from '../lib/util/fuzzy';
  import { PROVIDER_LABEL, TASK_STATUS_LABEL, taskState } from '../lib/util/labels';
  import { modal } from '../lib/util/modal';
  import type { Tone } from '../lib/narrate';
  import { oneLine } from '../lib/util/sanitize';
  import Icon from '../components/Icon.svelte';
  import Kbd from '../components/Kbd.svelte';
  import Portrait from '../components/Portrait.svelte';
  import StateMark from '../components/StateMark.svelte';
  import { skillsUi } from '../lib/state/skills.svelte';

  type Scope = '>' | '@' | '#' | '/' | '+' | null;
  type Row = {
    id: string;
    section: string;
    title: string;
    sub?: string;
    icon?: string;
    agentId?: string;
    state?: Tone;
    kbd?: string;
    hint?: string;
    run: () => void;
    alt?: { label: string; run: () => void };
    keep?: boolean;
  };

  const SCOPES: Record<Exclude<Scope, null>, { label: string; placeholder: string; icon: string }> = {
    '>': { label: 'Commands', placeholder: 'Run a command…', icon: 'command' },
    '@': { label: 'Agents', placeholder: 'Find an agent…', icon: 'at' },
    '#': { label: 'Tasks', placeholder: 'Find a task…', icon: 'hash' },
    '/': { label: 'Files', placeholder: 'Search the codebase…', icon: 'slash' },
    '+': { label: 'New task', placeholder: 'What needs doing?', icon: 'plus' },
  };

  let scope = $state<Scope>(null);
  let query = $state('');
  let active = $state(0);
  let inputEl: HTMLInputElement | undefined = $state();
  let listEl: HTMLDivElement | undefined = $state();
  let search = $state<SearchResult | null>(null);
  let searching = $state(false);
  let searchError = $state<string | null>(null);

  // Seed (⌘N → "+", ui.openPalette('>') …)
  const seed = ui.paletteSeed;
  if (seed && '>@#/+'.includes(seed[0])) {
    scope = seed[0] as Scope;
    query = seed.slice(1);
  } else query = seed;

  const RECENT_KEY = 'codly.palette.recents';
  let recents: string[] = [];
  try {
    recents = JSON.parse(localStorage.getItem(RECENT_KEY) ?? '[]');
  } catch {
    recents = [];
  }
  function remember(id: string) {
    recents = [id, ...recents.filter((r) => r !== id)].slice(0, 8);
    try {
      localStorage.setItem(RECENT_KEY, JSON.stringify(recents));
    } catch {
      /* ignore */
    }
  }

  function close() {
    ui.paletteOpen = false;
    ui.paletteSeed = '';
  }

  // ───────────── sources ─────────────
  const commands = $derived.by((): Row[] => {
    const rows: Row[] = [];
    for (const r of ROOMS) rows.push({ id: `go:${r.id}`, section: 'Go to', title: `Go to ${r.label}`, sub: r.hint, icon: r.icon, kbd: `G ${r.key}`, run: () => ui.go(r.id) });
    rows.push(
      { id: 'go:artifacts', section: 'Go to', title: 'Go to Artifacts', sub: 'Everything the agents made', icon: 'image', run: () => ((ui.changesLens = 'artifacts'), ui.go('changes')) },
      { id: 'go:timeline', section: 'Go to', title: 'Go to Timeline', sub: 'Per-agent lanes', icon: 'timeline', run: () => ((ui.planLens = 'timeline'), ui.go('plan')) },
      { id: 'go:list', section: 'Go to', title: 'Go to the task list', sub: 'The plan’s tasks by status', icon: 'list', run: () => ((ui.planLens = 'list'), ui.go('plan')) },
      { id: 'go:search', section: 'Go to', title: 'Search the code', icon: 'search', run: () => ((ui.knowledgeLens = 'search'), ui.go('knowledge')) },
      { id: 'view:office', section: 'View', title: ui.mode === 'office' ? 'Back to work' : 'Full office', icon: 'office', kbd: '⌘1', run: () => ui.toggleMode('office') },
      { id: 'view:map', section: 'View', title: ui.mode === 'map' ? 'Back to work' : 'Neighbourhood (all your lots)', icon: 'map', kbd: '⌘3', run: () => ui.toggleMode('map') },
      { id: 'view:work', section: 'View', title: ui.mode === 'work' ? 'Bring the office back' : 'Full work (hide the office)', icon: 'workspace', kbd: '⌘2', run: () => ui.toggleMode('work') },
      { id: 'view:focus', section: 'View', title: ui.focus ? 'End focus' : 'Focus: dim the office', icon: 'focus', kbd: '⌘.', run: () => (ui.focus = !ui.focus) },
      { id: 'request:new', section: 'Work', title: 'New request', sub: 'Its own Lead plans it, in parallel with the others', icon: 'plus', kbd: '⌘N', run: () => ui.newRequest() },
      { id: 'task:new', section: 'Work', title: 'New task…', icon: 'plus', kbd: '⇧⌘N', keep: true, run: () => setScope('+') },
      { id: 'repo:open', section: 'Work', title: 'Open repository…', icon: 'folder', kbd: '⌘O', run: () => openWorkspace() },
      { id: 'settings', section: 'App', title: 'Settings', icon: 'settings', kbd: '⌘,', run: () => (ui.settingsOpen = true) },
      { id: 'skills', section: 'App', title: 'Skills', sub: 'Installed skills, and a catalog to install from', icon: 'bolt', run: () => skillsUi.show() },
      { id: 'onboarding', section: 'App', title: 'Replay the quest', sub: 'Install agents, add models, images, voice and video', icon: 'flag', run: () => (ui.onboardingOpen = true) },
      { id: 'shortcuts', section: 'App', title: 'Keyboard shortcuts', icon: 'keyboard', kbd: '?', run: () => (ui.shortcutsOpen = true) },
      { id: 'theme:system', section: 'App', title: 'Theme: follow the system', icon: 'monitor', run: () => updateSettings({ theme: 'system' }) },
      { id: 'theme:dark', section: 'App', title: 'Theme: dark', icon: 'moon', run: () => updateSettings({ theme: 'dark' }) },
      { id: 'theme:light', section: 'App', title: 'Theme: light', icon: 'sun', run: () => updateSettings({ theme: 'light' }) },
      { id: 'clis:detect', section: 'App', title: 'Check agent CLIs again', icon: 'refresh', run: () => redetect().then(() => toast({ kind: 'success', title: 'CLIs re-detected' })) },
    );
    for (const p of store.plans) {
      if (p.status === 'awaiting_approval') rows.push({ id: `plan:run:${p.id}`, section: 'Plan', title: `Run plan: ${p.summary}`, icon: 'play', run: () => runPlan(p.id) });
      if (p.status === 'running' || p.status === 'awaiting_approval') rows.push({ id: `plan:cancel:${p.id}`, section: 'Plan', title: `Cancel plan: ${p.summary}`, icon: 'stop', run: () => cancelPlan(p.id) });
      if (p.status === 'done' && !store.planMerges[p.id]) rows.push({ id: `plan:merge:${p.id}`, section: 'Plan', title: `Merge into ${store.workspace?.current_branch ?? 'main'}: ${p.summary}`, icon: 'merge', run: () => mergePlan(p.id) });
    }
    for (const ap of store.pendingApprovals) {
      const a = store.agentMap.get(ap.agent_id);
      const what = ap.subject.kind === 'command' ? oneLine(ap.subject.command, 50) : ap.subject.kind;
      if (ap.choices.includes('allow_once')) rows.push({ id: `ap:ok:${ap.id}`, section: 'Needs you', title: `Approve ${a?.name ?? 'agent'}: ${what}`, icon: 'check', run: () => resolveApproval(ap.id, 'allow_once') });
      if (ap.choices.includes('deny')) rows.push({ id: `ap:no:${ap.id}`, section: 'Needs you', title: `Deny ${a?.name ?? 'agent'}: ${what}`, icon: 'x', run: () => resolveApproval(ap.id, 'deny') });
    }
    for (const c of store.clis)
      if (c.path && !c.problem)
        rows.push({ id: `hire:${c.provider}`, section: 'Agents', title: `Start a ${PROVIDER_LABEL[c.provider as Provider]} agent (1:1)`, icon: 'user-plus', run: () => (ui.modelDialog = { provider: c.provider as Provider, agentId: null }) });
    for (const a of store.agents)
      if (narrate(a.id)?.tone === 'live') rows.push({ id: `int:${a.id}`, section: 'Agents', title: `Interrupt ${a.name}`, icon: 'pause', run: () => interruptAgent(a.id) });
    return rows;
  });

  const agentRows = $derived.by((): Row[] => {
    void store.rev;
    return store.agents.map((a) => {
      const n = narrate(a.id);
      return {
        id: `agent:${a.id}`, section: 'Agents', title: a.name,
        sub: `${PROVIDER_LABEL[a.provider]} · ${n?.key.replace(/`/g, '') ?? ''}`,
        agentId: a.id, state: n?.tone ?? 'idle',
        hint: 'Details',
        run: () => ui.selectAgent(a.id),
        alt: { label: a.role === 'lead' ? 'Open its request' : 'Talk 1:1', run: () => ui.openThread(store.threadOf(a.id)) },
      };
    });
  });

  const toneOf = (s: ReturnType<typeof taskState>): Tone => (s === 'working' ? 'live' : s === 'waiting' ? 'wait' : s);
  const artifactRows = $derived(
    store.artifacts.map((a) => ({
      id: `art:${a.id}`, section: 'Artifacts', title: a.title, sub: a.path, icon: 'image', hint: 'Open',
      run: () => ui.openArtifact(a.id),
    })),
  );
  const taskRows = $derived(
    store.tasks.map((t) => ({
      id: `task:${t.id}`, section: 'Tasks', title: `${t.key} · ${t.title}`, sub: TASK_STATUS_LABEL[t.status], icon: 'plan', state: toneOf(taskState(t.status)), hint: 'Details',
      run: () => ui.selectTask(t.id),
      alt: { label: 'View changes', run: () => ui.openDiff({ kind: 'task', id: t.id }) },
    })),
  );

  let searchSeq = 0;
  $effect(() => {
    const q = query.trim();
    searchError = null;
    if (scope !== '/' || q.length < 2) {
      // Nothing to search any more: a request still on its way must not answer here.
      ++searchSeq;
      searching = false;
      search = null;
      return;
    }
    const my = ++searchSeq;
    searching = true;
    const t = setTimeout(() => {
      api.search(q)
        .then((r) => my === searchSeq && (search = r))
        .catch((e) => my === searchSeq && ((search = null), (searchError = e instanceof Error ? e.message : String(e))))
        .finally(() => my === searchSeq && (searching = false));
    }, 120);
    return () => clearTimeout(t);
  });

  function fileRow(c: Citation): Row {
    return {
      id: `file:${c.path}:${c.start_line}`, section: search ? `Files · ${search.confidence} confidence` : 'Files',
      title: `${c.path}:${c.start_line}–${c.end_line}`, sub: [c.symbol, ...c.why].filter(Boolean).join(' · '), icon: 'file', hint: 'Who touched it',
      run: () => ui.selectFile(c.path),
      alt: { label: 'Ask the Lead', run: () => ui.seedComposer(ui.request, `\`${c.path}:${c.start_line}-${c.end_line}\` `) },
    };
  }

  // ───────────── ranking ─────────────
  type Scored = Row & { hits: number[] };
  const rows = $derived.by((): Scored[] => {
    const q = query.trim();
    const rank = (list: Row[], limit = 50): Scored[] => {
      if (!q) return list.slice(0, limit).map((r) => ({ ...r, hits: [] }));
      return list
        .map((r) => {
          const sub = r.sub ? fuzzy(q, r.sub) : null;
          return { r, m: fuzzy(q, r.title) ?? (sub ? { score: sub.score / 3, hits: [] as number[] } : null) };
        })
        .filter((x) => x.m)
        .sort((a, b) => b.m!.score - a.m!.score + (recents.indexOf(b.r.id) >= 0 ? 2 : 0) - (recents.indexOf(a.r.id) >= 0 ? 2 : 0))
        .slice(0, limit)
        .map((x) => ({ ...x.r, hits: x.m!.hits }));
    };
    if (scope === '>') return rank(commands);
    if (scope === '@') return rank(agentRows);
    if (scope === '#') return rank(taskRows);
    if (scope === '/') return (search?.citations ?? []).map((c) => ({ ...fileRow(c), hits: [] }));
    if (scope === '+')
      return q
        ? [{ id: 'create', section: 'New task', title: `Create “${q}”`, sub: 'Lands in Backlog; move it to Ready and a free agent picks it up', icon: 'plus', hits: [], run: () => createTask(q).then(() => ((ui.planLens = 'list'), ui.go('plan'))) }]
        : [];
    // No scope: recents first (when empty), then everything.
    const all = [...agentRows, ...taskRows, ...artifactRows, ...commands];
    if (!q) {
      const rec = recents.map((id) => all.find((r) => r.id === id)).filter((r): r is Row => !!r).map((r) => ({ ...r, section: 'Recent', hits: [] }));
      // What waits on you is suggested as a place to go, never run: Enter on an empty palette must
      // not merge or approve. Running it takes a query (or the > scope).
      const needs: Scored[] = [
        ...store.plans
          .filter((p) => p.status === 'awaiting_approval' || (p.status === 'done' && !store.planMerges[p.id]))
          .map((p) => ({ id: `go:plan:${p.id}`, section: 'Suggested', title: `${p.status === 'done' ? 'Ready to merge' : 'Ready to run'}: ${p.summary}`, icon: 'plan', hits: [], run: () => ui.openPlan(p.id, p.thread) })),
        ...store.pendingApprovals
          .filter((ap) => ap.choices.includes('allow_once'))
          .map((ap) => ({ id: `go:ap:${ap.id}`, section: 'Suggested', title: `${store.agentMap.get(ap.agent_id)?.name ?? 'An agent'} needs you: ${ap.subject.kind === 'command' ? oneLine(ap.subject.command, 50) : ap.subject.kind}`, icon: 'check', hits: [], run: () => ui.selectAgent(ap.agent_id) })),
      ];
      const suggested = agentRows.slice(0, 6).map((r) => ({ ...r, hits: [] }));
      const seen = new Set([...rec, ...needs].map((r) => r.id));
      return [...needs, ...rec.filter((r) => !needs.some((n) => n.id === r.id)), ...suggested.filter((r) => !seen.has(r.id)), ...commands.filter((c) => c.section === 'Go to' && !seen.has(c.id)).map((r) => ({ ...r, hits: [] }))];
    }
    return rank(all, 12);
  });

  const withAsk = $derived.by((): Scored[] => {
    const q = query.trim();
    if (!q || (scope && scope !== '>')) return rows;
    return [
      ...rows,
      {
        id: 'ask', section: 'Ask', title: `Ask the Lead: “${oneLine(q, 70)}”`, sub: 'Sends it to the request you are on', icon: 'chat', hits: [], hint: 'Send',
        run: () => {
          sendChat(ui.request, q);
          ui.openRequest(ui.request);
        },
        alt: { label: 'Edit first', run: () => ui.seedComposer(ui.request, q) },
      },
    ];
  });

  $effect(() => {
    void query;
    void scope;
    active = 0;
  });
  $effect(() => {
    if (active >= withAsk.length) active = Math.max(0, withAsk.length - 1);
  });

  function setScope(s: Scope) {
    scope = s;
    query = '';
    tick().then(() => inputEl?.focus());
  }

  function exec(r: Row, alt = false) {
    remember(r.id);
    if (!r.keep) close();
    (alt && r.alt ? r.alt.run : r.run)();
  }

  function onInput() {
    if (!scope && query.length >= 1 && '>@#/+'.includes(query[0])) {
      scope = query[0] as Scope;
      query = query.slice(1).trimStart();
    } else if (!scope && query === '?') {
      close();
      ui.shortcutsOpen = true;
    }
  }

  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      if (scope) setScope(null);
      else close();
    } else if (e.key === 'Backspace' && !query && scope) {
      e.preventDefault();
      setScope(null);
    } else if (e.key === 'ArrowDown' || (e.ctrlKey && e.key === 'n')) {
      e.preventDefault();
      active = Math.min(withAsk.length - 1, active + 1);
      scrollActive();
    } else if (e.key === 'ArrowUp' || (e.ctrlKey && e.key === 'p')) {
      e.preventDefault();
      active = Math.max(0, active - 1);
      scrollActive();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const r = withAsk[active];
      if (r) exec(r, e.metaKey || e.ctrlKey);
    } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
      e.preventDefault();
      e.stopPropagation();
      close();
    }
  }
  function scrollActive() {
    tick().then(() => listEl?.querySelector('[data-active="true"]')?.scrollIntoView({ block: 'nearest' }));
  }

  onMount(() => {
    inputEl?.focus();
  });

  const placeholder = $derived(scope ? SCOPES[scope].placeholder : 'Search agents, tasks, artifacts and commands — or ask the Lead…');
  const activeRow = $derived(withAsk[active]);
</script>

<div class="scrim" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && close()}>
  <div class="palette" role="dialog" aria-modal="true" aria-label="Command palette" use:modal={close}>
    <div class="input-row">
      <Icon name="search" size={16} />
      {#if scope}
        <button class="scope" onclick={() => setScope(null)} data-tip="Clear scope (⌫)">
          <Icon name={SCOPES[scope].icon} size={12} />{SCOPES[scope].label}
        </button>
      {/if}
      <input
        bind:this={inputEl}
        bind:value={query}
        {placeholder}
        spellcheck="false"
        autocomplete="off"
        aria-label="Search"
        aria-controls="palette-list"
        aria-activedescendant={activeRow ? `pr-${active}` : undefined}
        oninput={onInput}
        onkeydown={key}
      />
      {#if searching}<span class="muted small">Searching…</span>{/if}
      <kbd>esc</kbd>
    </div>

    <div class="list" id="palette-list" role="listbox" bind:this={listEl}>
      {#if searchError}
        <p class="hint-empty">Search failed: {searchError}</p>
      {:else if scope === '/' && !search && query.trim().length < 2}
        <p class="hint-empty">Type at least two characters. Results come from Codly’s index: paths, symbols and code, with the evidence for each match.</p>
      {:else if !withAsk.length && !(scope === '/' && searching)}
        <p class="hint-empty">
          {#if scope === '/'}No matches for “{query}”. Try fewer words, or ask the Lead.
          {:else if scope === '+'}Type a title, then ↵.
          {:else}Nothing matches “{query}”.{/if}
        </p>
      {/if}
      {#each withAsk as r, i (r.id)}
        {#if i === 0 || withAsk[i - 1].section !== r.section}
          <div class="section eyebrow">{r.section}</div>
        {/if}
        <button
          id="pr-{i}"
          class="row"
          role="option"
          aria-selected={i === active}
          data-active={i === active}
          onpointermove={() => (active = i)}
          onclick={(e) => exec(r, e.metaKey || e.ctrlKey)}
        >
          <span class="lead">
            {#if r.agentId}<Portrait agent={store.agentMap.get(r.agentId)} size={20} />
            {:else}<Icon name={r.icon ?? 'arrow-right'} size={15} />{/if}
          </span>
          <span class="title truncate" class:mono-t={r.id.startsWith('file:')}>
            {#each highlight(r.title, r.hits) as part, pi (pi)}{#if part.hit}<mark>{part.s}</mark>{:else}{part.s}{/if}{/each}
          </span>
          {#if r.state}<StateMark tone={r.state} size={8} />{/if}
          {#if r.sub}<span class="sub truncate">{r.sub}</span>{/if}
          <span class="right">
            {#if i === active && r.alt}<span class="alt"><Kbd keys="⌘↵" /> {r.alt.label}</span>{/if}
            {#if r.kbd}<Kbd keys={r.kbd} />{:else if i === active && r.hint}<span class="alt"><Kbd keys="↵" /> {r.hint}</span>{/if}
          </span>
        </button>
      {/each}
    </div>

    <div class="foot">
      {#if !scope}
        <span><kbd>&gt;</kbd> commands</span><span><kbd>@</kbd> agents</span><span><kbd>#</kbd> tasks</span><span><kbd>/</kbd> files</span><span><kbd>+</kbd> new task</span><span><kbd>?</kbd> help</span>
      {:else}
        <span><kbd>⌫</kbd> all results</span>
      {/if}
      <span class="grow"></span>
      <span><kbd>↑</kbd><kbd>↓</kbd> move</span><span><kbd>↵</kbd> open</span>
    </div>
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; z-index: var(--z-palette); background: var(--scrim); animation: fade-in var(--d-2) var(--ease-out); }
  .palette { position: absolute; left: 50%; top: 14vh; width: min(660px, calc(100vw - 32px)); transform: translateX(-50%); display: flex; flex-direction: column; max-height: min(580px, 72vh); border-radius: var(--r-xl); background: var(--pop); box-shadow: var(--pop-shadow), 0 40px 100px -30px rgba(0, 0, 0, 0.6); overflow: hidden; animation: palette-in var(--d-4) var(--spring); }
  @keyframes palette-in { from { opacity: 0; transform: translateX(-50%) translateY(-8px) scale(0.985); } to { opacity: 1; transform: translateX(-50%); } }
  .input-row { display: flex; align-items: center; gap: 10px; height: 58px; padding: 0 16px 0 20px; box-shadow: inset 0 -1px 0 var(--line); color: var(--ink-3); flex-shrink: 0; }
  .input-row input { flex: 1; min-width: 0; border: 0; outline: 0; background: transparent; font-size: 16px; color: var(--ink-1); user-select: text; -webkit-user-select: text; }
  .scope { display: inline-flex; align-items: center; gap: 5px; height: 26px; padding: 0 10px; border-radius: var(--r-full); background: var(--surface-3); color: var(--ink-1); font-size: var(--t-sm); font-weight: var(--w-med); }
  .small { font-size: var(--t-sm); }
  .list { flex: 1; min-height: 0; overflow-y: auto; padding: 6px 8px 8px; }
  .section { padding: 12px 12px 4px; }
  .row { display: flex; align-items: center; gap: 12px; width: 100%; height: 40px; padding: 0 12px; border-radius: var(--r-md); text-align: left; color: var(--ink-1); }
  .row[data-active='true'] { background: var(--surface-3); }
  .lead { width: 20px; display: grid; place-items: center; color: var(--ink-2); flex-shrink: 0; }
  .title { flex-shrink: 1; min-width: 40px; font-size: var(--t-md); }
  .title mark { background: none; color: var(--ink-1); font-weight: var(--w-semi); text-decoration: underline; text-decoration-color: var(--aurora-m); text-underline-offset: 3px; }
  .title.mono-t { font-family: var(--font-mono); font-size: 12px; }
  .sub { flex: 1; min-width: 0; font-size: var(--t-sm); color: var(--ink-3); }
  .right { margin-left: auto; display: inline-flex; align-items: center; gap: 10px; flex-shrink: 0; }
  .alt { display: inline-flex; align-items: center; gap: 5px; font-size: var(--t-sm); color: var(--ink-3); white-space: nowrap; }
  .hint-empty { padding: 20px 16px; font-size: var(--t-md); color: var(--ink-3); line-height: var(--lh-md); }
  .foot { display: flex; align-items: center; gap: 14px; height: 38px; padding: 0 16px; box-shadow: inset 0 1px 0 var(--line); font-size: var(--t-sm); color: var(--ink-3); flex-shrink: 0; overflow: hidden; white-space: nowrap; }
  .foot span { display: inline-flex; align-items: center; gap: 4px; }
  .grow { flex: 1; }
</style>
