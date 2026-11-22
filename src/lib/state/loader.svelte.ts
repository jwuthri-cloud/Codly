// One thing a screen loads from the core (a Settings section's state): its value, or why the
// load failed, so the screen can say so and offer "Try again" instead of waiting for ever.
// A failed reload keeps the last good value; `fails` counts failures in a row (for a poll).

export class Loader<T> {
  value = $state.raw<T | null>(null);
  /** Why the last load failed (null: it worked, or none ran yet). */
  error = $state<string | null>(null);
  loading = $state(false);
  /** Loads that failed in a row. */
  fails = $state(0);
  private seq = 0;
  private readonly fetch: () => Promise<T>;

  constructor(fetch: () => Promise<T>) {
    this.fetch = fetch;
  }

  /** Load (again). Only the newest call's answer counts. True when it worked. */
  async load(): Promise<boolean> {
    const mine = ++this.seq;
    this.loading = true;
    try {
      const v = await this.fetch();
      if (mine === this.seq) this.set(v);
      return true;
    } catch (e) {
      if (mine === this.seq) {
        this.error = e instanceof Error ? e.message : String(e);
        this.fails++;
      }
      return false;
    } finally {
      if (mine === this.seq) this.loading = false;
    }
  }

  /** A newer value from elsewhere (the answer of a save): it wins over a load still on its way. */
  set(v: T) {
    this.seq++;
    this.value = v;
    this.error = null;
    this.fails = 0;
    this.loading = false;
  }
}
