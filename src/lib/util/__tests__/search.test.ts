import { describe, expect, it } from 'vitest';
import { searchEnd, searchFailure } from '../search';

describe('searchFailure', () => {
  it('says the index is not ready only when the core says so', () => {
    // The core's error for a repo with no index yet, as the app gets it (a string) and as an Error.
    expect(searchFailure('the code index is not ready; use grep/rg instead')).toBe('The code index is not ready yet.');
    expect(searchFailure(new Error('the code index is not ready; use grep/rg instead'))).toBe('The code index is not ready yet.');
  });

  it('says any other failure with its reason', () => {
    expect(searchFailure('code index: database disk image is malformed')).toBe('Code search failed: code index: database disk image is malformed');
    expect(searchFailure(new Error('no repository is open'))).toBe('Code search failed: no repository is open');
  });
});

describe('searchEnd', () => {
  it('a search that answered ends with its result and no failure', async () => {
    const found = { citations: [], confidence: 'low' };
    expect(await searchEnd(Promise.resolve(found))).toEqual({ res: found, failed: null });
  });

  it('a search that failed ends with the failure in words, not as a search that was never asked', async () => {
    // The view shows its empty state ("Ask the index.") only with no result and no failure.
    expect(await searchEnd(Promise.reject(new Error('no repository is open')))).toEqual({ res: null, failed: 'Code search failed: no repository is open' });
    expect(await searchEnd(Promise.reject('the code index is not ready; use grep/rg instead'))).toEqual({ res: null, failed: 'The code index is not ready yet.' });
  });
});
