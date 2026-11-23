// The model catalogs of a picker that saves at once (the request's menu under the composer,
// Settings → the Lead, Settings → Team): per CLI, what `api.modelCatalog` answered, or why it
// could not be read. Each picker has its own, so one that opens later asks again and shows a
// sign-in made since. What the pickers offer from a catalog is in ../util/models.ts.
import { api } from '../api';
import type { ModelCatalog } from '../gen/ModelCatalog';
import type { Provider } from '../gen/Provider';

const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));

export class ModelCatalogs {
  // Raw: a catalog can list thousands of models (OpenRouter), and nothing changes one in place.
  private lists = $state.raw<Partial<Record<Provider, ModelCatalog>>>({});
  private failures = $state.raw<Partial<Record<Provider, string>>>({});
  private asked = new Set<Provider>();

  /** The CLI's catalog. Null while it is read, and when it could not be read. */
  of(p: Provider): ModelCatalog | null {
    return this.lists[p] ?? null;
  }

  /** Why the last read of the CLI's catalog failed, until a read answers. Null when none failed. */
  failure(p: Provider): string | null {
    return this.failures[p] ?? null;
  }

  /** Read a CLI's catalog, once. `again`: also when it was asked before ("Try again"). */
  ask(p: Provider, again = false): void {
    if (this.asked.has(p) && !again) return;
    this.asked.add(p);
    api.modelCatalog(p).then(
      (catalog) => {
        this.lists = { ...this.lists, [p]: catalog };
        const { [p]: _gone, ...rest } = this.failures;
        this.failures = rest;
      },
      (e) => {
        this.failures = { ...this.failures, [p]: msg(e) };
      },
    );
  }
}
