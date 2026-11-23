import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { ModelCatalog } from '../gen/ModelCatalog';
import { ModelCatalogs } from '../state/modelCatalogs.svelte';

const codex: ModelCatalog = { source: 'cli', problem: null, models: [{ id: 'gpt-6.1-sol', name: 'GPT-6.1 Sol', is_default: true, efforts: ['low', 'high'], default_effort: 'low' }] };
/** Let the answers of the calls made so far arrive. */
const settle = () => new Promise((r) => setTimeout(r, 0));

describe('a picker’s model catalogs', () => {
  afterEach(() => vi.restoreAllMocks());

  it('reads each CLI’s catalog once', async () => {
    const read = vi.spyOn(api, 'modelCatalog').mockResolvedValue(codex);
    const catalogs = new ModelCatalogs();
    expect(catalogs.of('codex')).toBeNull();
    catalogs.ask('codex');
    catalogs.ask('codex');
    await settle();
    expect(catalogs.of('codex')).toEqual(codex);
    expect(catalogs.failure('codex')).toBeNull();
    expect(read).toHaveBeenCalledTimes(1);
    // Another CLI is another read; the first one's catalog stays.
    catalogs.ask('claude');
    expect(read).toHaveBeenLastCalledWith('claude');
    expect(catalogs.of('codex')).toEqual(codex);
  });

  it('keeps why a read failed, and reads again when asked to', async () => {
    const read = vi.spyOn(api, 'modelCatalog').mockRejectedValueOnce('opencode is not installed').mockResolvedValue(codex);
    const catalogs = new ModelCatalogs();
    catalogs.ask('opencode');
    await settle();
    expect(catalogs.of('opencode')).toBeNull();
    expect(catalogs.failure('opencode')).toBe('opencode is not installed');
    // Asked once already: only "Try again" reads again.
    catalogs.ask('opencode');
    expect(read).toHaveBeenCalledTimes(1);
    catalogs.ask('opencode', true);
    await settle();
    expect(catalogs.of('opencode')).toEqual(codex);
    expect(catalogs.failure('opencode')).toBeNull();
  });
});
