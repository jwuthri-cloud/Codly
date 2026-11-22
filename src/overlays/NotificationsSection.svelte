<!--
  Settings → General: the notifications of your system. One switch per kind of event, and one
  for when they show. The app sends them (src-tauri/src/notify.rs): each thing is told once,
  and several of one kind within a few seconds show as one notification.
-->
<script lang="ts">
  import type { NotificationSettings } from '../lib/gen/NotificationSettings';
  import { updateSettings } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { NOTIFY_KINDS, NOTIFY_WHEN, withSwitch } from '../lib/util/notifications';

  const n = $derived(store.settings.notifications);
  const set = (key: keyof NotificationSettings, on: boolean) => updateSettings({ notifications: withSwitch(n, key, on) });
</script>

<section>
  <h3 class="eyebrow">Notifications</h3>
  <p class="desc">
    Codly tells you through your system’s notifications when something needs you. Several of one kind within a few seconds show as one.
    Click a notification soon after it shows, and Codly opens what it names.
  </p>
  {#each [NOTIFY_WHEN, ...NOTIFY_KINDS] as s (s.key)}
    <label class="row toggle">
      <div class="r-text"><span class="r-title">{s.title}</span><span class="desc">{s.body}</span></div>
      <input type="checkbox" class="switch" checked={n[s.key]} onchange={(e) => set(s.key, e.currentTarget.checked)} />
    </label>
  {/each}
</section>

<style>
  section { padding: 20px 0; box-shadow: inset 0 -1px 0 var(--line); display: flex; flex-direction: column; gap: 12px; }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .row { display: flex; align-items: center; gap: 16px; min-height: 42px; }
  .r-text { flex: 1; display: flex; flex-direction: column; gap: 2px; }
  .r-title { font-weight: var(--w-med); }
</style>
