<!--
  Add an agent: a four-step dialog (identity · workspace · engine · briefing) that ends in a 1:1 agent.
  "Import hire…" (a .json manifest) and "Generate with AI" (the Lead drafts one) only pre-fill the
  steps for review — nothing starts until "Start agent" (hire manifests never auto-spawn). The core
  judges every manifest and re-checks it at the end; this dialog is a convenience, not a gate.
  Identity starts like Create-a-Sim: three applicants to pick from, then name, look and traits
  (green traits change how the agent works). An exported agent (codly/agent@1) keeps its career.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { HireManifest } from '../lib/gen/HireManifest';
  import type { HireReview } from '../lib/gen/HireReview';
  import type { LaunchPreview } from '../lib/gen/LaunchPreview';
  import type { Provider } from '../lib/gen/Provider';
  import { api } from '../lib/api';
  import { askForHire, startHire } from '../lib/state/hireWizard';
  import { store } from '../lib/state/store.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import {
    AVATARS, CUSTOM, MAX_BYTES, MAX_DESCRIPTION, MAX_GOAL, MAX_NAME, TEMPLATES, blankManifest, manifestFileName, manifestJson,
    type HireTemplate,
  } from '../lib/util/hire';
  import { COMPANY_LABEL, PROVIDER_LABEL, idVar } from '../lib/util/labels';
  import { ROLE_TITLES } from '../lib/util/team';
  import Dialog from '../components/Dialog.svelte';
  import Icon from '../components/Icon.svelte';
  import Portrait from '../components/Portrait.svelte';
  import ApplicantPicker from '../office/ApplicantPicker.svelte';
  import type { Applicant } from '../lib/gen/Applicant';
  import { TRAITS, clashes, rankLabel } from '../lib/util/career';

  const STEPS = ['Identity', 'Workspace', 'Engine', 'Briefing'] as const;
  const PROVIDERS: Provider[] = ['claude', 'codex', 'gemini', 'opencode', 'cursor', 'copilot'];

  const taken = store.agents.map((a) => a.color);
  const draft = blankManifest(ui.modelDialog?.provider ?? 'claude');
  draft.color = [0, 1, 2, 3, 4, 5, 6, 7].find((c) => !taken.includes(c)) ?? 0;
  draft.avatar = AVATARS[store.agents.length % AVATARS.length];

  let m = $state<HireManifest>(draft);
  let step = $state(0);
  let busy = $state(false);
  let fileInput: HTMLInputElement | undefined = $state();
  /** Where the fields came from, shown until the person edits on: a file, or the Lead's draft. */
  let loaded = $state<{ from: string; risks: string[] } | null>(null);
  let notice = $state<string[]>([]);
  let aiOpen = $state(false);
  let brief = $state('');
  let asking: AbortController | null = $state(null);
  let models = $state<string[]>([]);
  let preview = $state<LaunchPreview | null>(null);
  let review = $state<HireReview | null>(null);
  /** The person accepted the shell metacharacters in a custom command. */
  let confirmShell = $state(false);

  const custom = $derived(m.provider === CUSTOM);
  const folder = $derived(store.workspace);
  const cliOf = (p: Provider) => store.clis.find((c) => c.provider === p);
  /** Why an agent CLI cannot start now, or null. */
  function unavailable(p: Provider): string | null {
    const c = cliOf(p);
    if (!c?.path) return 'Not installed';
    return c.problem ?? (c.logged_in === false ? 'Not signed in' : null);
  }
  const problems = $derived(review?.problems ?? []);
  const risks = $derived(preview?.shell_risks ?? review?.shell_risks ?? []);
  /** Why "Start agent" is off: the core's findings, then what only this dialog knows. */
  const blocker = $derived.by(() => {
    if (!m.name.trim()) return 'Give the agent a name (step 1).';
    if (problems.length) return problems[0];
    if (custom) return 'Codly can’t launch custom commands yet. Save this as a hire file, or pick one of its agent CLIs.';
    if (!custom) {
      const why = unavailable(m.provider as Provider);
      if (why) return `${PROVIDER_LABEL[m.provider as Provider]}: ${why.toLowerCase()}.`;
    }
    if (store.agents.some((a) => !a.archived && a.name.toLowerCase() === m.name.trim().toLowerCase())) return `An agent called ${m.name.trim()} already exists.`;
    if (risks.length && !confirmShell) return 'Confirm the shell characters in the command (step 3).';
    return null;
  });

  let applicants = $state<Applicant[]>([]);
  let pickedApplicant = $state<number | null>(null);
  onMount(() => {
    api.hireApplicants(3).then((a) => (applicants = a)).catch(() => {});
  });
  function takeApplicant(i: number) {
    const a = applicants[i];
    if (!a) return;
    pickedApplicant = i;
    m.name = a.name;
    m.color = a.color;
    m.avatar = a.avatar;
    m.traits = [...a.traits];
  }
  function toggleTrait(id: string) {
    if (m.traits.includes(id)) m.traits = m.traits.filter((t) => t !== id);
    else if (m.traits.length < 3 && !m.traits.some((t) => clashes(t, id))) m.traits = [...m.traits, id];
  }

  // The core judges the form as it stands (its rules are the only copy of the rules).
  $effect(() => {
    const raw = JSON.stringify(m);
    let live = true;
    const t = setTimeout(() => api.hireReview(raw).then((r) => live && (review = r)).catch(() => live && (review = null)), 120);
    return () => ((live = false), clearTimeout(t));
  });
  $effect(() => {
    const [p, model, cmd] = [m.provider, m.model, m.command];
    let live = true;
    api.hireLaunchPreview(p, model, cmd).then((r) => live && (preview = r)).catch(() => live && (preview = null));
    return () => (live = false);
  });
  $effect(() => {
    const p = m.provider;
    models = [];
    if (p === CUSTOM) return;
    let live = true;
    api.listModels(p as Provider).then((l) => live && (models = l.slice(0, 8))).catch(() => {});
    return () => (live = false);
  });
  // A risky command needs a fresh confirmation whenever it changes.
  $effect(() => {
    void m.command;
    confirmShell = false;
  });

  const close = () => {
    asking?.abort();
    ui.modelDialog = null;
  };

  function pickEngine(p: string) {
    if (p === m.provider) return;
    if (p === CUSTOM) {
      m.command = preview && !preview.editable ? preview.command : '';
    } else m.command = null;
    m.model = null; // a model belongs to its CLI
    m.provider = p;
  }

  function apply(r: HireReview, from: string): boolean {
    if (!r.manifest) {
      notice = r.problems.length ? r.problems : ['That is not a hire manifest.'];
      return false;
    }
    const next = { ...blankManifest(), ...r.manifest };
    next.avatar ??= draft.avatar;
    next.color ??= draft.color;
    m = next;
    step = 0;
    notice = [];
    loaded = { from, risks: r.shell_risks };
    aiOpen = false;
    return true;
  }

  async function importFile(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    if (file.size > MAX_BYTES) {
      notice = [`${file.name} is ${Math.ceil(file.size / 1024)} KB; hire files are at most ${MAX_BYTES / 1024} KB.`];
      return;
    }
    try {
      apply(await api.hireReview(await file.text()), file.name);
    } catch (err) {
      notice = [String(err)];
    }
  }

  async function generate() {
    if (!brief.trim() || asking) return;
    const ctl = new AbortController();
    asking = ctl;
    notice = [];
    try {
      apply(await askForHire(brief, ctl.signal), 'the AI draft');
    } catch (err) {
      if (!ctl.signal.aborted) notice = [err instanceof Error ? err.message : String(err)];
    } finally {
      asking = null;
    }
  }

  function useTemplate(t: HireTemplate) {
    m.description = t.description;
    m.goal = t.goal;
    m.title ??= t.title;
  }

  function saveFile() {
    const url = URL.createObjectURL(new Blob([manifestJson(m)], { type: 'application/json' }));
    const a = Object.assign(document.createElement('a'), { href: url, download: manifestFileName(m) });
    a.click();
    URL.revokeObjectURL(url);
    toast({ kind: 'success', title: 'Hire file saved', body: `${a.download} can be imported again from this dialog.` });
  }

  async function next() {
    if (step < STEPS.length - 1) {
      step += 1;
      return;
    }
    if (blocker || busy) return;
    busy = true;
    const done = await startHire(m, confirmShell);
    busy = false;
    if (done) close();
  }

  const stepBlocked = $derived(step === 0 && !m.name.trim());
  const portrait = (seed: string) => ({ id: `wizard-${seed}`, color: m.color ?? 0, look_seed: seed, provider: (custom ? 'claude' : m.provider) as Provider, role: 'solo' as const, name: m.name });
</script>

<Dialog label="Add an agent" style="max-height: calc(100vh - 48px)" divider={false} onclose={close}>
  {#snippet actions()}
    <div class="starts">
      <button type="button" class="btn sm ghost" onclick={() => fileInput?.click()}><Icon name="reveal" size={13} />Import agent…</button>
      <button type="button" class="btn sm ghost" aria-expanded={aiOpen} onclick={() => (aiOpen = !aiOpen)}><Icon name="sparkle" size={13} />Generate with AI</button>
    </div>
    <input bind:this={fileInput} class="sr-only" type="file" accept=".json,application/json" tabindex="-1" onchange={importFile} />
  {/snippet}

  <ol class="steps" aria-label="Steps">
    {#each STEPS as s, i (s)}
      <li>
        <button type="button" class="step" class:on={i === step} class:done={i < step} aria-current={i === step ? 'step' : undefined} onclick={() => (step = i)}>
          <span class="n">{i + 1}</span>{s}
        </button>
      </li>
    {/each}
  </ol>

  <div class="body">
    {#if aiOpen}
      <div class="callout">
        <label class="field">
          <span class="eyebrow">What should this agent do?</span>
          <textarea rows="2" bind:value={brief} placeholder="e.g. Triage new bug reports and write up the likely cause" disabled={!!asking}></textarea>
        </label>
        <p class="muted small">The Lead drafts a hire and you review every field. It appears in the Lead’s conversation; nothing starts on its own.</p>
        <div class="row">
          {#if asking}
            <span class="muted small" role="status">Waiting for the Lead…</span>
            <button type="button" class="btn sm ghost" onclick={() => asking?.abort()}>Cancel</button>
          {:else}
            <button type="button" class="btn sm primary" disabled={!brief.trim()} onclick={generate}>Ask the Lead</button>
          {/if}
        </div>
      </div>
    {/if}
    {#if loaded}
      <div class="callout info" role="status">
        <b>Filled from {loaded.from}.</b> Check each step; nothing starts until you press Start agent.
        {#if loaded.risks.length}<br />It has a custom command with {loaded.risks.join(', ')}: you will be asked to confirm it.{/if}
      </div>
    {/if}
    {#if notice.length}
      <div class="callout bad" role="alert">
        {#each notice as n (n)}<div>{n}</div>{/each}
      </div>
    {/if}

    {#if step === 0}
      {#if applicants.length && !m.career}
        <div class="field">
          <span class="eyebrow">Applicants</span>
          <ApplicantPicker {applicants} provider={(custom ? 'claude' : m.provider) as Provider} role="solo" picked={pickedApplicant} onpick={takeApplicant} />
        </div>
      {/if}
      {#if m.career}
        <div class="callout info">
          <b>{rankLabel(m.career.rank)} · {m.career.xp} XP</b> — {m.career.stats.merged} merged, {m.career.stats.checks_passed} checks passed{m.memory ? ', brings its memory file' : ''}. Its track record carries over.
        </div>
      {/if}
      <label class="field">
        <span class="eyebrow">Name</span>
        <input bind:value={m.name} maxlength={MAX_NAME} placeholder="e.g. Mina" spellcheck="false" autocomplete="off" onkeydown={(e) => e.key === 'Enter' && (e.preventDefault(), !stepBlocked && next())} />
      </label>
      <div class="field">
        <span class="eyebrow">Avatar</span>
        <div class="avatars" role="radiogroup" aria-label="Avatar">
          {#each AVATARS as seed (seed)}
            <button type="button" class="avatar" role="radio" aria-checked={m.avatar === seed} aria-label={seed} class:on={m.avatar === seed} onclick={() => (m.avatar = seed)}>
              <Portrait agent={portrait(seed)} size={44} />
            </button>
          {/each}
        </div>
      </div>
      <div class="field">
        <span class="eyebrow">Colour</span>
        <div class="swatches" role="radiogroup" aria-label="Colour">
          {#each [0, 1, 2, 3, 4, 5, 6, 7] as c (c)}
            <button type="button" class="swatch" role="radio" aria-checked={m.color === c} aria-label={`Colour ${c + 1}`} class:on={m.color === c} style:--hue={idVar(c)} onclick={() => (m.color = c)}></button>
          {/each}
        </div>
      </div>
      <div class="field">
        <span class="eyebrow">Traits <span class="muted small">up to 3 · green ones change how it works</span></span>
        <div class="traits" role="group" aria-label="Traits">
          {#each TRAITS as t (t.id)}
            {@const on = m.traits.includes(t.id)}
            <button type="button" class="trait" class:on class:work={t.rule} aria-pressed={on} title={t.rule ?? 'Personality only (how it lives in the office)'} disabled={!on && (m.traits.length >= 3 || m.traits.some((x) => clashes(x, t.id)))} onclick={() => toggleTrait(t.id)}>{t.label}</button>
          {/each}
        </div>
        {#each m.traits.map((id) => TRAITS.find((t) => t.id === id)).filter((t) => t?.rule) as t (t?.id)}
          <span class="muted small">{t?.label}: {t?.rule}</span>
        {/each}
      </div>
      <label class="field">
        <span class="eyebrow">Title</span>
        <select value={m.title ?? ''} onchange={(e) => (m.title = e.currentTarget.value || null)}>
          <option value="">None</option>
          {#each ROLE_TITLES as t (t)}<option value={t}>{t}</option>{/each}
        </select>
      </label>
    {:else if step === 1}
      <div class="field">
        <span class="eyebrow">Project folder</span>
        <div class="path" title={folder?.repo_path}>
          <Icon name="reveal" size={14} />
          <span class="mono">{folder?.repo_path ?? 'No project open'}</span>
          {#if folder?.current_branch}<span class="muted small">on {folder.current_branch}</span>{/if}
        </div>
        <p class="muted small">Agents start in the project that is open. Open another repository from the Welcome screen to hire into it.</p>
      </div>
      <label class="check">
        <input type="checkbox" bind:checked={m.isolate} />
        <span><b>Git isolation (own worktree)</b><span class="muted small block">Edits land on a branch of its own that you review and merge. Recommended.</span></span>
      </label>
      {#if !m.isolate}
        <div class="callout warn" role="note">
          This agent edits your project folder directly, on whatever branch is checked out, and Codly will not commit for it. Your approval mode still applies, but Autonomous allows edits here.
        </div>
      {/if}
      <label class="field">
        <span class="eyebrow">Resume a session <span class="muted">(optional)</span></span>
        <input value={m.resume_session ?? ''} oninput={(e) => (m.resume_session = e.currentTarget.value.trim() || null)} placeholder="Session id from the CLI" spellcheck="false" autocomplete="off" />
        <span class="muted small">Continues that conversation. If the CLI can’t resume it here, Codly starts a fresh one.</span>
      </label>
    {:else if step === 2}
      <div class="field">
        <span class="eyebrow">Engine</span>
        <div class="choices" role="radiogroup" aria-label="Engine">
          {#each PROVIDERS as p (p)}
            {@const why = unavailable(p)}
            <button type="button" class="choice" role="radio" aria-checked={m.provider === p} class:on={m.provider === p} disabled={!!why && m.provider !== p} data-tip={why ?? undefined} onclick={() => pickEngine(p)}>
              <span class="radio"></span>{PROVIDER_LABEL[p]} <span class="muted small">{COMPANY_LABEL[p]}</span>
            </button>
          {/each}
          <button type="button" class="choice" role="radio" aria-checked={custom} class:on={custom} onclick={() => pickEngine(CUSTOM)}>
            <span class="radio"></span>Custom <span class="muted small">your command</span>
          </button>
        </div>
      </div>
      {#if !custom}
        <div class="field">
          <span class="eyebrow">Model</span>
          <div class="choices" role="radiogroup" aria-label="Model">
            <button type="button" class="choice" role="radio" aria-checked={!m.model} class:on={!m.model} onclick={() => (m.model = null)}><span class="radio"></span>CLI default</button>
            {#each models as name (name)}
              <button type="button" class="choice" role="radio" aria-checked={m.model === name} class:on={m.model === name} onclick={() => (m.model = name)}><span class="radio"></span>{name}</button>
            {/each}
          </div>
          <input value={m.model ?? ''} oninput={(e) => (m.model = e.currentTarget.value.trim() || null)} placeholder="Other model name" spellcheck="false" autocomplete="off" />
        </div>
      {/if}
      <div class="field">
        <span class="eyebrow">Launch command</span>
        {#if custom}
          <input class="mono" value={m.command ?? ''} oninput={(e) => (m.command = e.currentTarget.value)} placeholder="my-agent --acp" spellcheck="false" autocomplete="off" aria-label="Launch command" />
          <p class="muted small">Run as given, without a shell. Anything Codly can’t launch yet stays a saved hire file.</p>
        {:else}
          <input class="mono" value={preview?.command ?? ''} readonly aria-label="Launch command" />
          <p class="muted small">
            Codly adds the session, tool and MCP flags for each run.
            <button type="button" class="link" onclick={() => pickEngine(CUSTOM)}>Edit as a custom command</button>
          </p>
        {/if}
        {#if custom && preview?.problems.length}<p class="bad small">{preview.problems[0]}</p>{/if}
        {#if custom && risks.length}
          <div class="callout warn" role="alert">
            <label class="check">
              <input type="checkbox" bind:checked={confirmShell} />
              <span>This command contains {risks.join(', ')}. Codly runs it without a shell, so these are passed as plain text, which is rarely what was meant. I checked the command and want to keep it.</span>
            </label>
          </div>
        {/if}
      </div>
    {:else}
      <div class="field">
        <span class="eyebrow">Start from</span>
        <div class="choices">
          {#each TEMPLATES as t (t.id)}
            <button type="button" class="choice" class:on={m.description === t.description && m.goal === t.goal} onclick={() => useTemplate(t)}>{t.label}</button>
          {/each}
        </div>
      </div>
      <label class="field">
        <span class="eyebrow">Description <span class="muted">what this agent is</span></span>
        <textarea rows="3" bind:value={m.description} maxlength={MAX_DESCRIPTION} placeholder="A few sentences on its role."></textarea>
        <span class="count muted small">{m.description.length}/{MAX_DESCRIPTION}</span>
      </label>
      <label class="field">
        <span class="eyebrow">Goal <span class="muted">long-running</span></span>
        <textarea rows="4" bind:value={m.goal} maxlength={MAX_GOAL} placeholder="What it keeps working towards."></textarea>
        <span class="count muted small">{m.goal.length}/{MAX_GOAL}. Put in front of every message you send this agent.</span>
      </label>
      {#if blocker}<p class="bad small" role="status">{blocker}</p>{/if}
    {/if}
  </div>

  <footer>
    <button type="button" class="btn sm ghost save" onclick={saveFile} disabled={!m.name.trim()} data-tip="Download these settings as a .json hire file">Save as .json</button>
    <button type="button" class="btn sm ghost" onclick={() => (step > 0 ? (step -= 1) : close())}>{step > 0 ? 'Back' : 'Cancel'}</button>
    {#if step < STEPS.length - 1}
      <button type="button" class="btn sm primary" disabled={stepBlocked} onclick={next}>Next</button>
    {:else}
      <button type="button" class="btn sm primary" disabled={!!blocker || busy} onclick={next}>Start agent</button>
    {/if}
  </footer>
</Dialog>

<style>
  .starts { display: flex; align-items: center; gap: 4px; flex-wrap: wrap; justify-content: flex-end; margin: 0 4px 0 12px; }
  .steps { display: flex; gap: 4px; list-style: none; margin: 0; padding: 0 22px 12px; box-shadow: inset 0 -1px 0 var(--line); flex-shrink: 0; }
  .traits { display: flex; flex-wrap: wrap; gap: 6px; }
  .trait { height: 26px; padding: 0 11px; border-radius: var(--r-full); background: var(--surface-2); color: var(--ink-2); font-size: var(--t-sm); }
  .trait.work { box-shadow: inset 0 0 0 1px color-mix(in oklab, #3fcf72 55%, transparent); }
  .trait.on { background: var(--surface-3); color: var(--ink-1); box-shadow: inset 0 0 0 1.5px var(--ink-2); }
  .trait.work.on { box-shadow: inset 0 0 0 2px #3fcf72; }
  .trait:disabled { opacity: 0.4; }
  .step { display: inline-flex; align-items: center; gap: 7px; height: 28px; padding: 0 12px 0 6px; border-radius: var(--r-full); color: var(--ink-3); font-size: var(--t-sm); font-weight: var(--w-med); }
  .step:hover { background: var(--surface-2); color: var(--ink-1); }
  .step.on { background: var(--surface-3); color: var(--ink-1); }
  .n { display: grid; place-items: center; width: 18px; height: 18px; border-radius: 50%; font-size: 11px; box-shadow: inset 0 0 0 1.5px var(--line-3); }
  .on .n { background: var(--ink-1); color: var(--pop); box-shadow: none; }
  .done .n { box-shadow: inset 0 0 0 1.5px var(--ink-2); color: var(--ink-2); }
  .body { display: flex; flex-direction: column; gap: 16px; padding: 20px 26px; min-height: 340px; overflow-y: auto; }
  .field { display: flex; flex-direction: column; gap: 8px; }
  .row { display: flex; align-items: center; justify-content: flex-end; gap: 8px; }
  .callout { display: flex; flex-direction: column; gap: 8px; padding: 12px 14px; border-radius: var(--r-md); background: var(--surface-2); font-size: var(--t-sm); color: var(--ink-2); }
  .callout.info, .callout.warn { display: block; line-height: 1.45; }
  .callout.info { box-shadow: inset 0 0 0 1px var(--line-2); }
  .callout.warn { box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--amber) 55%, transparent); }
  .callout.bad { color: var(--bad); background: var(--bad-soft); }
  .avatars { display: flex; flex-wrap: wrap; gap: 8px; }
  .avatar { display: grid; place-items: center; width: 56px; height: 56px; border-radius: var(--r-lg); background: var(--surface-2); }
  .avatar:hover { background: var(--surface-3); }
  .avatar.on { background: var(--surface-3); box-shadow: inset 0 0 0 1.5px var(--ink-2); }
  .swatches { display: flex; gap: 8px; }
  .swatch { width: 26px; height: 26px; border-radius: 50%; background: var(--hue); box-shadow: inset 0 0 0 2px var(--pop); }
  .swatch.on { box-shadow: inset 0 0 0 2px var(--pop), 0 0 0 2px var(--ink-1); }
  .path { display: flex; align-items: center; gap: 8px; min-height: 36px; padding: 6px 12px; border-radius: var(--r-md); background: var(--surface-2); box-shadow: inset 0 0 0 1px var(--line); min-width: 0; }
  .path .mono { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; direction: rtl; text-align: left; flex: 1; min-width: 0; }
  .check { display: flex; align-items: flex-start; gap: 10px; font-size: var(--t-md); cursor: pointer; }
  .check input { margin-top: 3px; flex-shrink: 0; }
  .block { display: block; }
  .choices { display: flex; flex-wrap: wrap; gap: 6px; }
  .choice { display: inline-flex; align-items: center; gap: 8px; height: 30px; padding: 0 12px 0 10px; border-radius: var(--r-full); background: var(--surface-2); font-weight: var(--w-med); font-size: var(--t-sm); }
  .choice:hover:not(:disabled) { background: var(--surface-3); }
  .choice:disabled { opacity: 0.45; cursor: not-allowed; }
  .choice.on { background: var(--surface-3); box-shadow: inset 0 0 0 1.5px var(--ink-2); }
  .radio { width: 12px; height: 12px; border-radius: 50%; box-shadow: inset 0 0 0 1.5px var(--line-3); flex-shrink: 0; }
  .on .radio { box-shadow: inset 0 0 0 4px var(--ink-1); }
  .link { color: var(--ink-1); font-weight: var(--w-med); text-decoration: underline; text-decoration-color: var(--line-3); text-underline-offset: 3px; }
  .muted { color: var(--ink-3); font-weight: var(--w-reg); }
  .bad { color: var(--bad); }
  .mono { font-family: var(--font-mono); font-size: var(--t-sm); }
  .count { align-self: flex-end; }
  input.mono { font-family: var(--font-mono); font-size: var(--t-sm); }
  textarea { padding: 9px 12px; resize: vertical; line-height: 1.45; }
  input[readonly] { color: var(--ink-2); }
  footer { display: flex; justify-content: flex-end; gap: 8px; padding: 14px 26px 22px; flex-shrink: 0; box-shadow: inset 0 1px 0 var(--line); }
  .save { margin-right: auto; }
</style>
