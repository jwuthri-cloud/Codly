import { describe, expect, it } from 'vitest';
import type { Agent } from '../gen/Agent';
import { contextFraction, costLine, costShort, spentLine } from '../util/cost';

const usage = (cost_usd: number | null, context_used: number | null = null, context_window: number | null = null): Agent['usage'] => ({
  input_tokens: 0,
  output_tokens: 0,
  cache_read_tokens: 0,
  cache_write_tokens: 0,
  context_used,
  context_window,
  cost_usd,
});
const agent = (billing: Agent['billing'], cost: number | null, provider: Agent['provider'] = 'claude') => ({ usage: usage(cost), provider, billing });

describe('an agent’s cost', () => {
  it('says who pays, and marks an estimate', () => {
    expect(costLine(agent('subscription', 0.42))).toBe('≈ $0.42 at list price · included in your plan');
    expect(costLine(agent('api_key', 0.42))).toBe('$0.42 · billed to your API key');
    expect(costLine(agent('third_party', 0.42))).toBe('$0.42 · billed by the provider');
    expect(costLine(agent('unknown', 0.42))).toBe('≈ $0.42');
    expect(costShort(agent('subscription', 0.42))).toBe('≈ $0.42');
    expect(costShort(agent('unknown', 0.42))).toBe('≈ $0.42');
    expect(costShort(agent('api_key', 0.42))).toBe('$0.42');
    expect(costShort(agent('third_party', 12))).toBe('$12.00');
  });

  it('is not shown as $0 when the CLI reports none', () => {
    expect(costLine(agent('subscription', null, 'codex'))).toBe('Tokens only — Codex reports no cost');
    expect(costLine(agent('subscription', null))).toBe('No cost reported');
    expect(costShort(agent('api_key', null))).toBeNull();
  });
});

describe('the context meter', () => {
  it('needs both numbers from the CLI', () => {
    expect(contextFraction(usage(null, 90_000, 200_000))).toBe(0.45);
    expect(contextFraction(usage(null, null, 200_000))).toBeNull();
    expect(contextFraction(usage(null, 90_000, null))).toBeNull();
    expect(contextFraction(usage(null, 0, 200_000))).toBe(0);
  });
});

describe('a total', () => {
  it('shows the cost only when a CLI reported one', () => {
    expect(spentLine({ tokens: 1_240_000, cost_usd: 3.4 })).toBe('1.2M tokens · about $3.40');
    expect(spentLine({ tokens: 38_400, cost_usd: 0 })).toBe('38k tokens');
    expect(spentLine({ tokens: 0, cost_usd: 0 })).toBe('0 tokens');
  });
});
