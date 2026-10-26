<!-- Codly mark: an observatory dome with its slit open to an aurora sky. -->
<script lang="ts">
  interface Props {
    size?: number;
  }
  let { size = 22 }: Props = $props();
  const uid = $props.id();
</script>

<svg width={size} height={size} viewBox="0 0 24 24" aria-label="Codly" role="img">
  <defs>
    <linearGradient id="lg-{uid}" x1="0" y1="0" x2="1" y2="0.4">
      <stop offset="0" style="stop-color: var(--aurora-a)" />
      <stop offset="0.5" style="stop-color: var(--aurora-m)" />
      <stop offset="1" style="stop-color: var(--aurora-b)" />
    </linearGradient>
  </defs>
  <path d="M3 17.5a9 9 0 0 1 18 0z" style="fill: var(--ink-1)" />
  <path d="M10.6 8.6l2.8 0 1.5 8.9h-5.8z" fill="url(#lg-{uid})" />
  <rect x="2" y="18.6" width="20" height="2.2" rx="1.1" style="fill: var(--ink-1)" opacity="0.55" />
</svg>
