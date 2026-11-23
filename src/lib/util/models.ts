// Model pickers (hire wizard, model dialog, the request's menu, Settings → the Lead and Team):
// what a CLI's catalog (`api.modelCatalog`) offers for one model, and the lines that say where
// the list comes from or why it could not be read.

import type { ModelCatalog } from '../gen/ModelCatalog';
import type { ModelOption } from '../gen/ModelOption';
import type { Provider } from '../gen/Provider';
import { EFFORTS, EFFORT_LEVELS } from './team';

/** The catalog's entry for a model name ('' = the model the CLI runs when none is chosen). */
export function optionOf(catalog: ModelCatalog | null, name: string): ModelOption | null {
  return catalog?.models.find((m) => (name ? m.id === name : m.is_default)) ?? null;
}

/** The thinking efforts to offer for `name`: the model's own, in the CLI's order. For a name the
 *  CLI does not list (typed by hand), or one it lists without levels: every level its list
 *  names, weakest first. For a CLI that lists none: Codly's table (`EFFORTS`, Claude Code).
 *  Only levels the "model:effort" format carries: another word would become part of the name. */
export function effortsFor(catalog: ModelCatalog | null, provider: Provider, name: string): string[] {
  const own = (optionOf(catalog, name)?.efforts ?? []).filter((e) => EFFORT_LEVELS.includes(e));
  if (own.length) return own;
  const named = new Set((catalog?.models ?? []).flatMap((m) => m.efforts));
  const listed = EFFORT_LEVELS.filter((e) => named.has(e));
  return listed.length ? listed : (EFFORTS[provider] ?? []);
}

/** `effort` if `name` takes it, else '' (the model's default): a level one model has, another
 *  may not. Kept while nothing says which levels there are (the catalog is not read yet, or
 *  could not be read, and Codly has no table for the CLI). */
export function keptEffort(catalog: ModelCatalog | null, provider: Provider, name: string, effort: string): string {
  const offered = effortsFor(catalog, provider, name);
  if (!catalog && !offered.length) return effort;
  return offered.includes(effort) ? effort : '';
}

export const effortLabel = (e: string) => (e === 'xhigh' ? 'Extra high' : e[0].toUpperCase() + e.slice(1));

/** The choices of an effort picker that saves at once: what `name` takes, then the saved level
 *  when the model does not list it, marked, so a saved level is never hidden and can be changed. */
export function effortOptions(catalog: ModelCatalog | null, provider: Provider, name: string, saved: string): { id: string; label: string }[] {
  const offered = effortsFor(catalog, provider, name);
  const options = offered.map((id) => ({ id, label: effortLabel(id) }));
  if (!saved || offered.includes(saved)) return options;
  // Before the catalog is read nothing says that the model does not list it.
  return [...options, { id: saved, label: catalog ? `${effortLabel(saved)} (not listed for this model)` : effortLabel(saved) }];
}

/** The picker's "no effort chosen" choice, with the level the model then runs at when the CLI says it. */
export function defaultEffortLabel(catalog: ModelCatalog | null, name: string): string {
  const level = optionOf(catalog, name)?.default_effort;
  return level ? `Default (${effortLabel(level)})` : 'Default';
}

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

/** Why a picker does not show the CLI's own list, in the CLI's or Codly's words: the catalog
 *  could not be read at all (`failed`), or the CLI could not be asked and Codly's built-in list
 *  is shown instead. Null: nothing went wrong. */
export function listTrouble(catalog: ModelCatalog | null, failed: string | null): string | null {
  return failed ?? catalog?.problem ?? null;
}

/** That trouble as the line a picker shows with its model field. Null: nothing went wrong. */
export function troubleNote(catalog: ModelCatalog | null, failed: string | null, cli: string): string | null {
  if (failed) return `Couldn’t load ${cli}’s model list (${failed}). You can still type a name.`;
  return catalog?.problem ? sourceNote(catalog, cli) : null;
}
