// A small cache for text made from text by a pure function (a finished message's HTML).

/**
 * `cached(key, make)` is `make(key)`, made once while the pair is kept. The pairs used last are
 * kept, up to `maxEntries` pairs and `maxChars` characters (keys and values together); a pair
 * larger than `maxChars` is made every time.
 */
export function textCache(maxEntries: number, maxChars: number): (key: string, make: (key: string) => string) => string {
  // A Map keeps the order of insertion: the first pair is the one used longest ago.
  const kept = new Map<string, string>();
  let chars = 0;
  return (key, make) => {
    const hit = kept.get(key);
    if (hit !== undefined) {
      kept.delete(key);
      kept.set(key, hit);
      return hit;
    }
    const value = make(key);
    const size = key.length + value.length;
    if (size > maxChars) return value;
    kept.set(key, value);
    chars += size;
    for (const [k, v] of kept) {
      if (kept.size <= maxEntries && chars <= maxChars) break;
      kept.delete(k);
      chars -= k.length + v.length;
    }
    return value;
  };
}
