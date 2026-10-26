<!--
  Settings → Accounts: every sign-in of every CLI, grouped by CLI. With one sign-in per CLI it
  is one quiet row each (state, plan, quota) and an "Add another sign-in" link. With several,
  each row gets a name, an on/off switch and a place in the order (drag the grip or use ↑↓),
  and the spreading options appear. A sign-in that is not signed in shows the exact command
  to run in a terminal.
-->
<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api } from '../lib/api';
  import type { AccountInfo } from '../lib/gen/AccountInfo';
  import type { AccountStrategy } from '../lib/gen/AccountStrategy';
  import type { Provider } from '../lib/gen/Provider';
  import { copyText, refreshAccounts, saveAccounts, updateSettings } from '../lib/state/actions';
  import { accounts } from '../lib/state/accounts.svelte';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { ACCOUNT_VAR, asConfigs, derivedLabel, HOME_SUGGESTION, moveItem, normalizeHome, supportsAccounts, usable, validHome } from '../lib/util/accounts';
  import { PROVIDER_LABEL } from '../lib/util/labels';
  import { tildify } from '../lib/util/sanitize';
  import BillingBadge from '../components/BillingBadge.svelte';
  import Icon from '../components/Icon.svelte';
  import InlineText from '../components/InlineText.svelte';
  import UsageBars from '../components/UsageBars.svelte';

  const ORDER: Provider[] = ['claude', 'codex', 'gemini', 'opencode', 'cursor', 'copilot'];
  const s = $derived(store.settings);
  const groups = $derived(ORDER.map((p) => ({ p, list: accounts.list.filter((a) => a.provider === p) })).filter((g) => g.list.length));
  const several = $derived(accounts.multi.size > 0);
  const listOf = (p: Provider) => accounts.list.filter((a) => a.provider === p);

  // ── refresh ──
  let refreshing = $state(false);
  async function refresh() {
    refreshing = true;
    await refreshAccounts(true);
    refreshing = false;
  }
  onMount(() => void refreshAccounts(false));

  // ── edits (each one saves the CLI's whole list, in order) ──
  function save(p: Provider, list: AccountInfo[], probe = false) {
    return saveAccounts(p, asConfigs(list, store.settings.accounts), probe);
  }
  const toggle = (a: AccountInfo) => save(a.provider, listOf(a.provider).map((x) => (x.id === a.id ? { ...x, enabled: !x.enabled } : x)));
  function move(a: AccountInfo, to: number) {
    const list = listOf(a.provider);
    const from = list.findIndex((x) => x.id === a.id);
    const next = moveItem(list, from, to);
    if (next !== list) void save(a.provider, next);
  }
  const remove = (a: AccountInfo) => save(a.provider, listOf(a.provider).filter((x) => x.id !== a.id));
  function rename(a: AccountInfo, el: HTMLInputElement) {
    const t = el.value.trim();
    const label = t && t !== derivedLabel(a.provider, a.home) ? t : null;
    if ((t || derivedLabel(a.provider, a.home)) === a.label) {
      el.value = a.label;
      return;
    }
    const list = listOf(a.provider);
    const configs = asConfigs(list, store.settings.accounts).map((c, i) => (list[i].id === a.id ? { ...c, label } : c));
    void saveAccounts(a.provider, configs);
  }
  function labelKey(e: KeyboardEvent, a: AccountInfo) {
    const el = e.currentTarget as HTMLInputElement;
    if (e.key === 'Enter') el.blur();
    else if (e.key === 'Escape') {
      e.stopPropagation();
      el.value = a.label;
      el.blur();
    }
  }

  // ── drag to reorder (pointer events: HTML5 drag-and-drop is unreliable in the webview) ──
  let drag = $state<{ id: string; provider: Provider; from: number; over: number } | null>(null);
  function gripDown(e: PointerEvent, a: AccountInfo, i: number) {
    if (e.button !== 0) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag = { id: a.id, provider: a.provider, from: i, over: i };
  }
  function gripMove(e: PointerEvent) {
    if (!drag) return;
    const rows = [...document.querySelectorAll<HTMLElement>(`[data-acct-group="${drag.provider}"] > li`)];
    let over = rows.length - 1;
    for (let i = 0; i < rows.length; i++) {
      const r = rows[i].getBoundingClientRect();
      if (e.clientY < r.top + r.height / 2) {
        over = i;
        break;
      }
    }
    drag.over = over;
  }
  function gripUp() {
    if (!drag) return;
    const d = drag;
    drag = null;
    const a = accounts.byId.get(d.id);
    if (a && d.over !== d.from) move(a, d.over);
  }
  function gripKey(e: KeyboardEvent, a: AccountInfo, i: number) {
    if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
      e.preventDefault();
      move(a, i + (e.key === 'ArrowUp' ? -1 : 1));
    }
  }

  // ── add a sign-in ──
  const addable = $derived(ORDER.filter((p) => supportsAccounts(p) && store.clis.some((c) => c.provider === p && c.path)));
  let adding = $state(false);
  let addP = $state<Provider>('claude');
  let addHome = $state('');
  let addLabel = $state('');
  let addErr = $state<string | null>(null);
  let fresh = $state<{ provider: Provider; home: string } | null>(null);
  let homeInput: HTMLInputElement | undefined = $state();
  async function openAdd() {
    addP = addable.includes(addP) ? addP : addable[0];
    addHome = '';
    addLabel = '';
    addErr = null;
    adding = true;
    await tick();
    homeInput?.focus();
  }
  async function chooseFolder() {
    const f = await api.pickFolder();
    if (f) {
      addHome = tildify(f);
      addErr = null;
    }
  }
  async function add() {
    const home = normalizeHome(addHome || HOME_SUGGESTION[addP] || '');
    if (!validHome(home)) {
      addErr = 'Use a full path, or one under ~ like ~/.claude-work.';
      return;
    }
    const list = listOf(addP);
    if (list.some((a) => a.home && (a.home === home || tildify(a.home) === home))) {
      addErr = 'That folder is already listed.';
      return;
    }
    const configs = [...asConfigs(list, store.settings.accounts), { provider: addP, home, label: addLabel.trim() || null, enabled: true }];
    adding = false;
    fresh = { provider: addP, home };
    await saveAccounts(addP, configs, true);
    await tick();
    document.querySelector('.acct.fresh')?.scrollIntoView({ block: 'nearest', behavior: ui.reducedMotion ? 'auto' : 'smooth' });
  }
  const isFresh = (a: AccountInfo) =>
    !!fresh && !!a.home && a.provider === fresh.provider && (a.home === fresh.home || tildify(a.home) === fresh.home || (fresh.home.startsWith('~/') && a.home.endsWith(fresh.home.slice(1))));

  // ── spreading ──
  const STRATEGY: { id: AccountStrategy; label: string; body: string }[] = [
    { id: 'spread', label: 'Spread by quota left', body: 'The sign-in with the most quota left takes the next task.' },
    { id: 'in_order', label: 'In order', body: 'Your first sign-in takes the work; the next one takes over near its limit.' },
  ];
  const leadProvider = $derived(s.lead_provider ?? 'claude');
  const leadChoices = $derived(accounts.multi.has(leadProvider) ? listOf(leadProvider).filter(usable) : []);

  /** The problem line, unless it only repeats the login command shown below it. */
  const problemText = (a: AccountInfo) => (a.problem && !(a.login_command && a.problem.includes(a.login_command)) ? a.problem : null);
</script>

<section>
  <div class="s-head">
    <h3 class="eyebrow">Accounts</h3>
    <button class="btn sm ghost" onclick={refresh} disabled={refreshing} data-tip="Check again who is signed in to each CLI">
      <span class="ic" class:spin={refreshing}><Icon name="refresh" size={13} /></span>{refreshing ? 'Checking…' : 'Refresh'}
    </button>
  </div>
  <p class="desc">
    {#if several}Sign-ins Codly may use, in your order of preference. Each agent runs on one of them.
    {:else}The sign-in each CLI uses here, and the quota its agents last reported.{/if}
  </p>

  {#if !groups.length}
    <p class="desc">No CLI found yet. Install one, then Refresh.</p>
  {/if}

  {#each groups as g (g.p)}
    {@const many = g.list.length > 1}
    <div class="group">
      <div class="g-head">
        <span class="g-name">{PROVIDER_LABEL[g.p]}</span>
        {#if many}<span class="g-n num">{g.list.filter(usable).length} of {g.list.length} in use</span>{/if}
      </div>
      <ul class="accts" data-acct-group={g.p}>
        {#each g.list as a, i (a.id)}
          {@const out = a.logged_in === false}
          {@const fix = out || !!a.problem}
          <li
            class="acct"
            class:many
            class:off={!a.enabled}
            class:fresh={isFresh(a)}
            class:dragging={drag?.id === a.id}
            class:drop-before={drag && drag.provider === g.p && drag.over === i && drag.over < drag.from}
            class:drop-after={drag && drag.provider === g.p && drag.over === i && drag.over > drag.from}
          >
            <div class="a-top">
              {#if many}
                <button class="grip" aria-label="Move {a.label}: drag, or use the arrow keys" onpointerdown={(e) => gripDown(e, a, i)} onpointermove={gripMove} onpointerup={gripUp} onpointercancel={() => (drag = null)} onkeydown={(e) => gripKey(e, a, i)}><Icon name="grip" size={14} /></button>
                <input class="a-label" value={a.label} aria-label="Name of this {PROVIDER_LABEL[a.provider]} sign-in" spellcheck="false" onblur={(e) => rename(a, e.currentTarget)} onkeydown={(e) => labelKey(e, a)} />
                {#if a.primary}<span class="chip sm prim" data-tip="Your login shell uses this sign-in: it is what the CLI does in your terminal.">your terminal’s sign-in</span>{/if}
              {:else}
                <span class="a-state" class:ok={a.logged_in === true} class:warn={out}><span class="dot"></span>{a.logged_in === true ? 'Signed in' : out ? 'Not signed in' : 'Sign-in not checked'}</span>
                {#if a.identity}<span class="ident truncate selectable">{a.identity}</span>{/if}
              {/if}
              <span class="grow"></span>
              {#if !out}<BillingBadge billing={a.billing} plan={a.plan} />{/if}
              {#if many || !a.enabled}
                <input type="checkbox" class="switch" checked={a.enabled} aria-label="Let Codly use {a.label}" data-tip={a.enabled ? 'Codly may run agents on it' : 'Codly leaves it alone'} onchange={() => toggle(a)} />
              {/if}
            </div>

            {#if many || a.home}
              <p class="a-meta">
                {#if many}
                  <span class="a-state" class:ok={a.logged_in === true} class:warn={out}><span class="dot"></span>{a.logged_in === true ? 'Signed in' : out ? 'Not signed in' : 'Sign-in not checked'}</span>
                  {#if a.identity}<span class="sep">·</span><span class="selectable">{a.identity}</span>{/if}
                {/if}
                {#if a.home}{#if many}<span class="sep">·</span>{/if}<code class="home selectable" data-tip="{ACCOUNT_VAR[a.provider] ?? 'Home'} = {a.home}">{tildify(a.home)}</code>{/if}
                {#if many}
                  <span class="grow"></span>
                  <span class="a-actions">
                    <button class="icon-btn sm" aria-label="Prefer {a.label} more" disabled={i === 0} onclick={() => move(a, i - 1)}><Icon name="chevron-up" size={14} /></button>
                    <button class="icon-btn sm" aria-label="Prefer {a.label} less" disabled={i === g.list.length - 1} onclick={() => move(a, i + 1)}><Icon name="chevron-down" size={14} /></button>
                    {#if a.configured && !a.primary}<button class="btn sm ghost rm" onclick={() => remove(a)}>Remove</button>{/if}
                  </span>
                {/if}
              </p>
            {/if}

            {#if fix}
              <div class="fix">
                {#if problemText(a)}<p class="f-text"><InlineText text={problemText(a) ?? ''} /></p>{/if}
                {#if out && a.login_command}
                  <div class="cmd">
                    <code class="selectable">{a.login_command}</code>
                    <button class="btn sm" onclick={() => copyText(a.login_command ?? '', 'Command copied')}><Icon name="copy" size={12} />Copy</button>
                  </div>
                  <p class="f-hint">Run this in a terminal, then Refresh.</p>
                {/if}
              </div>
            {/if}

            {#if a.limits.length && !out}
              <div class="a-usage"><UsageBars limits={a.limits} now={ui.now} /></div>
            {/if}

          </li>
        {/each}
      </ul>
    </div>
  {/each}

  {#if adding}
    <div class="add" role="group" aria-label="Add a sign-in">
      <p class="r-title">Add a sign-in</p>
      <div class="choices" role="radiogroup" aria-label="CLI">
        {#each addable as p (p)}
          <button class="choice" role="radio" aria-checked={addP === p} class:on={addP === p} onclick={() => { addP = p; addErr = null; }}><span class="radio"></span>{PROVIDER_LABEL[p]}</button>
        {/each}
      </div>
      <label class="field">
        <span class="f-label">Folder</span>
        <span class="f-row">
          <input class="input mono" bind:this={homeInput} bind:value={addHome} placeholder={HOME_SUGGESTION[addP]} spellcheck="false" autocomplete="off" oninput={() => (addErr = null)} onkeydown={(e) => e.key === 'Enter' && add()} />
          <button class="btn sm" onclick={chooseFolder}><Icon name="folder" size={13} />Choose…</button>
        </span>
      </label>
      <label class="field">
        <span class="f-label">Name <span class="muted">(optional)</span></span>
        <input class="input" bind:value={addLabel} placeholder={derivedLabel(addP, normalizeHome(addHome) || HOME_SUGGESTION[addP] || null)} spellcheck="false" onkeydown={(e) => e.key === 'Enter' && add()} />
      </label>
      <p class="desc">Codly runs {PROVIDER_LABEL[addP]} with <code>{ACCOUNT_VAR[addP]}</code> set to this folder. A new folder starts signed out; Codly then shows the command that signs it in.</p>
      {#if addErr}<p class="err">{addErr}</p>{/if}
      <div class="add-actions">
        <button class="btn sm primary" onclick={add}>Add</button>
        <button class="btn sm ghost" onclick={() => (adding = false)}>Cancel</button>
      </div>
    </div>
  {:else if addable.length}
    {#if groups.some((g) => g.list.length > 1)}
      <div><button class="btn sm ghost" onclick={openAdd}><Icon name="plus" size={13} />Add a sign-in</button></div>
    {:else}
      <p class="desc">Have a second subscription? <button class="link" onclick={openAdd}>Add another sign-in</button></p>
    {/if}
  {/if}

  {#if several}
    <div class="row">
      <div class="r-text"><span class="r-title">Spreading work</span><span class="desc">{STRATEGY.find((x) => x.id === s.account_strategy)?.body}</span></div>
      <div class="lenses" role="radiogroup" aria-label="How work spreads over sign-ins">
        {#each STRATEGY as st (st.id)}
          <button role="radio" aria-checked={s.account_strategy === st.id} onclick={() => updateSettings({ account_strategy: st.id })}>{st.label}</button>
        {/each}
      </div>
    </div>
    {#if leadChoices.length > 1}
      <div class="row">
        <div class="r-text"><span class="r-title">The Lead runs on</span><span class="desc">Its own planning turns count against this sign-in’s quota.</span></div>
        <select class="input" aria-label="The Lead’s sign-in" value={s.lead_account ?? ''} onchange={(e) => updateSettings({ lead_account: (e.currentTarget as HTMLSelectElement).value || null })}>
          <option value="">Most quota left</option>
          {#each leadChoices as a (a.id)}<option value={a.id}>{PROVIDER_LABEL[a.provider]} · {a.label}</option>{/each}
        </select>
      </div>
    {/if}
  {/if}
  <div class="row">
    <div class="r-text"><span class="r-title">Workers per account</span><span class="desc">At most this many at once on one sign-in, so no subscription hits its rate limit.</span></div>
    <div class="stepper">
      <button class="icon-btn sm" aria-label="Fewer" onclick={() => updateSettings({ max_per_provider: Math.max(1, s.max_per_provider - 1) })}><Icon name="minus" size={14} /></button>
      <span class="num">{s.max_per_provider}</span>
      <button class="icon-btn sm" aria-label="More" onclick={() => updateSettings({ max_per_provider: Math.min(s.max_parallel, 8, s.max_per_provider + 1) })}><Icon name="plus" size={14} /></button>
    </div>
  </div>
</section>

<style>
  section { padding: 20px 0; box-shadow: inset 0 -1px 0 var(--line); display: flex; flex-direction: column; gap: 12px; }
  .s-head { display: flex; align-items: center; justify-content: space-between; margin: -4px -8px -4px 0; }
  .ic { display: inline-grid; }
  .spin { animation: spin 0.9s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .desc code { font-family: var(--font-mono); font-size: 11px; color: var(--ink-2); }
  .muted { color: var(--ink-3); font-weight: var(--w-reg); }
  .group { display: flex; flex-direction: column; gap: 6px; }
  .g-head { display: flex; align-items: baseline; justify-content: space-between; padding: 0 2px; }
  .g-name { font-weight: var(--w-semi); font-size: var(--t-md); }
  .g-n { font-size: var(--t-xs); color: var(--ink-3); }
  .accts { display: flex; flex-direction: column; border-radius: var(--r-md); background: var(--surface-1); }
  .acct { position: relative; display: flex; flex-direction: column; gap: 8px; padding: 11px 12px 12px 14px; transition: background-color var(--d-2) var(--ease-out), opacity var(--d-2) var(--ease-out); }
  .acct + .acct { box-shadow: inset 0 1px 0 var(--line); }
  .acct.many { padding-left: 6px; }
  .acct.many > :not(.a-top) { margin-left: 26px; }
  .acct.off .a-label, .acct.off .a-meta, .acct.off .a-usage { opacity: 0.55; }
  .acct.fresh { background: color-mix(in srgb, var(--focus) 8%, transparent); }
  .acct.dragging { background: var(--surface-2); opacity: 0.7; }
  .acct.drop-before::before, .acct.drop-after::after { content: ''; position: absolute; left: 10px; right: 10px; height: 2px; border-radius: 1px; background: var(--focus); }
  .acct.drop-before::before { top: -1px; }
  .acct.drop-after::after { bottom: -1px; }
  .acct:first-child { border-radius: var(--r-md) var(--r-md) 0 0; }
  .acct:last-child { border-radius: 0 0 var(--r-md) var(--r-md); }
  .acct:only-child { border-radius: var(--r-md); }
  .a-top { display: flex; align-items: center; gap: 8px; min-height: 26px; min-width: 0; }
  .grip { display: grid; place-items: center; width: 20px; height: 26px; border-radius: var(--r-sm); color: var(--ink-4); cursor: grab; touch-action: none; flex-shrink: 0; }
  .grip:hover { color: var(--ink-2); background: var(--surface-2); }
  .dragging .grip { cursor: grabbing; }
  .a-label { width: 9ch; min-width: 4ch; max-width: 18ch; field-sizing: content; height: 26px; padding: 0 6px; margin-left: -2px; border: 0; border-radius: var(--r-sm); background: transparent; font-weight: var(--w-semi); font-size: var(--t-md); outline: none; }
  .a-label:hover { background: var(--surface-2); }
  .a-label:focus { background: var(--surface-2); box-shadow: inset 0 0 0 1px var(--focus); }
  .prim { color: var(--ink-3); }
  .grow { flex: 1; }
  .a-state { display: inline-flex; align-items: center; gap: 6px; font-size: var(--t-sm); color: var(--ink-3); white-space: nowrap; }
  .a-top .a-state { font-size: var(--t-md); color: var(--ink-2); }
  .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--ink-4); }
  .a-state.ok .dot { background: var(--ok); }
  .a-state.warn { color: var(--amber-ink); }
  .a-state.warn .dot { background: var(--amber); }
  .ident { font-size: var(--t-sm); color: var(--ink-3); }
  .a-meta { display: flex; align-items: center; flex-wrap: wrap; gap: 0 6px; font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); min-width: 0; }
  .sep { color: var(--ink-4); }
  .home { font-family: var(--font-mono); font-size: 11px; color: var(--ink-3); }
  .fix { display: flex; flex-direction: column; gap: 6px; padding: 10px 12px; border-radius: var(--r-md); background: var(--amber-soft); }
  .f-text { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--amber-ink); }
  .cmd { display: flex; align-items: center; gap: 8px; }
  .cmd code { flex: 1; min-width: 0; padding: 6px 9px; border-radius: var(--r-sm); background: var(--code-bg); font-family: var(--font-mono); font-size: 11.5px; line-height: 16px; color: var(--ink-1); overflow-wrap: anywhere; }
  .f-hint { font-size: var(--t-xs); color: var(--ink-3); }
  .a-actions { display: inline-flex; align-items: center; gap: 2px; margin: -4px -4px -4px 0; opacity: 0; transition: opacity var(--d-2) var(--ease-out); }
  .acct:hover .a-actions, .acct:focus-within .a-actions { opacity: 1; }
  .rm { color: var(--ink-3); }
  .rm:hover { color: var(--bad); background: var(--bad-soft); }
  .link { color: var(--ink-1); font-weight: var(--w-med); text-decoration: underline; text-decoration-color: var(--line-3); text-underline-offset: 3px; }
  .link:hover { text-decoration-color: var(--ink-2); }
  .add { display: flex; flex-direction: column; gap: 10px; padding: 14px; border-radius: var(--r-md); background: var(--surface-1); box-shadow: inset 0 0 0 1px var(--line-2); }
  .field { display: flex; flex-direction: column; gap: 5px; }
  .f-label { font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-2); }
  .f-row { display: flex; gap: 8px; align-items: center; }
  .f-row .input { flex: 1; }
  .mono { font-family: var(--font-mono); font-size: 12px; }
  .err { font-size: var(--t-sm); color: var(--bad); }
  .add-actions { display: flex; gap: 6px; }
  .choices { display: flex; flex-wrap: wrap; gap: 6px; }
  .choice { display: inline-flex; align-items: center; gap: 8px; height: 30px; padding: 0 12px 0 10px; border-radius: var(--r-full); background: var(--surface-2); font-weight: var(--w-med); font-size: var(--t-sm); }
  .choice:hover { background: var(--surface-3); }
  .choice.on { background: var(--surface-3); box-shadow: inset 0 0 0 1.5px var(--ink-2); }
  .radio { width: 12px; height: 12px; border-radius: 50%; box-shadow: inset 0 0 0 1.5px var(--line-3); flex-shrink: 0; }
  .on .radio { box-shadow: inset 0 0 0 4px var(--ink-1); }
  .row { display: flex; align-items: center; gap: 16px; min-height: 42px; }
  .r-text { flex: 1; display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .r-title { font-weight: var(--w-med); }
  .row select.input { width: 190px; flex-shrink: 0; }
  .stepper { display: inline-flex; align-items: center; gap: 4px; padding: 2px; border-radius: var(--r-full); background: var(--surface-1); }
  .stepper .num { min-width: 22px; text-align: center; font-weight: var(--w-semi); }
</style>
