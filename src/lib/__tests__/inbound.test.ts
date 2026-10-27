import { describe, expect, it } from 'vitest';
import { inboundMock } from '../demo/inbound';
import { ago } from '../util/inbound';
import type { InboundState } from '../gen/InboundState';
import type { NewWebhook } from '../gen/NewWebhook';

describe('ago', () => {
  const now = 1_000_000_000_000;
  it('speaks in the coarsest honest unit', () => {
    expect(ago(now - 5_000, now)).toBe('just now');
    expect(ago(now - 5 * 60_000, now)).toBe('5 min ago');
    expect(ago(now - 3 * 3_600_000, now)).toBe('3 h ago');
    expect(ago(now - 2 * 86_400_000, now)).toBe('2 d ago');
    expect(ago(now + 10_000, now)).toBe('just now');
  });
});

describe('inbound mock follows the core rules', () => {
  it('creates endpoints switched off, listens only while one is on, returns the secret once', () => {
    const w = inboundMock('inbound_create_webhook', { name: ' CI ' }) as NewWebhook;
    expect(w.endpoint.enabled).toBe(false);
    expect(w.secret).toMatch(/^cwh_/);
    expect(w.endpoint.url).not.toContain(w.secret);
    let s = inboundMock('inbound_state', {}) as InboundState;
    expect(s.listening).toBe(false);
    expect(JSON.stringify(s)).not.toContain(w.secret);
    s = inboundMock('inbound_set_webhook', { id: w.endpoint.id, enabled: true }) as InboundState;
    expect(s.listening).toBe(true);
    s = inboundMock('inbound_revoke_webhook', { id: w.endpoint.id }) as InboundState;
    expect(s.listening).toBe(false);
    expect(() => inboundMock('inbound_set_webhook', { id: w.endpoint.id, enabled: true })).toThrow();
  });
});
