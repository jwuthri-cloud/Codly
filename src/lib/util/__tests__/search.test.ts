import { describe, expect, it } from 'vitest';
import { searchFailure } from '../search';

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
