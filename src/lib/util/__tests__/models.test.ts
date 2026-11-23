import { describe, expect, it } from 'vitest';
import type { ModelCatalog } from '../../gen/ModelCatalog';
import type { ModelOption } from '../../gen/ModelOption';
import { defaultEffortLabel, effortLabel, effortOptions, effortsFor, keptEffort, listTrouble, optionLabel, optionOf, sourceNote, troubleNote } from '../models';

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

  it('offers every level the CLI’s list names, weakest first, for a name the list does not have', () => {
    expect(effortsFor(codex, 'codex', 'my-own-model')).toEqual(['minimal', 'low', 'medium', 'high', 'xhigh']);
    // No default model in the list (Codly’s built-in Codex names): the same for "no model chosen".
    const builtIn: ModelCatalog = { source: 'built_in', problem: 'Codex is not signed in', models: [model('gpt-6-sol', ['low', 'high'])] };
    expect(effortsFor(builtIn, 'codex', '')).toEqual(['low', 'high']);
  });

  it('offers no level for a model that the CLI’s own list names without one', () => {
    const mixed: ModelCatalog = { source: 'cli', problem: null, models: [model('gpt-6-nova', []), model('gpt-6-luna', ['low', 'high'])] };
    expect(effortsFor(mixed, 'codex', 'gpt-6-nova')).toEqual([]);
    // A name the list does not have was typed by hand: nothing says what it takes, so every level of the list.
    expect(effortsFor(mixed, 'codex', 'typed-by-hand')).toEqual(['low', 'high']);
    // The same model in Codly's built-in list: Codly did not ask the CLI, so the list's levels stay on offer.
    expect(effortsFor({ ...mixed, source: 'built_in', problem: 'Codex is not signed in' }, 'codex', 'gpt-6-nova')).toEqual(['low', 'high']);
  });

  it('offers a level once when the CLI names it twice', () => {
    const twice: ModelCatalog = { source: 'cli', problem: null, models: [model('gpt-7', ['low', 'high', 'low'])] };
    expect(effortsFor(twice, 'codex', 'gpt-7')).toEqual(['low', 'high']);
  });

  it('uses Codly’s table only for a CLI that lists no levels: Claude Code', () => {
    expect(effortsFor(null, 'claude', 'opus')).toEqual(['low', 'medium', 'high', 'xhigh', 'max']);
    expect(effortsFor({ source: 'built_in', problem: null, models: [model('opus', [])] }, 'claude', 'opus')).toContain('max');
    expect(effortsFor(null, 'gemini', '')).toEqual([]);
    // Codex has no table: before its list is read, no level is offered.
    expect(effortsFor(null, 'codex', 'gpt-6.1-sol')).toEqual([]);
  });

  it('leaves out a level that the "model:effort" format cannot carry', () => {
    const odd: ModelCatalog = { source: 'cli', problem: null, models: [model('gpt-7', ['low', 'turbo', 'high'])] };
    expect(effortsFor(odd, 'codex', 'gpt-7')).toEqual(['low', 'high']);
    expect(effortsFor(odd, 'codex', 'typed-by-hand')).toEqual(['low', 'high']);
  });
});

describe('keptEffort', () => {
  it('keeps an effort the new model takes and drops one it does not', () => {
    expect(keptEffort(codex, 'codex', 'gpt-6-luna', 'low')).toBe('low');
    expect(keptEffort(codex, 'codex', 'gpt-6-luna', 'xhigh')).toBe('');
    expect(keptEffort(codex, 'codex', 'gpt-6.1-sol', '')).toBe('');
  });

  it('keeps the effort while nothing says which levels the model takes', () => {
    // The Codex list is not read yet, or could not be read.
    expect(keptEffort(null, 'codex', 'gpt-6-luna', 'xhigh')).toBe('xhigh');
    // Claude Code's levels are known without a list.
    expect(keptEffort(null, 'claude', 'opus', 'minimal')).toBe('');
    // A list that names no level at all: the CLI takes none.
    expect(keptEffort({ source: 'cli', problem: null, models: [model('big-pickle', [])] }, 'opencode', 'big-pickle', 'high')).toBe('');
  });

  it('drops the effort for a model that the CLI’s own list names without a level', () => {
    const mixed: ModelCatalog = { source: 'cli', problem: null, models: [model('gpt-6-nova', []), model('gpt-6-luna', ['low', 'high'])] };
    expect(keptEffort(mixed, 'codex', 'gpt-6-nova', 'high')).toBe('');
    // A name typed by hand keeps a level the list names.
    expect(keptEffort(mixed, 'codex', 'typed-by-hand', 'high')).toBe('high');
  });
});

describe('effortOptions', () => {
  it('lists the levels the chosen model takes', () => {
    expect(effortOptions(codex, 'codex', 'gpt-6-luna', 'low')).toEqual([
      { id: 'minimal', label: 'Minimal' },
      { id: 'low', label: 'Low' },
    ]);
    expect(effortOptions(codex, 'codex', '', '').map((o) => o.id)).toEqual(['low', 'medium', 'high', 'xhigh']);
  });

  it('keeps a saved level that the model does not list in view, and says so', () => {
    expect(effortOptions(codex, 'codex', 'gpt-6-luna', 'xhigh')).toEqual([
      { id: 'minimal', label: 'Minimal' },
      { id: 'low', label: 'Low' },
      { id: 'xhigh', label: 'Extra high (not listed for this model)' },
    ]);
  });

  it('shows only the saved level, marked, for a model that the CLI’s own list names without a level', () => {
    const mixed: ModelCatalog = { source: 'cli', problem: null, models: [model('gpt-6-nova', []), model('gpt-6-luna', ['low', 'high'])] };
    expect(effortOptions(mixed, 'codex', 'gpt-6-nova', 'high')).toEqual([{ id: 'high', label: 'High (not listed for this model)' }]);
    expect(effortOptions(mixed, 'codex', 'gpt-6-nova', '')).toEqual([]);
  });

  it('shows a saved level as it is while the list is not read', () => {
    expect(effortOptions(null, 'codex', 'gpt-6-luna', 'high')).toEqual([{ id: 'high', label: 'High' }]);
    expect(effortOptions(null, 'codex', 'gpt-6-luna', '')).toEqual([]);
  });
});

describe('labels', () => {
  it('marks the CLI’s default model', () => {
    expect(codex.models.map(optionLabel)).toEqual(['GPT-6.1 Sol (default)', 'gpt-6-luna']);
    expect([effortLabel('xhigh'), effortLabel('minimal')]).toEqual(['Extra high', 'Minimal']);
  });

  it('says which effort a model runs at when none is chosen, if the CLI says it', () => {
    expect(defaultEffortLabel(codex, 'gpt-6.1-sol')).toBe('Default (Medium)');
    expect(defaultEffortLabel(codex, '')).toBe('Default (Medium)');
    expect(defaultEffortLabel(codex, 'gpt-6-luna')).toBe('Default');
    expect(defaultEffortLabel(null, 'opus')).toBe('Default');
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

describe('a model list that could not be read', () => {
  const fallback: ModelCatalog = { source: 'built_in', problem: 'Codex is not signed in', models: [model('gpt-6-sol', ['low'])] };

  it('is said with the reason: the read failed, or the CLI could not be asked', () => {
    expect(listTrouble(null, 'opencode is not installed')).toBe('opencode is not installed');
    expect(listTrouble(fallback, null)).toBe('Codex is not signed in');
    expect(troubleNote(null, 'opencode is not installed', 'OpenCode')).toBe('Couldn’t load OpenCode’s model list (opencode is not installed). You can still type a name.');
    expect(troubleNote(fallback, null, 'Codex')).toBe(sourceNote(fallback, 'Codex'));
    // A read that fails after one that worked: the failure is what is said.
    expect(listTrouble(codex, 'Codex did not answer within 15 s')).toBe('Codex did not answer within 15 s');
  });

  it('is not said while the list loads or when the CLI listed its models', () => {
    expect([listTrouble(null, null), listTrouble(codex, null)]).toEqual([null, null]);
    expect([troubleNote(null, null, 'Codex'), troubleNote(codex, null, 'Codex')]).toEqual([null, null]);
    expect(troubleNote({ source: 'built_in', problem: null, models: [model('opus', [])] }, null, 'Claude Code')).toBeNull();
  });
});
