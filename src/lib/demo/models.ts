// The browser demo's model catalogs (`model_catalog`, `list_models`): what the core reads from
// each CLI. Codex lists its models with the efforts each one takes; Claude has no list, so
// its aliases are Codly's own; the other CLIs list nothing here.
import type { ModelCatalog } from '../gen/ModelCatalog';
import type { ModelOption } from '../gen/ModelOption';
import type { Provider } from '../gen/Provider';

const named = (id: string): ModelOption => ({ id, name: id, is_default: false, efforts: [], default_effort: null });

export function demoCatalog(provider: Provider): ModelCatalog {
  if (provider === 'codex') {
    return {
      source: 'cli',
      problem: null,
      models: [
        { id: 'gpt-6.1-sol', name: 'GPT-6.1 Sol', is_default: true, efforts: ['low', 'medium', 'high', 'xhigh'], default_effort: 'medium' },
        { id: 'gpt-6-luna', name: 'GPT-6 Luna', is_default: false, efforts: ['minimal', 'low', 'medium'], default_effort: 'low' },
      ],
    };
  }
  const models = provider === 'claude' ? ['sonnet', 'opus', 'haiku', 'fable'].map(named) : [];
  return { source: 'built_in', problem: null, models };
}
