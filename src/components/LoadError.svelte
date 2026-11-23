<!-- A load that failed: what, why (the core's own words), and a way to try again. -->
<script lang="ts">
  interface Props {
    /** What could not be loaded, after "Couldn’t load": "the endpoints". */
    what: string;
    error: string;
    /** A new load is on its way. */
    loading?: boolean;
    onretry: () => void;
  }
  let { what, error, loading = false, onretry }: Props = $props();
</script>

<p class="load-err" role="alert">
  <span>Couldn’t load {what}: {error}</span>
  <button class="btn sm" disabled={loading} onclick={onretry}>{loading ? 'Trying…' : 'Try again'}</button>
</p>

<style>
  .load-err { display: flex; align-items: center; gap: 10px; font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--bad); }
  .load-err span { min-width: 0; overflow-wrap: anywhere; }
  .btn { flex-shrink: 0; }
</style>
