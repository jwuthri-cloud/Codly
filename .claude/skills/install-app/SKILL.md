---
name: install-app
description: Build Codly.app and install it in /Applications so the user can try a change in the real desktop app. Use when the user says "rebuild Codly.app", "install the app", "let me try it in the app", or asks for a release build. Explains the fast development build vs the real release build (LTO).
---

# Build and install Codly.app

The user tries changes in `/Applications/Codly.app`, not in the browser demo (the desktop app
differs: real core, real repos, the app's CSP). So a "rebuild" always ends with that copy
replaced and reopened.

## Which build

| | Command | Time | When |
|---|---|---|---|
| Development | `scripts/install-app.sh` | first one a few minutes, then well under a minute | Every "rebuild", "install it", "let me try it" |
| Real release | `scripts/install-app.sh --release` | ~3 min, every time | Only when the user asks for a real release (to ship or share) |

Why: `Cargo.toml`'s `[profile.release]` has `lto = true` and `codegen-units = 1`. They make the
shipped binary small and fast but redo a whole-program optimization on every build, which is
most of the 3 minutes. The development build is the same release build with those turned off
through Cargo's environment overrides (`CARGO_PROFILE_RELEASE_LTO=false`, 256 codegen units,
incremental), so `Cargo.toml` stays as it is for real releases.

Don't use `tauri build --debug` for this: a debug bundle still loads the frontend from the Vite
dev server (`http://localhost:1420`), so it can show stale or dev-only behavior.

## Steps

1. Pick what goes in. The script builds **this checkout's working tree**. Other sessions may
   have uncommitted edits here (`git status`): if the user asked for "main" (or only your work),
   build from the commit instead:
   ```bash
   # from the repo root
   R=$PWD; B=<scratchpad>/main-build; rm -rf $B && mkdir -p $B && git archive HEAD | tar -x -C $B
   ln -s "$R/node_modules" $B/node_modules && cp scripts/install-app.sh $B/scripts/
   CARGO_TARGET_DIR="$R/target" $B/scripts/install-app.sh
   ```
   With `CARGO_TARGET_DIR` set, the build reuses this repo's compile cache and the script
   installs the bundle from there.
2. Run the script in the background (it takes minutes) and wait for it to finish.
3. Check that it worked: the script exits non-zero if the build fails; the last line shows
   `/Applications/Codly.app/Contents/MacOS/codly` with the new time. Never report done on an
   old timestamp.

## For quick iteration instead

`pnpm tauri dev` opens the real desktop window (real core and data) with instant frontend
reload and fast Rust rebuilds. Quit the installed Codly first: both share the same database
(`~/Library/Application Support/dev.codly.desktop`), and two cores would both run agents.
