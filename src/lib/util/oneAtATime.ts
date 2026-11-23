// For a read that costs something (it starts git) and is asked for by events that come in bursts.

/**
 * `job`, one run at a time. A call while it runs asks for one more run after it, however many
 * such calls come: no two runs overlap, an older answer never lands after a newer one, and the
 * last run always starts after the last call. `job` handles its own errors.
 */
export function oneAtATime(job: () => Promise<void>): () => Promise<void> {
  let running: Promise<void> | null = null;
  let again = false;
  return () => {
    if (running) {
      again = true;
      return running;
    }
    running = (async () => {
      try {
        do {
          again = false;
          await job();
        } while (again);
      } finally {
        running = null;
      }
    })();
    return running;
  };
}
