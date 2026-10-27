<!--
  Settings → Inbound: ways for something outside Codly to hand the Lead work. Webhooks (a local
  listener, one secret per endpoint, each created switched off, each revocable) and Slack (the
  settings are kept; the connection is not built yet, and this says so). Also: who answers
  inbound messages. A secret is shown once, right after creating its endpoint, and never again.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { InboundState } from '../lib/gen/InboundState';
  import type { NewWebhook } from '../lib/gen/NewWebhook';
  import { api } from '../lib/api';
  import { copyText } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import { ago } from '../lib/util/inbound';
  import Icon from '../components/Icon.svelte';

  let st = $state<InboundState | null>(null);
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
  const load = () => api.inboundState().then((s) => (st = s)).catch(() => {});
  onMount(() => {
    void load();
    const t = setInterval(load, 5000); // counts and the delivery log
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
  async function toggle(id: string, on: boolean) {
    const s = await run(on ? 'switch it on' : 'switch it off', () => api.inboundSetWebhook(id, on));
    if (s) st = s;
    else await load(); // the switch snaps back
  }
  async function revoke(id: string) {
    if (confirming !== id) {
      confirming = id;
      return;
    }
    confirming = null;
    const s = await run('revoke the endpoint', () => api.inboundRevokeWebhook(id));
    if (s) {
      st = s;
      if (fresh?.endpoint.id === id) fresh = null;
    }
  }
  async function setAnswerer(answerer: string) {
    const s = st && (await run('save', () => api.inboundSetSettings(answerer, st!.port)));
    if (s) st = s;
  }
  async function setPort(v: string) {
    const s = st && (await run('save the port', () => api.inboundSetSettings(st!.answerer, Number(v) || 0)));
    if (s) st = s;
    else await load();
  }

  const curl = (w: NewWebhook) =>
    `curl -X POST ${w.endpoint.url} \\\n  -H "Authorization: Bearer ${w.secret}" \\\n  -H "Content-Type: application/json" \\\n  -d '{"text": "Hello from a webhook"}'`;

  // ── Slack ──
  let botToken = $state('');
  let appToken = $state('');
  let users = $state('');
  let channels = $state('');
  let slackSeen = $state(false);
  $effect(() => {
    if (st && !slackSeen) {
      slackSeen = true;
      users = st.slack.config.user_ids.join(', ');
      channels = st.slack.config.channel_ids.join(', ');
    }
  });
  const list = (s: string) => s.split(/[\s,]+/).filter(Boolean);
  async function saveSlack(e: Event) {
    e.preventDefault();
    if (!st) return;
    const s = await run('save the Slack settings', () =>
      api.inboundSetSlack({
        config: { enabled: false, user_ids: list(users), channel_ids: list(channels) },
        bot_token: botToken.trim() || null,
        app_token: appToken.trim() || null,
      }),
    );
    if (!s) return;
    st = s;
    botToken = appToken = '';
    toast({ kind: 'success', title: 'Slack settings saved', body: 'Tokens are in your Keychain. Slack is not connected yet.' });
  }
  async function clearToken(which: 'bot' | 'app') {
    const s = await run('remove the token', () =>
      api.inboundSetSlack({ config: st!.slack.config, bot_token: which === 'bot' ? '' : null, app_token: which === 'app' ? '' : null }),
    );
    if (s) st = s;
  }
</script>

<section>
  <h3 class="eyebrow">Webhooks</h3>
  <p class="desc">
    Anyone holding an endpoint’s URL and secret can post a message to the Lead. Each endpoint has its own secret, starts switched off, and can be revoked on its own.
    The listener only accepts connections from this Mac (127.0.0.1) and only runs while an endpoint is on; to reach it from elsewhere, run a tunnel yourself.
    Messages reach the Lead marked as outside input.
  </p>

  {#if st}
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
              <input type="checkbox" class="switch" checked={e.enabled} disabled={busy || e.broken} aria-label={`${e.name} on`} onchange={(ev) => toggle(e.id, ev.currentTarget.checked)} />
            </div>
            <div class="e-url">
              <code class="mono small">{e.url}</code>
              <button class="icon-btn sm" aria-label="Copy URL" onclick={() => copyText(e.url, 'URL copied')}><Icon name="copy" size={13} /></button>
            </div>
            <span class="muted small">
              {e.enabled ? 'On' : 'Off'} · {e.received} received{e.last_received_at ? `, last ${ago(e.last_received_at)}` : ''}
            </span>
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
        <span class="desc">The Lead, unless you pick one agent to take them all.</span>
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
          {#each st.deliveries.slice(0, 6) as d (d.at + d.endpoint + d.outcome)}
            <li><span class="muted small">{ago(d.at)}</span><span>{d.endpoint}</span><span class="small" class:warn={d.outcome !== 'accepted'}>{d.outcome}</span></li>
          {/each}
        </ul>
      </div>
    {/if}
  {:else}
    <p class="desc">Loading…</p>
  {/if}
</section>

<section>
  <h3 class="eyebrow">Slack <span class="badge">Not connected yet</span></h3>
  <p class="desc">
    Slack through Socket Mode with your own Slack app, no public URL needed. Your settings and tokens are saved now (tokens go to your Keychain),
    but this version cannot open the connection, so nothing arrives from Slack yet. Only the people listed below will be heard once it can.
  </p>
  <form class="slack" onsubmit={saveSlack}>
    <label class="field">
      <span class="f-label">Bot token {#if st?.slack.bot_token_set}<span class="muted">saved</span> <button type="button" class="link" onclick={() => clearToken('bot')}>remove</button>{/if}</span>
      <input class="input mono" type="password" bind:value={botToken} placeholder={st?.slack.bot_token_set ? '••••••••' : 'xoxb-…'} autocomplete="off" spellcheck="false" />
    </label>
    <label class="field">
      <span class="f-label">App-level token {#if st?.slack.app_token_set}<span class="muted">saved</span> <button type="button" class="link" onclick={() => clearToken('app')}>remove</button>{/if}</span>
      <input class="input mono" type="password" bind:value={appToken} placeholder={st?.slack.app_token_set ? '••••••••' : 'xapp-…'} autocomplete="off" spellcheck="false" />
    </label>
    <label class="field">
      <span class="f-label">Allowed user IDs <span class="muted">(nobody is allowed until you list someone)</span></span>
      <input class="input mono" bind:value={users} placeholder="U0123ABCD, U0456EFGH" spellcheck="false" autocomplete="off" />
    </label>
    <label class="field">
      <span class="f-label">Channel IDs <span class="muted">(empty: any channel the bot is in)</span></span>
      <input class="input mono" bind:value={channels} placeholder="C0123ABCD" spellcheck="false" autocomplete="off" />
    </label>
    <button class="btn sm end" type="submit" disabled={busy || !st}>Save Slack settings</button>
  </form>
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
  .slack { display: flex; flex-direction: column; gap: 10px; }
  .field { display: flex; flex-direction: column; gap: 5px; min-width: 0; }
  .f-label { font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-2); }
  .link { font-size: var(--t-sm); color: var(--ink-3); text-decoration: underline; }
  .link:hover { color: var(--bad); }
</style>
