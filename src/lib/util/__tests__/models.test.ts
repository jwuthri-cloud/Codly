import { describe, expect, it } from 'vitest';
import type { ModelCatalog } from '../../gen/ModelCatalog';
import type { ModelOption } from '../../gen/ModelOption';
import { effortLabel, effortsFor, keptEffort, optionLabel, optionOf, sourceNote } from '../models';

const model = (id: string, efforts: string[], more: Partial<ModelOption> = {}): ModelOption => ({ id, name: id, is_default: false, efforts, default_effort: null, ...more });

const codex: ModelCatalog = {
  source: 'cli',
  problem: null,
  models: [model('gpt-6.1-sol', ['low', 'medium', 'high', 'xhigh'], { name: 'GPT-6.1 Sol', is_default: true, default_effort: 'medium' }), model('gpt-6-luna', ['minimal', 'low'])],
};

describe('effortsFor', () => {
  it('offers the efforts the CLI lists for that model', () => {
    expect(effortsFor(codex, 'codex', 'gpt-6-luna')).toEqual(['minimal', 'low']);
  });

  it('uses the default model’s efforts when no model is chosen', () => {
    expect(optionOf(codex, '')?.id).toBe('gpt-6.1-sol');
    expect(effortsFor(codex, 'codex', '')).toEqual(['low', 'medium', 'high', 'xhigh']);
  });

  it('falls back to the CLI’s usual levels for a name the catalog does not have', () => {
    expect(effortsFor(codex, 'codex', 'my-own-model')).toEqual(['low', 'medium', 'high', 'xhigh']);
    expect(effortsFor(null, 'claude', 'opus')).toEqual(['low', 'medium', 'high', 'xhigh', 'max']);
    expect(effortsFor({ source: 'built_in', problem: null, models: [model('opus', [])] }, 'claude', 'opus')).toContain('max');
    expect(effortsFor(null, 'gemini', '')).toEqual([]);
  });
});

describe('keptEffort', () => {
  it('keeps an effort the new model takes and drops one it does not', () => {
    expect(keptEffort(codex, 'codex', 'gpt-6-luna', 'low')).toBe('low');
    expect(keptEffort(codex, 'codex', 'gpt-6-luna', 'xhigh')).toBe('');
    expect(keptEffort(codex, 'codex', 'gpt-6.1-sol', '')).toBe('');
  });
});

describe('labels', () => {
  it('marks the CLI’s default model', () => {
    expect(codex.models.map(optionLabel)).toEqual(['GPT-6.1 Sol (default)', 'gpt-6-luna']);
    expect([effortLabel('xhigh'), effortLabel('minimal')]).toEqual(['Extra high', 'Minimal']);
  });
});

describe('sourceNote', () => {
  it('says the CLI listed the models', () => {
    expect(sourceNote(codex, 'Codex')).toBe('Codex’s own list, read just now.');
  });

  it('says when the built-in list is shown, and why', () => {
    const fallback: ModelCatalog = { source: 'built_in', problem: 'Codex is not signed in', models: [model('gpt-6-sol', ['low'])] };
    expect(sourceNote(fallback, 'Codex')).toBe('Couldn’t ask Codex for its models (Codex is not signed in). This is Codly’s built-in list; you can type any other name.');
    expect(sourceNote({ source: 'built_in', problem: null, models: [model('opus', [])] }, 'Claude Code')).toBe('Codly’s built-in names for Claude Code; you can type any other name.');
  });

  it('says nothing when nothing is listed', () => {
    expect(sourceNote(null, 'Codex')).toBeNull();
    expect(sourceNote({ source: 'built_in', problem: null, models: [] }, 'Gemini CLI')).toBeNull();
    expect(sourceNote({ source: 'cli', problem: null, models: [] }, 'OpenCode')).toBeNull();
  });
});
