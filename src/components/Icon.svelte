<!--
  Hand-drawn 16×16 line icons (stroke 1.5, round caps) — one consistent family, no icon dependency.
  Static markup only (safe for {@html}).
-->
<script lang="ts" module>
  const I: Record<string, string> = {
    home: '<path d="M2.75 7.25L8 3l5.25 4.25V13a.75.75 0 0 1-.75.75H9.75V10h-3.5v3.75H3.5a.75.75 0 0 1-.75-.75z"/>',
    plan: '<rect x="1.75" y="3" width="4.5" height="3.25" rx="1"/><rect x="1.75" y="9.75" width="4.5" height="3.25" rx="1"/><rect x="9.75" y="6.4" width="4.5" height="3.25" rx="1"/><path d="M6.25 4.6c2 0 1.6 3.4 3.5 3.4M6.25 11.4c2 0 1.6-3.4 3.5-3.4"/>',
    office: '<rect x="4.25" y="2.5" width="7.5" height="5.25" rx="1"/><path d="M8 7.75v2.25M1.75 10.25h12.5M3.25 10.25v3.25M12.75 10.25v3.25"/><path d="M6.5 5.25h3" opacity=".6"/>',
    focus: '<circle cx="8" cy="8" r="2.4"/><path d="M2.5 5.5V3.75a1.25 1.25 0 0 1 1.25-1.25H5.5M10.5 2.5h1.75a1.25 1.25 0 0 1 1.25 1.25V5.5M13.5 10.5v1.75a1.25 1.25 0 0 1-1.25 1.25H10.5M5.5 13.5H3.75a1.25 1.25 0 0 1-1.25-1.25V10.5"/>',
    image: '<rect x="2.25" y="2.75" width="11.5" height="10.5" rx="2"/><circle cx="5.75" cy="6" r="1.2"/><path d="M2.5 11.5l3.25-3 2.5 2.25L10.5 8.5l3.25 3"/>',
    shapes: '<circle cx="5.25" cy="5.25" r="2.75"/><path d="M10.5 2.75l3 5h-6z"/><rect x="7.75" y="9.25" width="5.5" height="4.25" rx="1"/><path d="M2.5 13.5l2.75-4.5L8 13.5z" opacity=".6"/>',
    globe: '<circle cx="8" cy="8" r="5.75"/><path d="M2.25 8h11.5M8 2.25c1.6 1.6 2.4 3.5 2.4 5.75S9.6 12.15 8 13.75C6.4 12.15 5.6 10.25 5.6 8S6.4 3.85 8 2.25z"/>',
    table: '<rect x="2.25" y="2.75" width="11.5" height="10.5" rx="1.75"/><path d="M2.25 6.25h11.5M2.25 9.75h11.5M6.25 6.25v7"/>',
    code: '<path d="M5.5 4.5L2 8l3.5 3.5M10.5 4.5L14 8l-3.5 3.5M9 3l-2 10"/>',
    reveal: '<path d="M2.75 4.5a1 1 0 0 1 1-1H6.5l1.5 1.5h4.25a1 1 0 0 1 1 1v1.25"/><path d="M2.75 4.5v7a1 1 0 0 0 1 1H8"/><path d="M10.25 9.25h3.5v3.5M13.75 9.25l-4 4"/>',
    expand: '<path d="M9.5 2.75h3.75V6.5M13.25 2.75L9 7M6.5 13.25H2.75V9.5M2.75 13.25L7 9"/>',
    question: '<path d="M3.5 3h9a1.5 1.5 0 0 1 1.5 1.5v5.25a1.5 1.5 0 0 1-1.5 1.5H8.5L5.5 13.5v-2.25h-2A1.5 1.5 0 0 1 2 9.75V4.5A1.5 1.5 0 0 1 3.5 3z"/><path d="M6.6 5.8a1.45 1.45 0 1 1 2 1.35c-.38.16-.6.48-.6.9v.2"/><circle cx="8" cy="9.6" r=".55" fill="currentColor" stroke="none"/>',
    sparkle: '<path d="M8 2.5l1.3 3.7 3.7 1.3-3.7 1.3L8 12.5l-1.3-3.7L3 7.5l3.7-1.3z"/><path d="M12.5 11.25v2.5M11.25 12.5h2.5" opacity=".7"/>',
    crew: '<circle cx="5" cy="6.25" r="2.25"/><circle cx="11" cy="6.25" r="2.25"/><path d="M1.75 13a3.25 3.25 0 0 1 6.5 0M7.75 13a3.25 3.25 0 0 1 6.5 0"/>',
    keep: '<path d="M4.5 2.75h7v10.5L8 10.5l-3.5 2.75z"/>',
    pin: '<path d="M9.75 2.5l3.75 3.75-1.75.5-2.25 2.25.25 2.75-1 1-2.25-2.25L3.5 13.5M5.5 8.5L4.25 7.5l1-1 2.75.25 2.25-2.25z"/>',
    x: '<path d="M4.5 4.5l7 7M11.5 4.5l-7 7"/>',
    plus: '<path d="M8 3.5v9M3.5 8h9"/>',
    minus: '<path d="M3.5 8h9"/>',
    check: '<path d="M3.5 8.5l3 3 6-7"/>',
    'chevron-right': '<path d="M6.5 4l4 4-4 4"/>',
    'chevron-left': '<path d="M9.5 4l-4 4 4 4"/>',
    'chevron-down': '<path d="M4 6.5l4 4 4-4"/>',
    'chevron-up': '<path d="M4 9.5l4-4 4 4"/>',
    'chevrons-up-down': '<path d="M5 6l3-3 3 3M5 10l3 3 3-3"/>',
    more: '<circle cx="3.75" cy="8" r=".9" fill="currentColor" stroke="none"/><circle cx="8" cy="8" r=".9" fill="currentColor" stroke="none"/><circle cx="12.25" cy="8" r=".9" fill="currentColor" stroke="none"/>',
    search: '<circle cx="7" cy="7" r="4.25"/><path d="M10.25 10.25l3.25 3.25"/>',
    inbox: '<path d="M2.5 9.5h3l1 1.5h3l1-1.5h3"/><path d="M2.5 9.5V12a1.5 1.5 0 0 0 1.5 1.5h8a1.5 1.5 0 0 0 1.5-1.5V9.5l-1.8-5a1.2 1.2 0 0 0-1.1-.8H5.4a1.2 1.2 0 0 0-1.1.8z"/>',
    chat: '<path d="M4.5 2.75h7a2 2 0 0 1 2 2v4.5a2 2 0 0 1-2 2H7.75L4.5 13.5v-2.25h0a2 2 0 0 1-2-2v-4.5a2 2 0 0 1 2-2z"/>',
    timeline: '<path d="M2.75 4.25h6M5.75 8h7.5M2.75 11.75h4M9.25 11.75h4"/>',
    board: '<rect x="2.5" y="2.75" width="3" height="10.5" rx="1"/><rect x="6.5" y="2.75" width="3" height="7" rx="1"/><rect x="10.5" y="2.75" width="3" height="8.75" rx="1"/>',
    graph: '<circle cx="4" cy="4.25" r="1.75"/><circle cx="12" cy="8" r="1.75"/><circle cx="4" cy="11.75" r="1.75"/><path d="M5.6 5l4.8 2.25M5.6 11l4.8-2.25"/>',
    diff: '<path d="M8 2.75v5.5M5.25 5.5h5.5M5.25 12.25h5.5"/>',
    terminal: '<rect x="2" y="3" width="12" height="10" rx="2"/><path d="M5 6.5l2 1.5-2 1.5M8.5 10h2.5"/>',
    memory: '<path d="M8 2.5l5.5 3-5.5 3-5.5-3z"/><path d="M2.5 8.5l5.5 3 5.5-3"/><path d="M2.5 11l5.5 3 5.5-3" opacity=".55"/>',
    world: '<path d="M2.5 13.5h11M3.5 13.5V6.75L8 3.25l4.5 3.5v6.75"/><path d="M6.5 13.5v-3.25h3v3.25"/>',
    workspace: '<rect x="2.5" y="2.5" width="11" height="11" rx="2"/><path d="M6.25 2.5v11M6.25 6.5h7.25"/>',
    split: '<rect x="2.5" y="2.5" width="11" height="11" rx="2"/><path d="M8 2.5v11"/>',
    sidebar: '<rect x="2.5" y="2.5" width="11" height="11" rx="2"/><path d="M6.25 2.5v11"/>',
    'panel-right': '<rect x="2.5" y="2.5" width="11" height="11" rx="2"/><path d="M9.75 2.5v11"/>',
    settings: '<path d="M2.75 5h4.25M10.5 5h2.75M2.75 11h2.25M8.5 11h4.75"/><circle cx="8.75" cy="5" r="1.6"/><circle cx="6.75" cy="11" r="1.6"/>',
    branch: '<circle cx="5" cy="4" r="1.5"/><circle cx="5" cy="12" r="1.5"/><circle cx="11" cy="5" r="1.5"/><path d="M5 5.5v5M11 6.5v.5a3 3 0 0 1-3 3H5.25"/>',
    repo: '<path d="M2.75 4.5a1 1 0 0 1 1-1H6.5l1.5 1.5h4.25a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-8.5a1 1 0 0 1-1-1z"/>',
    file: '<path d="M4.25 2.5H9l2.75 2.75v8.25h-7.5z"/><path d="M8.75 2.5v3h3"/>',
    play: '<path d="M5.25 3.5v9l7-4.5z"/>',
    stop: '<rect x="4" y="4" width="8" height="8" rx="1.5"/>',
    pause: '<path d="M5.75 4v8M10.25 4v8"/>',
    copy: '<rect x="5.5" y="5.5" width="8" height="8" rx="1.5"/><path d="M10.5 5.5V4a1.5 1.5 0 0 0-1.5-1.5H4A1.5 1.5 0 0 0 2.5 4v5A1.5 1.5 0 0 0 4 10.5h1.5"/>',
    external: '<path d="M9 2.75h4.25V7M13.25 2.75L7.5 8.5M11 9.5V12a1.25 1.25 0 0 1-1.25 1.25H4A1.25 1.25 0 0 1 2.75 12V6.25A1.25 1.25 0 0 1 4 5h2.5"/>',
    send: '<path d="M8 13V3.5M4 7.25L8 3.25l4 4"/>',
    flag: '<path d="M3.75 14V2.5"/><path d="M3.75 3h8.5l-1.75 3 1.75 3h-8.5"/>',
    alert: '<path d="M8 2.75l5.75 10H2.25z"/><path d="M8 6.5v3"/><circle cx="8" cy="11.25" r=".6" fill="currentColor" stroke="none"/>',
    'x-circle': '<circle cx="8" cy="8" r="5.5"/><path d="M6.1 6.1l3.8 3.8M9.9 6.1L6.1 9.9"/>',
    'check-circle': '<circle cx="8" cy="8" r="5.5"/><path d="M5.6 8.2l1.7 1.7 3.2-3.6"/>',
    clock: '<circle cx="8" cy="8" r="5.5"/><path d="M8 5v3.1l2 1.4"/>',
    live: '<circle cx="8" cy="8" r="2" fill="currentColor" stroke="none"/><path d="M4.6 4.6a4.8 4.8 0 0 0 0 6.8M11.4 4.6a4.8 4.8 0 0 1 0 6.8"/>',
    trash: '<path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 9h5.8l.6-9"/>',
    refresh: '<path d="M13 3.5v3h-3"/><path d="M12.7 6.5A4.75 4.75 0 1 0 12.3 10"/>',
    merge: '<circle cx="5" cy="3.75" r="1.5"/><circle cx="5" cy="12.25" r="1.5"/><circle cx="11.5" cy="8.75" r="1.5"/><path d="M5 5.25v5.5M5 5.25c0 2.5 2.25 3.5 5 3.5"/>',
    commit: '<circle cx="8" cy="8" r="2.25"/><path d="M2.5 8h3.25M10.25 8h3.25"/>',
    beaker: '<path d="M6 2.5v4L3.1 12.4a1 1 0 0 0 .9 1.6h8a1 1 0 0 0 .9-1.6L10 6.5v-4M5.5 2.5h5M4.6 10h6.8"/>',
    user: '<circle cx="8" cy="5.5" r="2.5"/><path d="M3.5 13.5a4.5 4.5 0 0 1 9 0"/>',
    'user-plus': '<circle cx="6.5" cy="5.5" r="2.5"/><path d="M2 13.5a4.5 4.5 0 0 1 9 0M12.5 4.5v4M10.5 6.5h4"/>',
    users: '<circle cx="6" cy="5.5" r="2.25"/><path d="M2 13a4 4 0 0 1 8 0M10.5 3.4a2.25 2.25 0 0 1 0 4.3M11.5 9.3A4 4 0 0 1 14 13"/>',
    help: '<circle cx="8" cy="8" r="5.5"/><path d="M6.4 6.4a1.6 1.6 0 1 1 2.2 1.5c-.4.2-.6.5-.6.9v.4"/><circle cx="8" cy="10.9" r=".6" fill="currentColor" stroke="none"/>',
    info: '<circle cx="8" cy="8" r="5.5"/><path d="M8 7.3v3.5"/><circle cx="8" cy="5.3" r=".6" fill="currentColor" stroke="none"/>',
    eye: '<path d="M1.75 8S4 3.75 8 3.75 14.25 8 14.25 8 12 12.25 8 12.25 1.75 8 1.75 8z"/><circle cx="8" cy="8" r="2"/>',
    link: '<path d="M7 9a2.5 2.5 0 0 0 3.5 0l2-2A2.5 2.5 0 0 0 9 3.5l-.6.6M9 7a2.5 2.5 0 0 0-3.5 0l-2 2A2.5 2.5 0 0 0 7 12.5l.6-.6"/>',
    sun: '<circle cx="8" cy="8" r="2.75"/><path d="M8 1.75V3M8 13v1.25M1.75 8H3M13 8h1.25M3.6 3.6l.9.9M11.5 11.5l.9.9M3.6 12.4l.9-.9M11.5 4.5l.9-.9"/>',
    moon: '<path d="M13 9.6A5.25 5.25 0 1 1 6.4 3a4.25 4.25 0 0 0 6.6 6.6z"/>',
    monitor: '<rect x="2" y="3" width="12" height="8" rx="1.5"/><path d="M6 13.5h4M8 11v2.5"/>',
    filter: '<path d="M2.75 3.5h10.5l-4 4.75v3.75l-2.5 1.25v-5z"/>',
    at: '<circle cx="8" cy="8" r="2.4"/><path d="M10.4 8v.9a1.7 1.7 0 0 0 3.4 0V8a5.8 5.8 0 1 0-2.3 4.6"/>',
    hash: '<path d="M6.25 2.75l-1.5 10.5M11.25 2.75l-1.5 10.5M3.25 6h10M2.75 10h10"/>',
    slash: '<path d="M10.5 2.75l-5 10.5"/>',
    'arrow-right': '<path d="M3 8h10M9 4l4 4-4 4"/>',
    'arrow-left': '<path d="M13 8H3M7 4L3 8l4 4"/>',
    enter: '<path d="M12.75 3.5v4a2 2 0 0 1-2 2H3.75M6.25 7l-2.5 2.5 2.5 2.5"/>',
    lock: '<rect x="3.5" y="7" width="9" height="6.5" rx="1.5"/><path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2"/>',
    list: '<path d="M2.75 4.25l1 1 2-2M7.5 4.5h5.75M2.75 9.25l1 1 2-2M7.5 9.5h5.75M7.5 13h5.75"/>',
    grip: '<circle cx="6" cy="4" r=".9" fill="currentColor" stroke="none"/><circle cx="10" cy="4" r=".9" fill="currentColor" stroke="none"/><circle cx="6" cy="8" r=".9" fill="currentColor" stroke="none"/><circle cx="10" cy="8" r=".9" fill="currentColor" stroke="none"/><circle cx="6" cy="12" r=".9" fill="currentColor" stroke="none"/><circle cx="10" cy="12" r=".9" fill="currentColor" stroke="none"/>',
    gauge: '<path d="M2.75 11.5a5.25 5.25 0 1 1 10.5 0"/><path d="M8 11.5l2.6-3.1"/>',
    command: '<path d="M6 6V4.5A1.5 1.5 0 1 0 4.5 6H6zm0 0h4m-4 0v4m4-4V4.5A1.5 1.5 0 1 1 11.5 6H10zm0 0v4m0 0h1.5a1.5 1.5 0 1 1-1.5 1.5V10zm0 0H6m0 0v1.5A1.5 1.5 0 1 1 4.5 10H6z"/>',
    sparkline: '<path d="M2.5 11l3-3.5 2.5 2 3.5-5 2 2"/>',
    folder: '<path d="M2.75 4.5a1 1 0 0 1 1-1H6.5l1.5 1.5h4.25a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-8.5a1 1 0 0 1-1-1z"/>',
    target: '<circle cx="8" cy="8" r="5.5"/><circle cx="8" cy="8" r="2"/>',
    note: '<path d="M3.5 2.75h9v7.5l-3 3h-6z"/><path d="M12.5 10.25h-3v3M5.75 5.75h4.5M5.75 8h3"/>',
    book: '<path d="M3 3.25a1 1 0 0 1 1-1h8.75v9.5H4a1 1 0 0 0-1 1z"/><path d="M3 12.75a1 1 0 0 0 1 1h8.75v-2"/>',
    keyboard: '<rect x="1.75" y="4" width="12.5" height="8" rx="1.5"/><path d="M4.5 6.75h.01M7 6.75h.01M9.5 6.75h.01M12 6.75h.01M5 9.5h6"/>',
    bolt: '<path d="M8.75 1.75L4 9h3.75l-.5 5.25L12 7H8.25z"/>',
    layers: '<path d="M8 2.5l5.5 3-5.5 3-5.5-3z"/><path d="M2.5 8.5l5.5 3 5.5-3"/>',
    hand: '<path d="M5.5 8V3.75a1 1 0 0 1 2 0V7.5M7.5 7V2.75a1 1 0 0 1 2 0V7.5M9.5 7.25V3.75a1 1 0 0 1 2 0v5.5a4.5 4.5 0 0 1-4.5 4.5h-.4a4 4 0 0 1-3.1-1.5L2 10a1 1 0 0 1 1.5-1.3L5.5 10.5"/>',
  };
  export type IconName = keyof typeof I;
  export const iconNames = Object.keys(I);
</script>

<script lang="ts">
  interface Props {
    name: string;
    size?: number;
    stroke?: number;
    class?: string;
    title?: string;
  }
  let { name, size = 16, stroke = 1.5, class: cls = '', title }: Props = $props();
</script>

<svg
  class="icon {cls}"
  width={size}
  height={size}
  viewBox="0 0 16 16"
  fill="none"
  stroke="currentColor"
  stroke-width={stroke}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden={title ? undefined : 'true'}
  role={title ? 'img' : undefined}
  aria-label={title}
>
  {@html I[name] ?? I.help}
</svg>

<style>
  .icon { flex-shrink: 0; display: block; }
</style>
