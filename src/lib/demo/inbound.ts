// Mock of the inbound hub (webhooks, Slack) for `pnpm dev`: the same rules as the Rust core
// (new endpoints are off and do not run plans without asking, the listener is "on" only while one
// is on, the secret is returned once and never kept), with a fake delivery log. No real server
// runs in the browser.
import type { InboundState } from '../gen/InboundState';
import type { SlackUpdate } from '../gen/SlackUpdate';
import type { WebhookEndpoint } from '../gen/WebhookEndpoint';

const state: InboundState = {
  answerer: 'lead',
  port: 47821,
  listening: false,
  listen_error: null,
  endpoints: [],
  slack: {
    config: { enabled: false, user_ids: [], channel_ids: [] },
    bot_token_set: false,
    app_token_set: false,
    connected: false,
    status: 'Not available yet: this version cannot connect to Slack, so it takes no Slack tokens.',
  },
  deliveries: [],
};

const hex = (n: number) => Array.from({ length: n }, () => Math.floor(Math.random() * 16).toString(16)).join('');
const url = (id: string) => `http://127.0.0.1:${state.port}/hook/${id}`;

function refresh(): InboundState {
  state.listening = state.endpoints.some((e) => e.enabled);
  for (const e of state.endpoints) e.url = url(e.id);
  return structuredClone(state);
}

export function inboundMock(cmd: string, a: Record<string, unknown>): unknown {
  const find = () => {
    const e = state.endpoints.find((x) => x.id === a.id);
    if (!e) throw new Error('unknown endpoint');
    return e;
  };
  switch (cmd) {
    case 'inbound_state':
      return refresh();
    case 'inbound_create_webhook': {
      const name = String(a.name ?? '').trim();
      if (!name || name.length > 60) throw new Error('give the endpoint a short name (1 to 60 characters)');
      const id = `wh_${hex(12)}`;
      const e: WebhookEndpoint = { id, name, enabled: false, created_at: Date.now(), url: url(id), received: 0, last_received_at: null, broken: false, auto_run: false };
      state.endpoints.push(e);
      return { endpoint: { ...e }, secret: `cwh_${hex(64)}` };
    }
    case 'inbound_set_webhook': {
      const e = find();
      e.auto_run = !!a.autoRun;
      e.enabled = !!a.enabled;
      if (e.enabled && !state.deliveries.length) {
        // Something to look at: a message that came in just now.
        e.received = 1;
        e.last_received_at = Date.now();
        state.deliveries.unshift({ at: Date.now(), endpoint: e.name, outcome: 'accepted', bytes: 118 });
      }
      return refresh();
    }
    case 'inbound_revoke_webhook':
      find();
      state.endpoints = state.endpoints.filter((x) => x.id !== a.id);
      return refresh();
    case 'inbound_set_settings': {
      const port = Number(a.port);
      if (port !== 0 && port < 1024) throw new Error('pick a port from 1024 up');
      state.answerer = String(a.answerer);
      state.port = port || 47821;
      return refresh();
    }
    case 'inbound_set_slack': {
      const u = a.update as SlackUpdate;
      // As the core: no new token while Slack cannot connect; '' removes a saved one.
      if (u.bot_token || u.app_token) throw new Error('Slack is not available yet, so Codly does not take Slack tokens');
      if (u.bot_token !== null) state.slack.bot_token_set = false;
      if (u.app_token !== null) state.slack.app_token_set = false;
      state.slack.config = u.config;
      return refresh();
    }
  }
  throw new Error(`unknown inbound command ${cmd}`);
}
