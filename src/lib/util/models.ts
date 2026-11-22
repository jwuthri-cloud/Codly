// Model pickers (hire wizard, model dialog): what a CLI's catalog (`api.modelCatalog`) offers
// for one model, and the line that says where the list comes from.

import type { ModelCatalog } from '../gen/ModelCatalog';
import type { ModelOption } from '../gen/ModelOption';
import type { Provider } from '../gen/Provider';
import { EFFORTS } from './team';

/** The catalog's entry for a model name ('' = the model the CLI runs when none is chosen). */
export function optionOf(catalog: ModelCatalog | null, name: string): ModelOption | null {
  return catalog?.models.find((m) => (name ? m.id === name : m.is_default)) ?? null;
}

/** The thinking efforts to offer for `name`: the model's own when the CLI lists them, else the
 *  CLI's usual levels (a name typed by hand, a CLI that does not say). */
export function effortsFor(catalog: ModelCatalog | null, provider: Provider, name: string): string[] {
  const own = optionOf(catalog, name)?.efforts ?? [];
  return own.length ? own : (EFFORTS[provider] ?? []);
}

/** `effort` if `name` takes it, else '' (the model's default): a level one model has, another may not. */
export function keptEffort(catalog: ModelCatalog | null, provider: Provider, name: string, effort: string): string {
  return effortsFor(catalog, provider, name).includes(effort) ? effort : '';
}

export const effortLabel = (e: string) => (e === 'xhigh' ? 'Extra high' : e[0].toUpperCase() + e.slice(1));

/** The picker's label of a model: the CLI's name for it, marked when it is the CLI's default. */
export function optionLabel(m: ModelOption): string {
  return m.is_default ? `${m.name} (default)` : m.name;
}

/** Where a picker's list comes from, in one line. Null: nothing is listed. */
export function sourceNote(catalog: ModelCatalog | null, cli: string): string | null {
  if (!catalog) return null;
  if (catalog.source === 'cli') return catalog.models.length ? `${cli}’s own list, read just now.` : null;
  if (catalog.problem) return `Couldn’t ask ${cli} for its models (${catalog.problem}). This is Codly’s built-in list; you can type any other name.`;
  return catalog.models.length ? `Codly’s built-in names for ${cli}; you can type any other name.` : null;
}
