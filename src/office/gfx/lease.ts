// A helper resource that is costly to hold (a WebGL context: the page gets few of them) is made
// on first use and let go a moment after its last use, so it is not held for the whole session.

/** `take()` gives the resource, making it when there is none; `idleMs` after the last `take`,
 *  `release` gets it back. */
export function idleLease<T>(make: () => T, release: (held: T) => void, idleMs: number): () => T {
  let held: { it: T } | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  return () => {
    held ??= { it: make() };
    const { it } = held;
    clearTimeout(timer);
    timer = setTimeout(() => {
      held = null;
      release(it);
    }, idleMs);
    return it;
  };
}
