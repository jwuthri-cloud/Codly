# Codly: Tauri 2 + Frontend Stack (checked 2026-10-25)

Scope: the Tauri shell, Rust↔webview streaming, Rust crates for the core, the frontend framework and the component libraries, a scaffold that runs without prompts, macOS idle CPU and memory, and headless testing.
Machine used: macOS 26.5.2 (arm64, Apple M5 Pro), rustc/cargo 1.96.0, node 22.23.3, pnpm 11.5.0, bun 1.3.14, git 2.54.0 (Homebrew; `/usr/bin/git` from Xcode is also present).

**Evidence labels** used throughout:

- **[V-src]**: I read it in the upstream source code. Clones are in `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/research-src/{tauri,tauri-v3,plugins-workspace,create-tauri-app,fix-path-env-rs}`.
- **[V-docs]**: I read it in official docs on the date above.
- **[V-reg]**: I checked it on the npm or crates.io registry on the date above.
- **[V-local]**: I ran it on this machine. Scaffolds are in `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/scaffold-test/`.
- **[Claim]**: project-reported, community or experience-based. Not verified today.

Rust code snippets in this report have **not been compiled**. I deliberately did not run `cargo build` or `cargo check` on downloaded code. Treat them as design sketches that are close to correct.

---

## 0. Decisions at a glance

| Area | Decision | Version (checked today) |
|---|---|---|
| Shell | **Tauri 2.12.x stable**. Do not start on the v3 alpha. | `tauri` 2.12.5, `tauri-build` 2.7.4, `@tauri-apps/api` 2.12.4, `@tauri-apps/cli` 2.12.4 [V-reg] |
| Hot-path IPC | **`tauri::ipc::Channel<T>`**, one per stream, with batches built in Rust at about 30 Hz. Never use `emit` on hot paths. | [V-src] |
| Binary IPC | `Channel<InvokeResponseBody>` sending `InvokeResponseBody::Raw`. **Not** `Channel<Vec<u8>>` or `Channel<&[u8]>`, which are sent as JSON number arrays. | [V-src] |
| Frontend | **Svelte 5 (runes) + plain Vite 8, no SvelteKit**, TypeScript 6.0 | `svelte` 5.57.2, `@sveltejs/vite-plugin-svelte` 7.3.1, `vite` 8.3.5, `typescript` 6.0.3 [V-reg] |
| Terminal | `@xterm/xterm` 6 + WebGL addon + `portable-pty` | 6.0.0 / 0.19.0 / `portable-pty` 0.9.0 |
| Diff review | `@pierre/diffs` (vanilla API, Shiki) for agent patches. CodeMirror 6 for the code viewer and editable merge. | 1.6.1 / `@codemirror/view` 6.43.15, `@codemirror/merge` 6.12.3 |
| Dependency graph | `@xyflow/svelte` (Svelte Flow) + `@dagrejs/dagre` layout. elkjs only if compound or port layouts are needed. | 1.7.0 / 3.1.3 |
| Kanban DnD | `svelte-dnd-action`, which is pointer-based and so avoids the Tauri file-drop vs HTML5 DnD conflict | 0.9.80 |
| Pixel world | **Canvas 2D**, own render loop that runs only when something changed. Not PixiJS. | — |
| Palette | `bits-ui` `Command` | 2.20.2 |
| Markdown | `marked` + `dompurify` + `shiki` (JS regex engine, lazy-loaded) | 18.1.1 / 3.4.17 / 4.5.0 |
| Virtualization | `virtua` (native Svelte 5 export). xterm for raw logs. | 0.53.6 |
| SQLite | **`rusqlite` 0.40.2 `bundled`** + `rusqlite_migration` 2.6.0, one writer actor, WAL. **Not** sqlx. | [V-reg] |
| FS watching | `notify` 8.2.1 + `notify-debouncer-full` 0.7.0 (FSEvents) | [V-reg] |
| Git | **Shell out to `git`** for all mutations (worktree, merge, diff). gix later, for read-only hot paths only. | gix 0.90.0 / git2 0.21.0 [V-reg] |
| Search | `ignore` 0.4.33 + `grep-searcher` 0.1.17 + `grep-regex` 0.1.14 | [V-reg] |
| PATH fix | **Own ~60-line `shell_env` module**: `$SHELL -ilc` with a timeout, `env -0`, explicit `envs()` on every spawn, plus `which::which_in`. Do not depend on `fix-path-env`. | `which` 8.0.6 |
| Types across IPC | `ts-rs` 12.0.1 (stable). Not `tauri-specta`, which is still `2.0.0-rc.26`. | [V-reg] |
| Testing | Frontend in a browser with a Transport abstraction + Playwright WebKit. Real app E2E via `@wdio/tauri-service` + `tauri-plugin-wdio-webdriver`, which embeds a WebDriver server and works on macOS. | 1.64.1 / 1.5.1 |

---

## 1. Tauri: versions and release status

### 1.1 Current versions [V-reg][V-src]

| Package | Latest stable | Published | Notes |
|---|---|---|---|
| `tauri` (crate) | **2.12.5** | 2026-10-21 | MSRV **1.90** (raised in 2.12.0); edition 2024 |
| `tauri-build` | 2.7.4 | 2026-10-21 | |
| `tauri-runtime-wry` / `wry` / `tao` | 2.12.3 / 0.57.1 / 0.37.2 | | resolved by `cargo generate-lockfile` in the scaffold [V-local] |
| `@tauri-apps/api` | **2.12.4** | 2026-10-14 | `next` tag = 3.0.0-alpha.3 |
| `@tauri-apps/cli` (npm) / `tauri-cli` (crate) | **2.12.4** | 2026-10-21 | `pnpm tauri --version` → `tauri-cli 2.12.4` [V-local] |
| `create-tauri-app` (npm) | 4.7.5 | 2026-10-09 | the binary prints `4.8.0` in `--help` [V-local] |

Changes in 2.12.x that matter for Codly [V-src: `crates/tauri/CHANGELOG.md`]:

- **Security fix in 2.12.0 (GHSA-w28w-mhc8-qvjv).** Channel payload queues are now bound to the webview that created them and purged when it closes. This is a reason to stay at ≥2.12.0.
- **2.12.0:** Rust-side event listeners and JS listener entries no longer leak when a webview or window is destroyed. Asset-protocol files now load asynchronously, so they no longer block the event loop.
- **2.12.3:** a `Channel` keeps delivering messages after the `onmessage` handler throws. Before this, every later message stayed queued. Earlier versions are dangerous for a streaming UI.
- **2.12.1:** transparency and fullscreen on macOS no longer need `macos-private-api`.
- **2.12.0:** adds macOS 27 Liquid Glass window effects (`Regular`/`Clear`, `interactive`).

### 1.2 Tauri v3 is in alpha. Do not start on it [V-src][V-reg]

- `tauri` 3.0.0-alpha.0 shipped 2026-09-13. The latest is **3.0.0-alpha.5** (2026-10-14). MSRV is 1.95. Every official plugin has a `3.0.0-alpha.x` line.
- Breaking direction: the webview runtime is now chosen at build time with `tauri::Builder::default().runtime(tauri_runtime_wry::Wry::default())`. The other option is `tauri_runtime_cef::Cef`, a Chromium Embedded Framework runtime that is also alpha. Runtime-specific APIs such as `with_webview` move to extension traits in the runtime crates. ACL manifests also get smaller, which changes the autogenerated permission storage.
- **What Codly should do:** build on 2.12.x. Put every `with_webview`/wry/objc call in a single `platform/` module so the v3 migration touches one place. CEF on v3 is a future escape hatch if a WKWebView quirk ever blocks us [Claim: still alpha].

---

## 2. Streaming Rust → webview

### 2.1 How `Channel` actually works [V-src: `crates/tauri/src/ipc/channel.rs`]

- `Channel<TSend = InvokeResponseBody>`. Each `send()` stamps a monotonically increasing `index`. The JS `Channel` class buffers out-of-order messages and delivers them **in order** [V-src: `packages/api/src/core.ts`].
- Delivery path per message:
  - **JSON < 8192 bytes** → `webview.eval("…runCallback(id, {message: <json>, index})")`. The JSON goes straight into the script.
  - **Raw < 1024 bytes** → `eval` of `new Uint8Array([..]).buffer`.
  - **Anything bigger** → the payload is queued in `ChannelDataIpcQueue`. A small `eval` then makes the page `invoke('plugin:__TAURI_CHANNEL__|fetch')` it over the custom IPC protocol, so it never passes through a JS string. Source comments: 8 KB JSON is "~2x faster through eval than fetch on WebView2", and 1 KB raw is "~30% faster through eval than fetch on macOS".
- **Every `send()` costs at least one `evaluateJavaScript` round trip** into the WebContent process. Batching in Rust therefore matters more than serde speed.
- When the last Rust clone of a frontend-created channel is dropped, the JS side gets `{end: true}` and unregisters its callback.
- Since 2.12.x, `Builder::channel_interceptor` lets you replace delivery entirely. Codly does not need it.

### 2.2 `emit` vs `Channel` [V-src][V-docs]

- `emit` serializes to JSON (`EmitArgs::new` → `serde_json::to_string`). It then `eval`s a script into **every webview that has a listener**, and it has no fetch path for large payloads [V-src: `event/mod.rs`, `event/listener.rs`].
- The official docs say: *"The event system is not designed for low latency or high throughput situations"*. Channels *"are designed to be fast and deliver ordered data"* [V-docs: v2.tauri.app/develop/calling-frontend].
- **Rule for Codly:** use `emit` only for rare, global notifications such as "settings changed" or "update available". Everything that streams uses Channels.

### 2.3 Binary payload gotcha [V-src]

The docs page "Calling Rust" shows `tauri::ipc::Channel<&[u8]>` for streaming file chunks [V-docs]. The source documentation contradicts it: *"a `Channel<Vec<u8>>` does not do this — `Vec<u8>` is `Serialize`, so it is sent as a JSON array of numbers."*
For real binary, use `Channel<InvokeResponseBody>` with `InvokeResponseBody::Raw(bytes)`. JS then receives an `ArrayBuffer`. For one-shot large results from a command, return `tauri::ipc::Response::new(bytes)`.

### 2.4 Codly UI-bus design (recommended)

1. **One UI Channel per webview** for structured events, and **one Channel per attached terminal** for raw PTY bytes.
2. **Rust coalesces events.** It merges consecutive `TextDelta`s for the same message, collapses repeated status updates for the same agent, and flushes at **≈30 Hz (33 ms)**. It flushes immediately when a batch passes about 4k events. A focused terminal flushes at ≈60 Hz (16 ms) or 64 KiB. Unfocused terminals stay buffered in a Rust ring buffer and are not streamed.
3. **No wakeups while idle.** The pump waits on `rx.recv()`. Only the first event arms a deadline. Never use a permanent `tokio::time::interval`.
4. **Snapshot + seq protocol.** Every domain event carries the core's monotonic `seq`, the same id as its SQLite event-log row. `ui_subscribe` installs the sink first, then returns a snapshot `{asOf: seq, …}`. The client drops any `seq <= asOf`. This one protocol covers reloads, HMR, a webview suspended in the background (§6) and late-opened windows.
5. **Visibility-aware.** On `visibilitychange` → hidden, the frontend calls `ui_set_visible(false)`. Rust then stops sending fine-grained deltas: no token deltas, only status. On visible, the frontend re-subscribes and gets a snapshot. This prevents a queue of `eval`s from piling up against a suspended WebContent process.
6. **Huge payloads never go through the bus.** Tool outputs, file contents and diffs are stored in SQLite or the file system. The bus carries a small preview and an id, and the UI fetches the full content on demand with a command that returns `Response`.

```rust
// src-tauri/src/ui_bus.rs (sketch)
use std::{sync::{Arc, Mutex}, time::Duration};
use tauri::ipc::Channel;
use tokio::{sync::mpsc, time::{sleep_until, Instant}};

#[derive(Clone, serde::Serialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum UiEvent {
    AgentStatus { agent: String, status: String },
    TextDelta   { agent: String, msg: String, text: String },
    Timeline    { item: serde_json::Value },
    // ...
}

#[derive(serde::Serialize)]
pub struct UiBatch { pub events: Vec<(u64, UiEvent)> } // (seq, event)

pub type UiSink = Arc<Mutex<Option<Channel<UiBatch>>>>;

pub async fn ui_pump(mut rx: mpsc::Receiver<(u64, UiEvent)>, sink: UiSink, window: Duration) {
    let mut buf: Vec<(u64, UiEvent)> = Vec::with_capacity(512);
    while let Some(first) = rx.recv().await {           // parked: zero wakeups when idle
        push_coalesced(&mut buf, first);
        let deadline = sleep_until(Instant::now() + window);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                biased;
                _ = &mut deadline => break,
                ev = rx.recv() => match ev {
                    Some(ev) => { push_coalesced(&mut buf, ev); if buf.len() >= 4096 { break } }
                    None => break,
                },
            }
        }
        let batch = UiBatch { events: std::mem::take(&mut buf) };
        let ch = sink.lock().unwrap().clone();           // never hold the lock across send
        if let Some(ch) = ch {
            if ch.send(batch).is_err() { *sink.lock().unwrap() = None; }
        }
    }
}

fn push_coalesced(buf: &mut Vec<(u64, UiEvent)>, (seq, ev): (u64, UiEvent)) {
    if let (Some((s0, UiEvent::TextDelta { msg: m0, text: t0, .. })), UiEvent::TextDelta { msg, text, .. }) =
        (buf.last_mut(), &ev)
    {
        if *m0 == *msg { t0.push_str(text); *s0 = seq; return; }
    }
    buf.push((seq, ev));
}

#[tauri::command]
pub async fn ui_subscribe(
    ui: tauri::State<'_, crate::UiState>,
    on_batch: Channel<UiBatch>,          // JS passes { onBatch: channel } (camelCase is the default) [V-src]
) -> Result<crate::Snapshot, crate::CmdError> {
    *ui.sink.lock().unwrap() = Some(on_batch);   // 1) start receiving deltas
    Ok(ui.core.snapshot().await?)                 // 2) then snapshot { as_of: seq, ... }
}
```

```ts
// src/lib/transport/tauri.ts
import { Channel, invoke } from '@tauri-apps/api/core';
type UiBatch = { events: [number, UiEvent][] };

export async function connect(store: Store) {
  let asOf = Number.POSITIVE_INFINITY;           // drop everything until the snapshot lands
  const early: UiBatch[] = [];
  const apply = (b: UiBatch) => { for (const [seq, ev] of b.events) if (seq > asOf) store.apply(ev); };
  const ch = new Channel<UiBatch>((b) => (asOf === Number.POSITIVE_INFINITY ? early.push(b) : apply(b)));
  const snap = await invoke<Snapshot>('ui_subscribe', { onBatch: ch });
  store.load(snap); asOf = snap.asOf; early.forEach(apply);
}
```

**Serde cost** at about 30 batches/s of a few KB each is negligible. Use `serde_json` by default. Parse agent stdout with `serde_json::from_slice` on bytes. If profiling ever shows JSON parsing of multi-MB agent lines as hot, `sonic-rs` 0.5.10 or `simd-json` 0.18.1 can be swapped in [V-reg; benefit is a Claim].

### 2.5 Commands with async state [V-docs][V-src]

- **Commands without `async` run on the main thread** [V-docs]. One slow sync command freezes the UI. Make every Codly command `async`, or mark it `#[tauri::command(async)]`.
- Async commands run on `tauri::async_runtime` (tokio). A borrowed argument such as `State<'_, T>` or `&str` in an async command requires a `Result` return type [V-docs].
- Keep `State` cheap: a `Clone` handle (an `mpsc::Sender` to an orchestrator actor) instead of `tokio::Mutex<BigState>`. Never hold a `std::sync::Mutex` across `.await`.

```rust
#[derive(Debug, thiserror::Error)]
pub enum CmdError {
    #[error(transparent)] Core(#[from] anyhow::Error),
    #[error("not found: {0}")] NotFound(String),
}
impl serde::Serialize for CmdError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> { s.serialize_str(&self.to_string()) }
}

#[tauri::command]
async fn start_task(core: tauri::State<'_, CoreHandle>, req: StartTask) -> Result<TaskId, CmdError> {
    Ok(core.start_task(req).await?)
}
```

---

## 3. Plugins: which to use [V-reg][V-src]

| Plugin | Use? | Crate / npm version | Why |
|---|---|---|---|
| `tauri-plugin-single-instance` | **Yes** | 2.5.4 / (Rust only) | Forwards `codly <path>` invocations to the running instance. The README says *"plugins run in the order they were added… make sure that this plugin is registered first"*. `tauri add` **prepends** each plugin, so after several `tauri add` runs single-instance ended up **last** [V-local]. Fix the order by hand. |
| `tauri-plugin-window-state` | **Yes** | 2.5.0 / 2.5.0 | The default flags are `StateFlags::all()`, which includes `VISIBLE`. Use `all() & !VISIBLE` if you ever hide windows to the tray [V-src]. |
| `tauri-plugin-dialog` | **Yes** | 2.8.3 / 2.8.3 | Repo and folder pickers. `dialog:default` = message/save/open. |
| `tauri-plugin-opener` | **Yes** | 2.7.2 / 2.7.2 | `openUrl`, `openPath`, `revealItemInDir`. `opener:default` = http(s)/mailto/tel + reveal. |
| `tauri-plugin-notification` | **Yes** | 2.7.0 / 2.7.0 | "Agent needs approval" or "done" alerts. **In `tauri dev` the notification is attributed to `com.apple.Terminal`** (`notify_rust::set_application(if tauri::is_dev() {"com.apple.Terminal"} …)`) [V-src]. Test real behavior from a bundled build. |
| `tauri-plugin-shell` | **No** | 2.5.0 | Agents are spawned from Rust with `tokio::process` and the PTY. Exposing a shell to JS only widens the attack surface. |
| `tauri-plugin-store` | **No** | 2.5.0 | Settings go in SQLite. One source of truth. |
| `tauri-plugin-sql` | **No** | — | It is sqlx-based and would conflict with rusqlite's `libsqlite3-sys` (§4.4). |
| `tauri-plugin-log` | Optional | 2.10.1 | Prefer `tracing` + `tracing-appender` directly. If used, enable its `tracing` feature [V-src]. |
| `tauri-plugin-updater` / `-process` | Later | 2.14.0 / 2.4.0 | 2.12 added `exit` to `@tauri-apps/api/app` (`core:app:allow-exit`), so `-process` is not needed just to quit [V-src]. |

---

## 4. Capabilities and permissions (Tauri 2 format) [V-src][V-local]

Files live in `src-tauri/capabilities/*.json` or `.toml`. All files in that folder are enabled by default. Fields, from `tauri-utils/src/acl/capability.rs`:

- `identifier`, `description`.
- `windows`: label globs. `webviews`: label globs, preferred for multi-webview windows.
- `local`: defaults to `true`.
- `remote`: `{ "urls": [...] }`. Never use it for Codly.
- `platforms`: e.g. `["macOS","windows","linux"]`.
- `permissions`: either strings `"<plugin>:<perm>"`, or objects `{ "identifier": "...", "allow": [...], "deny": [...] }` for scoped permissions.

The schema is generated at `src-tauri/gen/schemas/desktop-schema.json` when you build.

```json
// src-tauri/capabilities/main.json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "main",
  "description": "Main Codly window",
  "windows": ["main"],
  "platforms": ["macOS", "linux", "windows"],
  "permissions": [
    "core:default",
    "core:window:allow-start-dragging",
    "dialog:default",
    "opener:default",
    "notification:default",
    "window-state:default",
    "allow-ui-subscribe",
    "allow-start-task"
  ]
}
```

- `core:window:default` only grants read-only getters. `data-tauri-drag-region` needs **`core:window:allow-start-dragging`** [V-src: `permissions/window/autogenerated/reference.md`].
- **App commands are not ACL-checked until you declare an app manifest.** The check is `plugin_command.is_some() || has_app_acl_manifest || !is_local` [V-src: `webview/mod.rs`]. Declare one so every command must be granted explicitly. Permission ids are `allow-<command with _ → ->` [V-src: `tauri-utils/src/acl/build.rs`].

```rust
// src-tauri/build.rs
fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "ui_subscribe", "ui_set_visible", "start_task", "cancel_task",
            "pty_attach", "pty_write", "pty_resize", "get_artifact",
        ]),
    )).expect("tauri-build failed");
}
```

- `tauri add <plugin>` on desktop-only plugins created a `desktop.json` capability with `platforms` set. After `tauri add dialog` I did **not** find `dialog:default` added to any capability, so check capabilities after every `tauri add` [V-local].
- CLI helpers [V-local]: `tauri capability new --windows main --permission core:default`, `tauri permission add|rm|ls`.
- **Set a real CSP.** Templates ship `"csp": null`. Start from:
  `"default-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' asset: http://asset.localhost data: blob:; style-src 'self' 'unsafe-inline'; worker-src 'self' blob:"`
  [Claim: verify against Shiki and CodeMirror at runtime. Tauri injects hashes and nonces for its own scripts.]

---

## 5. Rust crates

### 5.1 Versions [V-reg, 2026-10-25]

| Purpose | Crate | Latest stable | Notes |
|---|---|---|---|
| async | `tokio` | 1.54.1 | features `rt-multi-thread,macros,process,io-util,sync,time,fs,signal` |
| | `tokio-util` | 0.7.20 | |
| PTY | `portable-pty` | 0.9.0 (2025-02-11) | On crates.io since Feb 2025, but upstream `wezterm/pty` still gets commits (2026-09-12). 14M recent downloads; used by Codex. |
| SQLite | `rusqlite` | 0.41.0 (**2026-10-24**) / **0.40.2** | see §5.4 |
| | `rusqlite_migration` | 2.6.0 | requires `rusqlite ^0.40` |
| | `sqlx` | 0.9.0 | MSRV 1.94. Its SQLite driver pins `libsqlite3-sys >=0.30.1, <0.38` |
| watch | `notify` / `notify-debouncer-full` | 8.2.1 / 0.7.0 | 9.0.0-rc.6 and 0.8.0-rc.2 are pre-release. Default feature is `macos_fsevent` on both. |
| git | `gix` / `git2` | 0.90.0 / 0.21.0 | see §5.6 |
| search | `ignore` / `grep-searcher` / `grep-regex` / `grep-matcher` | 0.4.33 / 0.1.17 / 0.1.14 / 0.1.9 | |
| parse | `tree-sitter` | 0.27.1 (MSRV 1.90) | grammars: `tree-sitter-rust` 0.24.2, `-typescript` 0.23.2, `-javascript` 0.25.0, `-python` 0.25.0, `-go` 0.25.0; `tree-sitter-tags` 0.27.1; `tree-sitter-language-pack` 1.22.1 |
| serde | `serde` / `serde_json` | 1.0.229 / 1.0.154 | |
| errors | `thiserror` / `anyhow` | 2.0.21 / 1.0.104 | |
| logs | `tracing` / `tracing-subscriber` / `tracing-appender` | 0.1.44 / 0.3.23 / 0.2.5 | `tauri` has a `tracing` feature for IPC spans [V-src] |
| misc | `memchr` 2.8.3, `which` 8.0.6, `nix` 0.31.3, `dirs` 7.0.0, `uuid` 1.29.0, `ts-rs` 12.0.1, `parking_lot` 0.13.0, `axum` 0.8.9, `objc2` 0.6.5, `objc2-foundation` 0.3.2 | | |
| types | `tauri-specta` / `specta` | 2.0.0-rc.26 / 2.0.0-rc.27 | still RC; stable is 1.x. Avoid for now. |

### 5.2 `tokio::process` + JSONL with very large lines

Facts:

- `tokio_util::codec::LinesCodec::new()` has **no maximum length**. Docs: an attacker *"could exploit this unbounded buffer"*. `new_with_max_length(n)` returns `MaxLineLengthExceeded` and then discards up to the next `\n`.
- `FramedRead` yields `None` once after a decoder error ([tokio#3976]). A `while let Some(..)` loop therefore stops silently [V-src: `framed_impl.rs`].
- `LinesCodec` also forces UTF-8 `String`s.

So use a small hand-written **bounded byte reader**. Agents can print multi-MB JSON lines, for example tool results containing whole files.

```rust
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

pub enum Line { Complete, TooLong(usize), Eof }

/// Read one '\n'-terminated record into `buf` (newline stripped) without ever buffering > `max` bytes.
pub async fn read_record<R: AsyncBufRead + Unpin>(r: &mut R, buf: &mut Vec<u8>, max: usize) -> std::io::Result<Line> {
    buf.clear();
    let (mut seen, mut overflow) = (0usize, false);
    loop {
        let avail = r.fill_buf().await?;
        if avail.is_empty() {
            return Ok(match (seen, overflow) { (0, _) => Line::Eof, (n, true) => Line::TooLong(n), _ => Line::Complete });
        }
        let (chunk, used, done) = match memchr::memchr(b'\n', avail) {
            Some(i) => (&avail[..i], i + 1, true),
            None => (avail, avail.len(), false),
        };
        seen += chunk.len();
        if !overflow && buf.len() + chunk.len() > max { overflow = true; buf.clear(); }
        if !overflow { buf.extend_from_slice(chunk); }
        r.consume(used);
        if done {
            if buf.last() == Some(&b'\r') { buf.pop(); }
            return Ok(if overflow { Line::TooLong(seen) } else { Line::Complete });
        }
    }
}
```

```rust
use std::process::Stdio;
use tokio::{io::BufReader, process::Command};

let mut child = Command::new(&resolved_bin)          // absolute path from which_in (§5.10)
    .args(&args)
    .current_dir(&worktree)
    .env_clear().envs(&env)                          // captured login env + per-agent overrides
    .stdin(Stdio::piped())                           // or Stdio::null() for one-shot modes
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true)                              // SIGKILL to the direct child on drop
    .process_group(0)                                // child gets its own pgid → killpg kills the tree
    .spawn()?;
let pgid = child.id().unwrap() as i32;
tokio::spawn(drain_into_ring(child.stderr.take().unwrap(), 64 * 1024)); // MUST drain, or the child blocks on a full pipe
let mut out = BufReader::with_capacity(256 * 1024, child.stdout.take().unwrap());
let mut buf = Vec::with_capacity(64 * 1024);
loop {
    match read_record(&mut out, &mut buf, 64 << 20).await? {
        Line::Eof => break,
        Line::TooLong(n) => adapter.warn(format!("dropped {n}-byte record")),
        Line::Complete if buf.is_empty() => {}
        Line::Complete => match serde_json::from_slice::<serde_json::Value>(&buf) {
            Ok(v) => adapter.on_json(v),
            Err(_) => adapter.on_text(&buf),        // CLIs print banners/warnings on stdout too
        },
    }
}
```

- `process_group`, `kill_on_drop`, `env_clear` and `envs` all exist on `tokio::process::Command` [V-src: `tokio/src/process/mod.rs`].
- Graceful shutdown: `nix::sys::signal::killpg(Pid::from_raw(pgid), Signal::SIGTERM)`, then wait at most about 3 s, then `SIGKILL`.
- Run shutdown for all agents from `RunEvent::Exit` with `tauri::async_runtime::block_on(core.shutdown(..))`.

### 5.3 `portable-pty` → xterm.js

- The `MasterPty` trait has `resize(PtySize)`, `get_size()`, `try_clone_reader() -> Box<dyn Read + Send>`, `take_writer()` (only once), `process_group_leader()` and `as_raw_fd()` [V-docs: docs.rs portable-pty 0.9.0].
- It is **blocking I/O**, so use one std thread per PTY reader.

```rust
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tauri::ipc::{Channel, InvokeResponseBody};

let pair = native_pty_system().openpty(PtySize { rows: 30, cols: 120, pixel_width: 0, pixel_height: 0 })?;
let mut cmd = CommandBuilder::new(&shell_abs_path);
cmd.cwd(&worktree);
for (k, v) in &env { cmd.env(k, v); }
cmd.env("TERM", "xterm-256color");
let child = pair.slave.spawn_command(cmd)?;
drop(pair.slave);                          // so the reader sees EOF when the child exits
let mut reader = pair.master.try_clone_reader()?;
let writer = pair.master.take_writer()?;   // keep it in the PTY session for pty_write
let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
std::thread::spawn(move || {
    let mut b = [0u8; 16 * 1024];
    while let Ok(n) = reader.read(&mut b) { if n == 0 || tx.blocking_send(b[..n].to_vec()).is_err() { break } }
});
// async side: coalesce ≤16 ms or ≤64 KiB (focused terminal), keep a scrollback ring for replay,
// then: on_data.send(InvokeResponseBody::Raw(chunk))?;    // ArrayBuffer in JS [V-src]
```

```ts
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebglAddon } from '@xterm/addon-webgl';
import '@xterm/xterm/css/xterm.css';                              // path verified in the 6.0.0 tarball

const term = new Terminal({ scrollback: 10_000, cursorBlink: false, fontFamily: 'Menlo, monospace', fontSize: 12 });
const fit = new FitAddon(); term.loadAddon(fit); term.open(el);
try { const gl = new WebglAddon(); gl.onContextLoss(() => gl.dispose()); term.loadAddon(gl); } catch { /* DOM renderer */ }
fit.fit();
const ch = new Channel<ArrayBuffer>((buf) => term.write(new Uint8Array(buf)));   // write(string | Uint8Array) [V-src typings]
await invoke('pty_attach', { ptyId, onData: ch });
term.onData((data) => invoke('pty_write', { ptyId, data }));
new ResizeObserver(() => { fit.fit(); invoke('pty_resize', { ptyId, cols: term.cols, rows: term.rows }); }).observe(el);
```

- **xterm 6.0.0** shipped 2025-12-22 together with addons webgl 0.19.0, fit 0.11.0, search 0.16.0 and web-links 0.12.0 [V-reg].
- Breaking changes in 6.0: the **canvas renderer was removed** (use DOM or WebGL); `overviewRulerWidth` moved to `overviewRuler`; the alt→ctrl+arrow hack was removed; the viewport and scrollbar were rewritten.
- New in 6.0: synchronized output (DEC 2026), OSC 52, ESM build [V-src: GitHub release notes].
- Measured sizes (min+gzip): `xterm.mjs` 88 KB, `addon-webgl.mjs` 36 KB [V-local].

### 5.4 SQLite: rusqlite vs sqlx

**Decision: `rusqlite = { version = "=0.40.2", features = ["bundled"] }` + `rusqlite_migration = "2.6.0"`.**

- **They cannot coexist.** Both link `sqlite3` through `libsqlite3-sys`. rusqlite 0.41 → `libsqlite3-sys ^0.39`; rusqlite 0.40.2 → `^0.38`; sqlx-sqlite 0.9 → `<0.38` [V-reg]. Pick one. This also excludes `tauri-plugin-sql`, which uses sqlx.
- **Pin 0.40.x for now.** rusqlite 0.41.0 came out yesterday. `rusqlite_migration` 2.6, `tokio-rusqlite` 0.8, `r2d2_sqlite` 0.35, `deadpool-sqlite` 0.14 and `async-sqlite` 0.6 all require `^0.40`, and `refinery-core` requires `<=0.40` [V-reg]. Move to 0.41 when they do.
- The bundled build enables **FTS5**, JSON1, RTREE, the session extension and `DEFAULT_FOREIGN_KEYS=1` [V-src: `libsqlite3-sys/build.rs` on master]. libsqlite3-sys master bundles **SQLite 3.54.1**; rusqlite 0.40.1 bundled 3.53.2 [V-src: release notes].
- Why rusqlite: Codly's database is a write-heavy local event log. The simplest fast design is **one writer thread** that batches inserts into a transaction every 50–100 ms, plus 2–4 read connections. sqlx gives async ergonomics and compile-time-checked queries (offline `.sqlx` cache). It costs more compile time, and its SQLite driver runs a worker thread per connection anyway [Claim based on sqlx design]. Codex (OpenAI) uses sqlx 0.9; that is a valid choice, but not needed here.
- Pragmas at open: `journal_mode=WAL; synchronous=NORMAL; busy_timeout=5000; foreign_keys=ON; temp_store=MEMORY; mmap_size=268435456`. Run `wal_checkpoint(TRUNCATE)` when the app goes idle.

### 5.5 File watching

```rust
use notify_debouncer_full::{new_debouncer, notify::RecursiveMode, DebounceEventResult};
let mut deb = new_debouncer(std::time::Duration::from_millis(300), None, move |res: DebounceEventResult| {
    if let Ok(events) = res { let _ = tx.blocking_send(events); }   // filter target/, node_modules/, .git/ here
})?;
deb.watch(&repo_root, RecursiveMode::Recursive)?;   // FSEvents on macOS; stitches renames via file IDs
```

[V-docs: docs.rs notify-debouncer-full 0.7.0; defaults V-reg]

- FSEvents cannot exclude subtrees in the kernel. Filter in the callback and debounce git-status refreshes by 300–500 ms.
- Keep one watcher per repo root, not one per worktree, when worktrees live under one parent directory.

### 5.6 Git: gix vs git2 vs shelling out

From `gitoxide/crate-status.md` (fetched today) [V-src], gix supports:

- `status`, `commit`, blob and tree diff, `merge-base`, and **creating linked worktrees**.

It does **not** yet support:

- `checkout`/`switch`/`reset` orchestration, `merge`/`cherry-pick`/`rebase`, removing, moving or repairing worktrees, or **client-side hooks**.

git2 (libgit2) can add and prune worktrees and merge, but it **does not run hooks**. It also needs a C build, and its behavior differs subtly from the user's `git` (config, credential helpers, fsmonitor) [Claim, well known].

**Decision: shell out to `git`** for every mutation, using porcelain output with stable flags. Agents run plain `git` inside the worktrees anyway, so Codly then sees exactly what they see.

```
git -C <repo> worktree add -b codly/<task-id> <path> <base-ref>
git -C <repo> worktree remove --force <path> && git -C <repo> worktree prune
git -C <wt> status --porcelain=v2 -z --branch --untracked-files=all
git -C <wt> diff --no-color --no-ext-diff --find-renames -U3 <base>...HEAD     # feed to @pierre/diffs
git -C <wt> diff --no-color --no-ext-diff --numstat -z <base>...HEAD
git -C <repo> merge --no-ff --no-edit codly/<task-id>
```

- Environment for background calls: `GIT_OPTIONAL_LOCKS=0` (status must not take `index.lock` while an agent commits), `GIT_TERMINAL_PROMPT=0`, `LC_ALL=C`.
- Resolve `git` once with `which_in` on the captured PATH. Prefer Homebrew git if present. `/usr/bin/git` is a shim that needs Xcode CLT [Claim].
- Keep worktrees **outside** the main repo tree, so the repo watcher and agents' globbing never see them.
- Consider `gix` later only for read-only hot paths, such as status polling across many worktrees, if CLI spawn cost ever shows up in profiles.

### 5.7 Workspace search

```rust
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{sinks::UTF8, BinaryDetection, SearcherBuilder};
use ignore::{WalkBuilder, WalkState};

let matcher = RegexMatcherBuilder::new().case_smart(true).build(&pattern)?;
WalkBuilder::new(&root).hidden(false).git_ignore(true).threads(8).build_parallel().run(|| {
    let matcher = matcher.clone();
    let tx = tx.clone();
    let mut searcher = SearcherBuilder::new().binary_detection(BinaryDetection::quit(0)).line_number(true).build();
    Box::new(move |entry| {
        let Ok(e) = entry else { return WalkState::Continue };
        if !e.file_type().is_some_and(|t| t.is_file()) { return WalkState::Continue }
        let path = e.path().to_path_buf();
        let _ = searcher.search_path(&matcher, &path, UTF8(|ln, line| {
            Ok(tx.blocking_send((path.clone(), ln, line.to_owned())).is_ok())   // false = stop this file
        }));
        WalkState::Continue
    })
});
```

### 5.8 tree-sitter

Defer it to the "explainable orchestration v2" milestone, when symbol-level context is needed. The core is 0.27.1. Grammar crates depend on `tree-sitter-language` 0.1.9 (`LanguageFn`), so they keep working when the core is bumped [V-reg]. `tree-sitter-typescript` 0.23.2 dates from 2024 but still loads [Claim: verify ABI].

### 5.9 Errors, logging and shared types

- Use `thiserror` 2 in `codly-core` for typed errors, and `anyhow` at the shell and adapter edges.
- `tracing` + `tracing-subscriber` (`EnvFilter`, JSON) + `tracing-appender::rolling::daily(app_log_dir, "codly.log")`.
- Generate TS types for `UiEvent`, `Snapshot` and the command DTOs with **`ts-rs` 12** (`#[derive(TS)] #[ts(export)]`) into `src/lib/gen/`. Hand-write a thin typed `api.ts` around `invoke`.

### 5.10 macOS GUI PATH problem

The problem was verified locally:

```
claude:   /Users/julien/.local/bin/claude
codex:    /Users/julien/.nvm/versions/node/v22.23.3/bin/codex
gemini:   /Users/julien/.nvm/versions/node/v22.23.3/bin/gemini
opencode: /Users/julien/.nvm/versions/node/v22.23.2/bin/opencode   # different nvm version!
```

An app launched from Finder or the Dock gets launchd's minimal PATH, so none of these resolve. Guessing a single nvm version would still miss `opencode`. **You must use the user's real shell PATH.** `zsh -ilc 'env -0'` took **0.28 s** on this machine, and `/usr/bin/env -0` works on macOS [V-local].

`fix-path-env` (tauri-apps/fix-path-env-rs) [V-src]:

- **It is not on crates.io.** It is a git dependency at version `0.0.0`; the last commit was 2025-08-25.
- It runs `$SHELL -ilc "echo -n DELIM; env; echo -n DELIM; exit"` with `DISABLE_AUTO_UPDATE=true` and parses newline-separated output, which breaks on multi-line values.
- It has **no timeout**. A `.zshrc` that auto-attaches tmux, `exec`s another shell or waits for input hangs app startup.
- It writes `std::env::set_var` into the global process environment. That call is `unsafe` in edition 2024 because it is not thread-safe. The crate is edition 2021, which hides that.

**Recommended `shell_env` module** (own code, about 60 lines):

```rust
use std::{collections::HashMap, io::Read, process::{Command, Stdio}, sync::mpsc, time::Duration};
const BEGIN: &[u8] = b"__CODLY_ENV_BEGIN__";
const END: &[u8] = b"__CODLY_ENV_END__";

pub fn capture_login_env(timeout: Duration) -> Option<HashMap<String, String>> {
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".into());
    let script = "printf '__CODLY_ENV_BEGIN__'; /usr/bin/env -0; printf '__CODLY_ENV_END__'";
    let mut child = Command::new(&shell)
        .args(["-i", "-l", "-c", script])
        .env("DISABLE_AUTO_UPDATE", "true")      // oh-my-zsh updater prompt
        .env("ZSH_TMUX_AUTOSTART", "false")      // oh-my-zsh tmux plugin
        .env("CODLY_SHELL_PROBE", "1")           // lets users guard slow rc code
        .current_dir(std::env::var_os("HOME")?)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || { let mut b = Vec::new(); let _ = stdout.read_to_end(&mut b); let _ = tx.send(b); });
    let out = match rx.recv_timeout(timeout) {
        Ok(b) => b,
        Err(_) => { let _ = child.kill(); let _ = child.wait(); return None; }
    };
    let _ = child.wait();
    let start = memchr::memmem::find(&out, BEGIN)? + BEGIN.len();
    let end = start + memchr::memmem::rfind(&out[start..], END)?;
    Some(out[start..end].split(|&b| b == 0).filter_map(|kv| {
        let (k, v) = std::str::from_utf8(kv).ok()?.split_once('=')?;
        Some((k.to_owned(), v.to_owned()))
    }).collect())
}
```

How to use it:

1. Run it **once at startup on a background thread**, with a 5 s timeout, before the first agent spawn. The UI shows "detecting CLIs…".
2. Build an effective PATH: the captured PATH, then fallbacks appended only if missing:
   - `~/.local/bin`, `~/.bun/bin`, `~/.volta/bin`, `~/.cargo/bin`, `~/Library/pnpm`, `~/.npm-global/bin`, `~/.opencode/bin`, `~/.claude/local`
   - every `~/.nvm/versions/node/*/bin`, newest first by semver
   - `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`, `/bin`
3. Resolve each CLI with `which::which_in(name, Some(&path), &cwd)`. Store the **absolute path** in settings, and let the user override it.
4. Pass `envs(&captured)` explicitly to every `Command` and PTY spawn. **Never call `set_var`.**
5. Keep the captured environment in memory only. Never log it, because it may contain API keys.
6. Add a "Re-detect" button for when the user installs a CLI while Codly is running.

---

## 6. Frontend framework

### 6.1 Facts that drive the decision (2026-10-25)

| | SolidJS | Svelte 5 | React 19 + Compiler |
|---|---|---|---|
| Latest stable | `solid-js` 1.9.17 | `svelte` 5.57.2 (5.0 GA 2024-10-19) | `react` 19.3.1; `babel-plugin-react-compiler` 1.0.0 (2025-10-07); `@vitejs/plugin-react` 6.1.2 |
| Next major | **2.0.0-rc.18** (RC since 2026-08-12, near-daily releases). Big API changes: `createResource`, `batch`, `startTransition` and `on`/`createComputed` removed; `createEffect(compute, apply)` split; `For`/`Index` unified; `solid-js/web` → `@solidjs/web`; new OXC compiler and `@solidjs/vite-plugin` 3.0.0-next | none announced | none announced |
| Updates | fine-grained signals | fine-grained signals (runes), compiled | VDOM re-render per component, reduced by auto-memoization |
| Runtime size | ~21 KB gz unminified `solid.js`+`web.js` (minified ~7–10 KB) [V-local, Claim for minified] | small and tree-shaken [Claim ~10–15 KB typical] | `react-dom-client.production.js` = **110 KB gz** (625 KB raw) [V-local] |
| Graph lib | none first-party | `@xyflow/svelte` 1.7.0 | `@xyflow/react` 12.12.0 |
| DnD | `@dnd-kit/solid` 0.5.1 (new arch); `@thisbeyond/solid-dnd` last released 2023 | `svelte-dnd-action` 0.9.80; `@dnd-kit/svelte` 0.5.1 | `@dnd-kit/react` 0.5.1; `@dnd-kit/core` 6.3.1 (2024) |
| Palette / UI kit | `cmdk-solid` 1.2.0 + `@kobalte/core` 0.13.14 | `bits-ui` 2.20.2 (Command, menus, dialogs) | `cmdk` 1.1.1, Radix and others |
| Ecosystem vs next major | Kobalte (`^1.9.8`), TanStack (`^1.3`), dnd-kit (`^1.8`) peer on **1.x** | n/a | n/a |
| Coding-agent fluency | moderate on 1.x, poor on 2.0 | good. Svelte ships an MCP (`@sveltejs/mcp` 0.1.27) [V-reg] | best |

All of the above is [V-reg] unless noted.

### 6.2 Decision: **Svelte 5 + plain Vite, no SvelteKit**

- **Fine-grained updates without hand-tuned memoization.** Thirty batches per second fanning out to the timeline, kanban, graph, inspector and world update only the bound text nodes and attributes. React can reach acceptable performance with external stores, selectors and the Compiler, but that takes discipline across ~10 panels. The hot visuals (xterm, CodeMirror, Canvas world) are imperative in every framework anyway.
- **Stable today.** Solid is mid-transition. Starting on 1.9 means a guaranteed rewrite of effects and stores soon. Starting on the 2.0 RC means the component ecosystem (all peering `solid-js ^1.x`) and coding agents both lag. Revisit Solid only after 2.0 is stable and Kobalte, TanStack and dnd-kit support it.
- **Good enough ecosystem.** Svelte Flow, bits-ui, svelte-dnd-action, virtua and paneforge cover every panel. Everything else is framework-agnostic.
- **No SvelteKit.** The `create-tauri-app` svelte-ts template uses SvelteKit 2 + `adapter-static`, with an SPA fallback and `ssr = false` [V-local]. Codly has no routing or SSR needs, and Kit 3.1.0 / adapter-static 4.0.0 are already out [V-reg], so the template would start out of date. Plain `@sveltejs/vite-plugin-svelte` is smaller and has fewer moving parts.
- **Stay on TypeScript 6.0.3**, not 7.0.2. `svelte-check` 4.7.7 peers `typescript ^5 || ^6` [V-reg].

State patterns for streaming in Svelte 5 (APIs verified in `svelte/types/index.d.ts` 5.57.2):

```ts
// src/lib/state/agents.svelte.ts
import { SvelteMap } from 'svelte/reactivity';
export class AgentView {                        // field-level reactivity
  status = $state<'idle' | 'thinking' | 'tool' | 'waiting' | 'done' | 'error'>('idle');
  tool = $state<string | null>(null);
  costUsd = $state(0);
}
export class Timeline {                         // append-only: mutable array + version signal (no O(n) copies)
  #items: TimelineItem[] = [];
  version = $state(0);
  get items() { this.version; return this.#items; }
  append(xs: TimelineItem[]) { this.#items.push(...xs); this.version++; }
}
export const agents = new SvelteMap<string, AgentView>();
```

Other APIs to use:

- `$state.raw` for large immutable blobs.
- `createSubscriber` (from `svelte/reactivity`) to wrap external sources such as a Channel.
- `$state.snapshot` when sending state over IPC.

### 6.3 Per-panel library choices (versions [V-reg])

| Panel | Choice | Notes |
|---|---|---|
| Terminal | `@xterm/xterm` 6.0.0, `@xterm/addon-webgl` 0.19.0, `@xterm/addon-fit` 0.11.0, `@xterm/addon-search` 0.16.0, `@xterm/addon-web-links` 0.12.0 | Limit live WebGL addons, for example to visible terminals only, because browsers cap active WebGL contexts [Claim: ~16 in WebKit/Chromium]. Hidden terminals: dispose the WebGL addon and keep the `Terminal`. |
| Diff review | **`@pierre/diffs` 1.6.1** (Apache-2.0) | Vanilla exports `FileDiff`, `VirtualizedFileDiff`, `parsePatchFiles`, `parseDiffFromFile` and a `./worker` pool. React is an *optional* peer. Renders `git diff` patches directly, with split or stacked layouts and annotations for accept/reject UI [V-src: tarball]. The project is young [Claim]. Fallback: `@codemirror/merge`. |
| Code viewer / editable merge | CodeMirror 6: `@codemirror/state` 6.7.6, `@codemirror/view` 6.43.15, `@codemirror/language` 6.13.1, `@codemirror/language-data` 6.5.2, `@codemirror/search` 6.7.2, `@codemirror/merge` 6.12.3 | Viewport-virtualized and modular. `MergeView` / `unifiedMergeView`. **Not Monaco**: `monaco-editor` 0.57.0 is 101 MB unpacked, needs worker plumbing and is memory-heavy [V-reg size; memory Claim]. |
| Dependency graph | `@xyflow/svelte` 1.7.0 + `@dagrejs/dagre` 3.1.3 | dagre 3 ESM is **49 KB min / 17 KB gz** [V-local], synchronous, fine for ≤ a few hundred nodes. `elkjs` 0.12.0 is **1.6 MB min / 467 KB gz** [V-local]. Load it only in a Worker, and only if compound or port layouts are needed. |
| Kanban DnD | `svelte-dnd-action` 0.9.80 | Pointer-based (not HTML5 DnD), keyboard accessible. HTML5 DnD in a Tauri webview conflicts with Tauri's native drop handler: you must set `dragDropEnabled: false`, which **loses absolute file paths** for "drop files into chat" [V-src config docs; macOS symptom Claim]. Keep `dragDropEnabled: true` and use pointer-based DnD. |
| Pixel world | **Canvas 2D** | `pixi.js` 8.22.0 is **841 KB min / 237 KB gz** [V-local] and its ticker runs every frame unless you manage it. A tile office with ≤ ~50 sprites is trivial in 2D. pixel-agents (prior art) uses React + Canvas 2D [V-src]. Design in §7.2. |
| Command palette | `bits-ui` 2.20.2 `Command` (+ peer `@internationalized/date` 3.12.4) | cmdk-style scoring (`compute-command-score`) [V-src tarball]. |
| Markdown | `marked` 18.1.1 + `dompurify` 3.4.17 + `shiki` 4.5.0 | Use `shiki/core` + `shiki/engine/javascript` (no WASM, CSP friendly) with per-language dynamic imports. While streaming, re-lex only the tail block and cache finished blocks' HTML. `@shikijs/stream` 4.5.0 handles streaming code blocks. |
| Virtualization | `virtua` 0.53.6 (`virtua/svelte`) | Dynamic heights and reverse (chat) mode. `@tanstack/svelte-virtual` 3.13.48 also works but is store-based. Raw logs → read-only xterm. |
| Split panes | `paneforge` 1.0.2 | |
| Icons | `@lucide/svelte` 1.61.0 | |
| Tests | `vitest` 5.0.5, `@testing-library/svelte` 5.4.2, `@playwright/test` 1.64.1 | |

### 6.4 If the team overrules Svelte

- **React BOM:**
  - `react`/`react-dom` 19.3.1, `@vitejs/plugin-react` 6.1.2 (+ `babel-plugin-react-compiler` 1.0.0 via `@rolldown/plugin-babel`, a peer of the plugin) [V-reg]
  - `zustand` 5.0.16, with selectors per row or node
  - `@xyflow/react` 12.12.0, `cmdk` 1.1.1, `@dnd-kit/react` 0.5.1, `@tanstack/react-virtual` 3.14.22, `@pierre/diffs/react`
  - `create-vite` has a `react-compiler-ts` template [V-local]
- **Solid BOM:** `solid-js` 1.9.17, `vite-plugin-solid` 2.11.14, `@kobalte/core` 0.13.14, `cmdk-solid` 1.2.0, `@dnd-kit/solid` 0.5.1, `@tanstack/solid-virtual` 3.13.49. Plan for the 2.0 migration.

---

## 7. macOS idle CPU and memory

### 7.1 Platform facts

- **WKWebView background policy.** `app.windows[].backgroundThrottling` = `disabled | suspend | throttle` maps to `WKPreferences.inactiveSchedulingPolicy` [V-src: wry 0.57.0 `wkwebview/mod.rs`]. It needs macOS 14+. The default is `suspend`: per Tauri's docs, browsers *"throttle timers and even unload the whole tab… after roughly 5 minutes"* when hidden [V-src: config.rs docs]. **Keep the default.** Codly's source of truth lives in Rust, and the snapshot+seq protocol (§2.4) makes a suspended webview harmless.
- **App Nap affects the Rust process.** Nothing in tauri, tao or wry opts out of App Nap [V-src: grep]. When the window is hidden and agents are running, timers and I/O can be throttled [Claim: Apple behavior]. Hold an activity assertion **only while agents run**:

```rust
#[cfg(target_os = "macos")]
pub struct NapGuard(objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2::runtime::NSObjectProtocol>>);
#[cfg(target_os = "macos")]
impl NapGuard {
    pub fn begin(reason: &str) -> Self {
        use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};   // features: NSProcessInfo, NSString
        Self(NSProcessInfo::processInfo().beginActivityWithOptions_reason(
            NSActivityOptions::UserInitiatedAllowingIdleSystemSleep, &NSString::from_str(reason)))
    }
}
#[cfg(target_os = "macos")]
impl Drop for NapGuard {
    fn drop(&mut self) { unsafe { objc2_foundation::NSProcessInfo::processInfo().endActivity(&self.0) } }
}
```

  The signatures were verified on docs.rs: `beginActivityWithOptions_reason` is safe and `endActivity` is `unsafe`. Verify the effect empirically with Activity Monitor's "App Nap" column and `pmset -g assertions`. Forum reports say the assertion sometimes does not prevent napping [Claim].
- **`rAF` may be capped at 60 fps** in WKWebView on ProMotion displays. vibe-kanban ships `tauri-plugin-macos-fps` to lift the cap [V-src comment; Claim]. Codly does not want more than 60 fps, so ignore this.
- **The WebKit version is the system Safari version.** Set `bundle.macOS.minimumSystemVersion: "14.0"` (also needed for `backgroundThrottling`). Use a Vite `build.target` around `safari17` instead of the docs' `safari13` [Claim: verify the Vite 8 target string].

### 7.2 Rules for Codly

1. **No permanent loops.** Render loops (pixel world, tweens) run only while something changes. Schedule the next frame only if `dirty || anyTweenActive`. Sprite animation runs at 8–12 fps, interpolated movement at ≤30 fps. Stop on `document.hidden`, when the World tab is not visible (`IntersectionObserver`), and when all agents are idle and the last tween has finished.
2. **Pixel world mechanics.** Pre-render static layers (floor, walls, furniture) to an offscreen canvas once. Use `ctx.imageSmoothingEnabled = false`, CSS `image-rendering: pixelated`, an integer zoom, and a backing store of `cssSize × devicePixelRatio` snapped to the tile size. Do **not** copy pixel-agents' continuous `requestAnimationFrame` loop, which re-renders every frame forever [V-src].
3. **No polling, in JS or Rust.** Use FSEvents plus debounce for git and file state, and events for agent state. Background `git status` runs only on change or focus, with `GIT_OPTIONAL_LOCKS=0`.
4. **No infinite CSS animations** (spinners, shimmer). Use a static "working" glyph, or animate only when the element is visible and the window is focused. Respect `prefers-reduced-motion`. Avoid large `backdrop-filter` blurs and too many `will-change` layers.
5. **Terminals.** Use `cursorBlink: false`. Stream unfocused terminals at ≤4 Hz or buffer them in Rust. Keep WebGL only on visible terminals.
6. **Memory.** Keep ring-buffered timelines in the UI (e.g., the last 5k items per agent) and page older items from SQLite. Destroy CodeMirror views on hidden tabs but keep their `EditorState`. Use `content-visibility: auto` on offscreen panels [Claim: Safari 18+].
7. **Rust.** Idle tokio workers park, which is fine. Arm the UI pump only on events (§2.4). Build release with `lto`/`codegen-units=1` (the template already does).
8. **Measure.** Activity Monitor's "Energy Impact" and "Idle Wake Ups", `sudo powermetrics --samplers tasks`, Safari Web Inspector Timelines (right-click → Inspect in dev builds, or `features = ["devtools"]` for release), and `tauri`'s `tracing` feature for IPC spans.

### 7.3 WKWebView and Tauri desktop quirks to handle up front

- **Custom menus must keep the Edit items** (`PredefinedMenuItem::copy/paste/cut/select_all/undo/redo`), or ⌘C/⌘V stop working in inputs [Claim: long-standing macOS and Tauri behavior].
- Set `overscroll-behavior: none` on `html, body` to stop rubber-banding of the app shell.
- Use `spellcheck="false" autocorrect="off" autocapitalize="off"` on prompt and code inputs [Claim: macOS text substitutions].
- Set `backgroundColor` in the window config to avoid a white flash in dark mode.
- For native chrome, use `titleBarStyle: "Overlay"`, `hiddenTitle: true`, `trafficLightPosition` and `data-tauri-drag-region`, which needs `allow-start-dragging` [V-src config keys].
- Disable the default WebKit context menu outside text fields in production.
- `zoomHotkeysEnabled` is off by default [V-src].

---

## 8. Scaffold commands that need no prompts (verified)

### 8.1 Recommended: create-vite (svelte-ts) + `tauri init`

Steps 1–3 were run as written [V-local]. Step 4 was **not** run, because I did not compile or launch anything.

```bash
# 1) Frontend
pnpm create vite@9.3.1 codly --template svelte-ts --no-interactive --no-immediate
cd codly
pnpm add -E @tauri-apps/api@2.12.4
pnpm add -D -E @tauri-apps/cli@2.12.4

# 2) Tauri shell (non-interactive)
pnpm tauri init --ci -A codly -W Codly -D ../dist -P http://localhost:1420 \
  --before-dev-command "pnpm dev" --before-build-command "pnpm build"

# 3) Plugins. Add single-instance LAST, because `tauri add` prepends each plugin
pnpm tauri add window-state
pnpm tauri add dialog
pnpm tauri add opener
pnpm tauri add notification
pnpm tauri add single-instance

# 4) Run
pnpm tauri dev
```

What it generates [V-local]:

- `create-vite` 9.3.1 creates `index.html`, `src/` (App.svelte, main.ts), `public/`, `svelte.config.js`, `vite.config.ts`, and `tsconfig.json` / `tsconfig.app.json` / `tsconfig.node.json`. It resolved `svelte` 5.57.2, `@sveltejs/vite-plugin-svelte` 7.3.1, `vite` 8.3.5, `typescript` 6.0.3, `svelte-check` 4.7.7 and `@tsconfig/svelte` 5.0.8.
- `tauri init` creates:
  - `src-tauri/{Cargo.toml, build.rs, tauri.conf.json, capabilities/default.json, src/{lib.rs,main.rs}, icons/}`
  - `Cargo.toml` with `edition = "2024"`, `rust-version = "1.90"`, `tauri = "2.12.5"`, `tauri-build = "2.7.4"` and `tauri-plugin-log = "2"` (debug-only in `lib.rs`)
  - `tauri.conf.json` with `"$schema": "../node_modules/@tauri-apps/cli/config.schema.json"`

**Required patches after scaffolding:**

1. `vite.config.ts`: copy the Tauri block from the create-tauri-app template [V-local]:

   ```ts
   const host = process.env.TAURI_DEV_HOST;
   export default defineConfig({
     plugins: [svelte()],
     clearScreen: false,
     envPrefix: ['VITE_', 'TAURI_ENV_*'],
     server: { port: 1420, strictPort: true, host: host || false,
               hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
               watch: { ignored: ['**/src-tauri/**'] } },
     build: { target: 'safari17', minify: !process.env.TAURI_ENV_DEBUG, sourcemap: !!process.env.TAURI_ENV_DEBUG },
   });
   ```

2. `tauri.conf.json`:
   - `tauri init` sets `identifier` to `com.tauri.dev`. Use e.g. `dev.codly.desktop`. **Do not end the identifier in `.app`**: the CLI warns that it *"conflicts with the application bundle extension on macOS"* [V-src: `tauri-cli/src/build.rs`].
   - Set `app.windows[0].label: "main"` explicitly, a CSP, `bundle.macOS.minimumSystemVersion: "14.0"`, and `backgroundColor`.
3. Rename the crate from `app` / `app_lib` to `codly` / `codly_lib`.
4. Capabilities: check `default.json` and `desktop.json` after `tauri add` (§4).

### 8.2 Alternative: `create-tauri-app` (verified)

```bash
pnpm create tauri-app@4.7.5 codly --template solid-ts --manager pnpm --identifier dev.codly.desktop --yes
#   flags [V-src args.rs + V-local --help]: -m/--manager, -t/--template, --identifier, -y/--yes, -f/--force, --tauri-version 1|2
#   templates: vanilla(-ts) vue(-ts) svelte(-ts) react(-ts) solid(-ts) preact(-ts) angular yew leptos sycamore blazor dioxus
```

I ran it for `solid-ts`, `svelte-ts` and `react-ts` [V-local]. The output has the same `src-tauri` layout plus `tauri-plugin-opener` and a sample `greet` command, already configured for port 1420. `solid-ts` resolved `solid-js` 1.9.17, `vite-plugin-solid` 2.11.14 and `vite` 8.3.5. `svelte-ts` uses SvelteKit 2.65 + `adapter-static` 3.0.10. `react-ts` uses React ^19.1 and `@vitejs/plugin-react` ^6.0.2. `pnpm install --ignore-scripts` succeeds, and the Tauri CLI runs without install scripts because it ships as optional platform packages [V-local].

### 8.3 Target repo layout

```
codly/
├─ package.json  pnpm-workspace.yaml(optional)  vite.config.ts  index.html
├─ src/                         # Svelte app
│  ├─ lib/transport/{tauri.ts, ws.ts, mock.ts}   # one interface, three backends (§9)
│  ├─ lib/state/*.svelte.ts     # runes stores fed by UiBatch
│  ├─ lib/gen/                  # ts-rs output (UiEvent, Snapshot, DTOs)
│  ├─ panels/{chat,timeline,kanban,graph,terminal,diff,code,approvals,inspector}/
│  └─ world/                    # Canvas 2D engine (no framework inside the loop)
├─ crates/
│  ├─ codly-core/               # NO tauri dependency: orchestrator, adapters, pty, db, git, fs, search
│  └─ codly-devserver/          # axum 0.8 WS bridge exposing codly-core to a browser (dev/test only)
└─ src-tauri/                   # thin shell: commands + channels → codly-core; capabilities; platform/ (objc)
```

Keeping `codly-core` free of Tauri makes it unit-testable with `cargo test`, reusable by the devserver, and insulated from the v3 migration.

---

## 9. Running and testing without a GUI

- **There is no WKWebView WebDriver on macOS.** The official `tauri-driver` supports only Linux (WebKitWebDriver) and Windows (Edge Driver); macOS is still "Todo" [V-src: `crates/tauri-driver/README.md`].
- The Tauri docs now recommend **`@wdio/tauri-service`**, whose *embedded* provider runs a WebDriver server inside the app and is auto-detected on macOS [V-docs: v2.tauri.app/develop/tests/webdriver].

Recommended layers:

1. **Rust core**: `cargo test` on `codly-core`. Replay recorded agent JSONL fixtures through the adapters and check the normalized events, including oversize lines and invalid UTF-8. For Tauri commands, the `tauri` `test` feature provides `tauri::test::{mock_builder, mock_context, noop_assets, get_ipc_response}` [V-src: `crates/tauri/src/test/mod.rs`].
2. **Frontend in a browser, no Tauri.** Define a `Transport` interface (`invoke`, `subscribe`) with three implementations, selected by `isTauri()` from `@tauri-apps/api/core` [V-src] plus a `VITE_TRANSPORT` env var:
   - `TauriTransport`: Channels.
   - `WsTransport`: talks to `codly-devserver`, the real Rust core over WebSocket.
   - `MockTransport`: replays fixture sessions on a virtual clock. This is deterministic and covers the pixel world too.

   Run `pnpm dev`, then **Playwright 1.64.1** with the `webkit` project, the engine closest to WKWebView [Claim: same engine family, not identical], plus chromium. Use `webServer: { command: 'pnpm vite --port 1420', url: 'http://localhost:1420' }`. This is fully headless and fast. Coding agents can drive it via Playwright MCP.

   Also available: `@tauri-apps/api/mocks` has `mockIPC(cb, { shouldMockEvents: true })`, `mockWindows` and `clearMocks` for Vitest [V-src]. `mockIPC` doesn't model Channels, so the Transport abstraction is the better seam.
3. **Real app E2E on macOS.**
   - Add **`tauri-plugin-wdio-webdriver` 1.5.1** behind a cargo feature (`e2e`), never in release builds. It is a fork of `Choochmeque/tauri-plugin-webdriver` that uses WKWebView native APIs.
   - Drive it with `webdriverio` 10.1.0 + `@wdio/tauri-service` 1.5.1 (`driverProvider: 'embedded'`, `'tauri:options': { application: './src-tauri/target/debug/codly' }`) [V-reg + upstream README].
   - Build the binary with `pnpm tauri build --debug --no-bundle --features e2e` (flags verified with `--help`).
   - macOS needs a logged-in GUI session; GitHub's macOS runners provide one [Claim].
   - Make hidden test windows deterministic with `--config '{"app":{"windows":[{"label":"main","backgroundThrottling":"disabled"}]}}'`. `-c/--config` merges JSON [V-local --help].
   - CrabNebula's `tauri-driver` fork also covers macOS but needs a paid key [V-docs].
4. **`pnpm tauri dev` for humans and agents.** It always opens a window; there is no real headless mode on macOS. Useful flags: `--no-watch`, `--exit-on-panic`, `--no-dev-server-wait`, `-c <json>`, and `-- [runnerArgs] -- [appArgs]` [V-local].

---

## 10. Recommendations for Codly

1. **Tauri 2.12.x, pinned to minor**: `tauri = "2.12.5"`, `@tauri-apps/api = "2.12.4"`, `@tauri-apps/cli = "2.12.4"`. Do not adopt v3 until it is stable. Put all wry/objc code in `src-tauri/src/platform/`.
2. **One Channel per stream, batched in Rust at 30 Hz.** The pump arms its timer only on events, coalesces deltas, and uses the snapshot+`seq` protocol with visibility gating. Use `emit` only for rare global notices. Binary goes through `InvokeResponseBody::Raw`. Big artifacts are fetched on demand.
3. **Every command is `async`** and returns `Result<_, CmdError>`. Declare an **app ACL manifest** in `build.rs` and grant commands per capability. Set a real CSP.
4. **Plugins:** single-instance (registered first), window-state, dialog, opener, notification. **No** shell, store or sql plugins.
5. **`codly-core` without a Tauri dependency.** It owns `tokio::process` agents (bounded JSONL reader, process groups, stderr drain, killpg on exit), `portable-pty` terminals, **`rusqlite` 0.40.2 bundled** with a WAL writer actor, `notify` + debouncer, **`git` CLI** for worktrees, diffs and merges, and `ignore` + `grep-searcher` for search.
6. **PATH:** build the own `shell_env` (`$SHELL -ilc '/usr/bin/env -0'` with a 5 s timeout and markers). Use explicit `envs()` on every spawn, `which_in` with fallbacks including all nvm versions, user-overridable absolute paths, and a "Re-detect" button. Do not add `fix-path-env`.
7. **Frontend: Svelte 5.57 + Vite 8.3 + TS 6.0, no SvelteKit.**
   - xterm 6 + WebGL; `@pierre/diffs` for review and CodeMirror 6 for viewing and merging; Svelte Flow + dagre; svelte-dnd-action.
   - Canvas 2D world that sleeps when nothing changes; bits-ui Command; marked + DOMPurify + Shiki 4 (JS engine, lazy); virtua.
   - Generate TS types with `ts-rs`.
8. **Idle budget:** 0 wakeups per second from Codly code when no agent is running and the window is unfocused. No permanent rAF, no polling, no infinite CSS animations. While agents run, hold an App Nap activity assertion.
9. **Testing:**
   - `cargo test` with replay fixtures.
   - Vite + Playwright (WebKit) through `MockTransport` and `WsTransport` (axum devserver) for headless UI work and for agents building Codly.
   - WDIO embedded WebDriver (feature-gated) for a few real-app smoke tests on macOS.
10. **Re-check in Q1 2027:** Solid 2.0 stable and its ecosystem; Tauri v3 beta; `notify` 9 and `notify-debouncer-full` 0.8 stable; `rusqlite` 0.41 adoption by `rusqlite_migration`; gix worktree remove and merge support; `tauri-specta` 2.0 stable.

---

## 11. Open questions and unverified items

- **Not run:** `cargo build` / `cargo check` of the scaffold, and `pnpm tauri dev`. Dependency resolution (`pnpm install --ignore-scripts`, `cargo generate-lockfile`) and the CLI itself were verified.
- **App Nap:** whether it measurably throttles Tauri's Rust side while hidden with child agents running, and whether `beginActivity` is enough. Measure.
- **HTML5 DnD on macOS** with `dragDropEnabled: true`: the community reports a symptom ("plus icon, no drag") that I did not reproduce. It doesn't matter if we use pointer-based DnD.
- **WebKit active WebGL context cap** (~16): not confirmed for current WebKit.
- **`@pierre/diffs` maturity** (1.x, a startup project): the API was verified from its typings, but I did not evaluate performance on large patches.
- **Vite 8 `build.target: 'safari17'`:** the exact target string under Rolldown/Oxc should be checked.
- Mobile targets and Linux/Windows specifics are out of scope (Codly is macOS-first).

## 12. Sources

- Tauri source (cloned 2026-10-25, `dev` = 2.12.5; `v3` = 3.0.0-alpha.5): https://github.com/tauri-apps/tauri. Files: `crates/tauri/src/ipc/channel.rs`, `crates/tauri/src/event/{mod,listener}.rs`, `crates/tauri/src/webview/mod.rs`, `crates/tauri-utils/src/acl/{capability,build}.rs`, `crates/tauri-utils/src/config.rs`, `crates/tauri-build/src/{lib,acl}.rs`, `crates/tauri/CHANGELOG.md`, `crates/tauri-driver/README.md`, `packages/api/src/{core,mocks}.ts`, `crates/tauri-cli/src/build.rs`
- Releases: https://github.com/tauri-apps/tauri/releases
- Tauri docs: https://v2.tauri.app/develop/calling-frontend/ · https://v2.tauri.app/develop/calling-rust/ · https://v2.tauri.app/start/frontend/vite/ · https://v2.tauri.app/develop/tests/webdriver/
- Plugins: https://github.com/tauri-apps/plugins-workspace (single-instance README, window-state `lib.rs`, notification `desktop.rs`, default permissions)
- create-tauri-app: https://github.com/tauri-apps/create-tauri-app (`src/args.rs`) · fix-path-env-rs: https://github.com/tauri-apps/fix-path-env-rs (`src/lib.rs`)
- wry 0.57.0 `src/wkwebview/mod.rs` (local cargo registry) — inactiveSchedulingPolicy mapping
- tokio-util `codec/lines_codec.rs`, `codec/framed_impl.rs`; tokio `process/mod.rs`: https://github.com/tokio-rs/tokio
- portable-pty docs: https://docs.rs/portable-pty/0.9.0/portable_pty/trait.MasterPty.html · wezterm pty commits: https://github.com/wezterm/wezterm/commits/main/pty
- notify-debouncer-full docs: https://docs.rs/notify-debouncer-full/0.7.0/
- rusqlite releases: https://github.com/rusqlite/rusqlite/releases · `libsqlite3-sys/build.rs`, `bindgen_bundled_version.rs`
- gitoxide status: https://github.com/GitoxideLabs/gitoxide/blob/main/crate-status.md
- objc2-foundation NSProcessInfo: https://docs.rs/objc2-foundation/0.3.2/objc2_foundation/struct.NSProcessInfo.html
- xterm.js 6.0.0 release: https://github.com/xtermjs/xterm.js/releases/tag/6.0.0
- Solid 2.0 RC: https://github.com/solidjs/solid/releases (v2.0.0-rc.0 notes, 2026-08-13)
- WebdriverIO Tauri: https://github.com/webdriverio/desktop-mobile (packages/tauri-plugin-webdriver README) · https://github.com/Choochmeque/tauri-plugin-webdriver
- @pierre/diffs (npm tarball 1.6.1, README/typings) · https://diffs.com
- pixel-agents (prior art, React + Canvas 2D): https://github.com/pablodelucca/pixel-agents (package.json points to github.com/pixel-agents-hq/pixel-agents)
- vibe-kanban Tauri shell (prior art, capabilities and plugin usage): https://github.com/BloopAI/vibe-kanban
- App Nap background: https://developer.apple.com/forums/thread/679178 · https://wiki.lazarus.freepascal.org/macOS_App_Nap
- HTML5 DnD vs Tauri drop handler: https://react-dropzone.js.org/guide/tauri/
- npm registry and crates.io API queried 2026-10-25 for every version number above.
