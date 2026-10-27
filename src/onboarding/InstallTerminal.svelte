<!--
  A small live terminal for one install or sign-in run. The command comes from Codly's own table
  (never typed here); you can still answer prompts, such as a password, right in it.
-->
<script lang="ts">
  import '@xterm/xterm/css/xterm.css';
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import { xtermTheme } from '../rooms/terminal/sessions';

  interface Props {
    ptyId: string;
  }
  let { ptyId }: Props = $props();
  let host: HTMLDivElement | undefined = $state();

  onMount(() => {
    let dead = false;
    let cleanup = () => {};
    (async () => {
      const [{ Terminal }, { FitAddon }] = await Promise.all([import('@xterm/xterm'), import('@xterm/addon-fit')]);
      if (dead || !host) return;
      const term = new Terminal({ fontFamily: '"JetBrains Mono Variable", ui-monospace, Menlo, monospace', fontSize: 12, lineHeight: 1.2, rows: 9, theme: xtermTheme(), scrollback: 2000 });
      const fit = new FitAddon();
      term.loadAddon(fit);
      term.open(host);
      const refit = () => {
        try {
          fit.fit();
          api.ptyResize(ptyId, term.cols, term.rows).catch(() => {});
        } catch {
          /* not measurable yet */
        }
      };
      refit();
      const ro = new ResizeObserver(refit);
      ro.observe(host);
      let busy = false;
      let pending = '';
      const flush = async () => {
        if (busy || !pending) return;
        busy = true;
        const d = pending;
        pending = '';
        try {
          await api.ptyWrite(ptyId, d);
        } catch {
          term.write('\r\n\x1b[2m[finished]\x1b[0m\r\n');
        } finally {
          busy = false;
          if (pending) void flush();
        }
      };
      term.onData((d) => {
        pending += d;
        void flush();
      });
      await api.ptyAttach(ptyId, (bytes) => term.write(bytes));
      cleanup = () => {
        ro.disconnect();
        term.dispose();
      };
    })();
    return () => {
      dead = true;
      cleanup();
    };
  });
</script>

<div class="term" bind:this={host} aria-label="Install terminal"></div>

<style>
  .term { height: 190px; padding: 8px 10px; border-radius: var(--r-md); background: var(--glass-solid); box-shadow: inset 0 0 0 1px var(--line-2); overflow: hidden; }
</style>
