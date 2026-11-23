// For a plain module (no component): run a function again whenever the state it read changes.

/** Runs `fn` now and after each change of the `$state` it read. Call the result to stop. */
export function watch(fn: () => void): () => void {
  return $effect.root(() => {
    $effect(fn);
  });
}
