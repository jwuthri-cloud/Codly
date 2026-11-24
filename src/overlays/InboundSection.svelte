<!--
  Settings → Inbound: ways for something outside Codly to hand the Lead work. Webhooks (a local
  listener, one secret per endpoint, each created switched off, each revocable). Slack is not
  available yet: the section says so and takes no token; tokens that an earlier version saved can
  be removed here. Also: who answers inbound messages. A secret is shown once, right after
  creating its endpoint, and never again.
  A plan the Lead makes from an inbound message waits for Run plan, unless you let that endpoint
  run plans without asking: the switch says what that hands over. An agent picked to answer
  instead of the Lead makes no plan, so nothing waits: its row says so.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { InboundState } from '../lib/gen/InboundState';
  import type { NewWebhook } from '../lib/gen/NewWebhook';
  import type { WebhookEndpoint } from '../lib/gen/WebhookEndpoint';
  import { api } from '../lib/api';
  import { copyText } from '../lib/state/actions';
  import { Loader } from '../lib/state/loader.svelte';
  import { store } from '../lib/state/store.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import { ago } from '../lib/util/inbound';
  import Icon from '../components/Icon.svelte';
  import LoadError from '../components/LoadError.svelte';

  const inbound = new Loader<InboundState>(() => api.inboundState());
  const st = $derived(inbound.value);
  let fresh = $state<NewWebhook | null>(null); // the secret on screen, once
  let name = $state('');
  let confirming = $state<string | null>(null);
  let busy = $state(false);

  /** Run a call; show its failure as a toast with the core's own explanation. */
  async function run<T>(what: string, f: () => Promise<T>): Promise<T | null> {
    busy = true;
    try {
      return await f();
    } catch (e) {
      toast({ kind: 'error', title: `Couldn’t ${what}`, body: String(e instanceof Error ? e.message : e) });
      return null;
    } finally {
      busy = false;
    }
  }
  const load = () => inbound.load();
  onMount(() => {
    void load();
    const t = setInterval(() => store.offline || void load(), 5000); // counts and the delivery log
    return () => clearInterval(t);
  });

  async function create(e: Event) {
    e.preventDefault();
    const n = name.trim();
    if (!n) return;
    const w = await run('create the endpoint', () => api.inboundCreateWebhook(n));
    if (!w) return;
    fresh = w;
    name = '';
    await load();
  }
  async function toggle(e: WebhookEndpoint, on: boolean) {
    const s = await run(on ? 'switch it on' : 'switch it off', () => api.inboundSetWebhook(e.id, on, e.auto_run));
    if (s) inbound.set(s);
    else await load(); // the switch snaps back
  }
  async function setAutoRun(e: WebhookEndpoint, on: boolean) {
    const s = await run('save', () => api.inboundSetWebhook(e.id, e.enabled, on));
    if (s) inbound.set(s);
    else await load(); // the box snaps back
  }
  async function revoke(id: string) {
    if (confirming !== id) {
      confirming = id;
      return;
    }
    confirming = null;
    const s = await run('revoke the endpoint', () => api.inboundRevokeWebhook(id));
    if (s) {
      inbound.set(s);
      if (fresh?.endpoint.id === id) fresh = null;
    }
  }
  async function setAnswerer(answerer: string) {
    const s = st && (await run('save', () => api.inboundSetSettings(answerer, st!.port)));
    if (s) inbound.set(s);
  }
  async function setPort(v: string) {
    const s = st && (await run('save the port', () => api.inboundSetSettings(st!.answerer, Number(v) || 0)));
    if (s) inbound.set(s);
    else await load();
  }

  const curl = (w: NewWebhook) =>
    `curl -X POST ${w.endpoint.url} \\\n  -H "Authorization: Bearer ${w.secret}" \\\n  -H "Content-Type: application/json" \\\n  -d '{"text": "Hello from a webhook"}'`;

  // ── Slack: not available yet. Nothing is collected; tokens from an earlier version can go. ──
  const slackTokensSaved = $derived(!!st && (st.slack.bot_token_set || st.slack.app_token_set));
  async function removeSlackTokens() {
    const s = await run('remove the Slack tokens', () => api.inboundSetSlack({ config: st!.slack.config, bot_token: '', app_token: '' }));
    if (!s) return;
    inbound.set(s);
    toast({ kind: 'success', title: 'Slack tokens removed', body: 'They are no longer in your Keychain.' });
  }
</script>

<section>
  <h3 class="eyebrow">Webhooks</h3>
  <p class="desc">
    Anyone holding an endpoint’s URL and secret can post a message to the Lead. Each endpoint has its own secret, starts switched off, and can be revoked on its own.
    The listener only accepts connections from this Mac (127.0.0.1) and only runs while an endpoint is on; to reach it from elsewhere, run a tunnel yourself.
    A message shows in the thread under its endpoint’s name, and the Lead reads it as outside input, not as your words.
    A plan the Lead makes from it waits for you to click Run plan, unless you let that endpoint run plans without asking.
  </p>

  {#if st}
    {#if inbound.fails >= 2}<p class="desc warn" role="status">Not updating: {inbound.error}</p>{/if}
    <div class="status" class:bad={!!st.listen_error}>
      <span class="dot" class:live={st.listening}></span>
      {#if st.listen_error}
        {st.listen_error}
      {:else if st.listening}
        Listening on 127.0.0.1:{st.port}
      {:else}
        Not listening: nothing is switched on.
      {/if}
    </div>

    {#if fresh}
      <div class="secret" role="status">
        <div class="s-head">
          <span><strong>{fresh.endpoint.name}</strong> is ready, and switched off.</span>
          <span class="warn">Copy the secret now: it is not shown again.</span>
        </div>
        <div class="s-row">
          <code class="mono">{fresh.secret}</code>
          <button class="btn sm" onclick={() => copyText(fresh!.secret, 'Secret copied')}><Icon name="copy" size={12} />Copy</button>
        </div>
        <details>
          <summary class="small">Example request</summary>
          <pre class="mono small">{curl(fresh)}</pre>
          <button class="btn sm ghost" onclick={() => copyText(curl(fresh!), 'Command copied')}><Icon name="copy" size={12} />Copy command</button>
        </details>
        <button class="btn sm ghost end" onclick={() => (fresh = null)}>I saved it</button>
      </div>
    {/if}

    {#if st.endpoints.length}
      <ul class="eps">
        {#each st.endpoints as e (e.id)}
          <li class:off={!e.enabled}>
            <div class="e-top">
              <span class="e-name">{e.name}</span>
              {#if e.broken}<span class="badge bad">secret missing</span>{/if}
              <span class="grow"></span>
              {#if confirming === e.id}
                <span class="small warn">Its URL stops working at once.</span>
                <button class="btn sm danger" onclick={() => revoke(e.id)}>Revoke</button>
                <button class="btn sm ghost" onclick={() => (confirming = null)}>Keep</button>
              {:else}
                <button class="btn sm ghost rm" onclick={() => revoke(e.id)}><Icon name="trash" size={13} />Revoke</button>
              {/if}
              <input type="checkbox" class="switch" checked={e.enabled} disabled={busy || e.broken} aria-label={`${e.name} on`} onchange={(ev) => toggle(e, ev.currentTarget.checked)} />
            </div>
            <div class="e-url">
              <code class="mono small">{e.url}</code>
              <button class="icon-btn sm" aria-label="Copy URL" onclick={() => copyText(e.url, 'URL copied')}><Icon name="copy" size={13} /></button>
            </div>
            <span class="muted small">
              {e.enabled ? 'On' : 'Off'} · {e.received} received{e.last_received_at ? `, last ${ago(e.last_received_at)}` : ''}
            </span>
            <label class="e-auto small">
              <input type="checkbox" checked={e.auto_run} disabled={busy || e.broken} onchange={(ev) => setAutoRun(e, ev.currentTarget.checked)} />
              <span>
                Run plans without asking.
                {#if e.auto_run}
                  <span class="warn">In an Auto or Direct request, a plan made from this endpoint’s messages starts at once. Anyone who can post here, or who writes what gets posted here, can start work on your Mac.</span>
                {:else}
                  <span class="muted">Off: a plan made from this endpoint’s messages waits for your Run plan click.</span>
                {/if}
              </span>
            </label>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="desc">No endpoints yet.</p>
    {/if}

    <form class="new" onsubmit={create}>
      <input class="input" bind:value={name} maxlength="60" placeholder="Name, e.g. CI alerts" aria-label="New endpoint name" spellcheck="false" autocomplete="off" />
      <button class="btn sm" type="submit" disabled={busy || !name.trim()}><Icon name="plus" size={13} />New endpoint</button>
    </form>

    <div class="row">
      <div class="r-text">
        <span class="r-title">Who answers inbound messages</span>
        <span class="desc">
          The Lead, unless you pick one agent to take them all.
          <span class:warn={st.answerer !== 'lead'}>
            An agent you pick gets each message at once, in its own chat. It is told to report and not to obey, but it makes no plan: nothing waits for
            Run plan, the “Run plans without asking” switches do nothing, and it can act under its own approval mode.
          </span>
        </span>
      </div>
      <select class="input sel" value={st.answerer} aria-label="Who answers inbound messages" onchange={(e) => setAnswerer(e.currentTarget.value)}>
        <option value="lead">The Lead</option>
        {#each store.agents.filter((a) => a.role !== 'lead') as a (a.id)}<option value={`agent:${a.id}`}>{a.name}</option>{/each}
        {#if st.answerer !== 'lead' && !store.agents.some((a) => `agent:${a.id}` === st!.answerer)}<option value={st.answerer}>{st.answerer}</option>{/if}
      </select>
    </div>
    <div class="row">
      <div class="r-text"><span class="r-title">Port</span><span class="desc">On 127.0.0.1. Changing it moves every URL above.</span></div>
      <input class="input num" type="number" min="1024" max="65535" value={st.port} aria-label="Webhook port" onchange={(e) => setPort(e.currentTarget.value)} />
    </div>

    {#if st.deliveries.length}
      <div class="log">
        <span class="f-label">Recent requests</span>
        <ul>
          {#each st.deliveries.slice(0, 6) as d, i (i)}
            <li><span class="muted small">{ago(d.at)}</span><span>{d.endpoint}</span><span class="small" class:warn={d.outcome !== 'accepted'}>{d.outcome}</span></li>
          {/each}
        </ul>
      </div>
    {/if}
  {:else if inbound.error}
    <LoadError what="the endpoints" error={inbound.error} loading={inbound.loading} onretry={load} />
  {:else}
    <p class="desc">Loading…</p>
  {/if}
</section>

<section>
  <h3 class="eyebrow">Slack <span class="badge">Not available yet</span></h3>
  <p class="desc">This version cannot connect to Slack, so nothing arrives from Slack and Codly asks for no Slack token.</p>
  {#if slackTokensSaved}
    <div class="row">
      <div class="r-text">
        <span class="r-title">Slack tokens from an earlier version</span>
        <span class="desc">They are still in your Keychain, and nothing uses them.</span>
      </div>
      <button class="btn sm" onclick={removeSlackTokens} disabled={busy}><Icon name="trash" size={13} />Remove saved Slack tokens</button>
    </div>
  {/if}
</section>

<style>
  section { padding: 20px 0; box-shadow: inset 0 -1px 0 var(--line); display: flex; flex-direction: column; gap: 12px; }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .muted { color: var(--ink-3); font-weight: var(--w-reg); }
  .small { font-size: var(--t-sm); }
  .warn { color: var(--amber-ink); }
  .grow { flex: 1; }
  .end { align-self: flex-end; }
  .mono { font-family: var(--font-mono); }
  .badge { font-size: var(--t-xs); padding: 1px 7px; border-radius: var(--r-full); background: var(--surface-3); color: var(--ink-2); white-space: nowrap; text-transform: none; letter-spacing: 0; margin-left: 6px; }
  .badge.bad { background: var(--bad-soft); color: var(--bad); margin-left: 0; }
  .status { display: flex; align-items: center; gap: 8px; font-size: var(--t-sm); color: var(--ink-2); }
  .status.bad { color: var(--bad); }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--line-3); flex-shrink: 0; }
  .dot.live { background: var(--ok, var(--aurora-m)); }
  .secret { display: flex; flex-direction: column; gap: 8px; padding: 12px 14px; border-radius: var(--r-md); background: var(--surface-2); box-shadow: inset 0 0 0 1.5px var(--ink-3); }
  .s-head { font-size: var(--t-sm); display: flex; flex-direction: column; gap: 2px; }
  .s-row { display: flex; align-items: center; gap: 8px; }
  .s-row code { flex: 1; min-width: 0; overflow-wrap: anywhere; font-size: var(--t-sm); user-select: all; }
  pre { margin: 6px 0; padding: 8px 10px; border-radius: var(--r-sm); background: var(--surface-1); overflow-x: auto; white-space: pre; }
  .eps { display: flex; flex-direction: column; border-radius: var(--r-md); background: var(--surface-1); }
  .eps li { display: flex; flex-direction: column; gap: 6px; padding: 12px 14px; }
  .eps li + li { box-shadow: inset 0 1px 0 var(--line); }
  .eps li.off .e-name, .eps li.off .e-url { opacity: 0.6; }
  .e-top { display: flex; align-items: center; gap: 8px; min-height: 26px; }
  .e-name { font-weight: var(--w-semi); }
  .e-url { display: flex; align-items: center; gap: 4px; min-width: 0; }
  .e-url code { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--ink-2); }
  .e-auto { display: flex; align-items: flex-start; gap: 8px; line-height: var(--lh-sm); color: var(--ink-2); }
  .e-auto input { margin-top: 2px; flex-shrink: 0; }
  .rm { color: var(--ink-3); }
  .rm:hover { color: var(--bad); background: var(--bad-soft); }
  .new { display: flex; gap: 8px; }
  .new .input { flex: 1; min-width: 0; }
  .row { display: flex; align-items: center; gap: 16px; min-height: 42px; }
  .r-text { flex: 1; display: flex; flex-direction: column; gap: 2px; }
  .r-title { font-weight: var(--w-med); }
  .sel { width: 180px; }
  .num { width: 90px; }
  .log { display: flex; flex-direction: column; gap: 4px; }
  .log ul { display: flex; flex-direction: column; border-radius: var(--r-md); background: var(--surface-1); }
  .log li { display: grid; grid-template-columns: 64px 1fr auto; gap: 10px; align-items: center; padding: 6px 14px; }
  .log li + li { box-shadow: inset 0 1px 0 var(--line); }
  .f-label { font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-2); }
</style>
