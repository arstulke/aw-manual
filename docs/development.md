# Development

## Layout

```text
crates/core     aw-manual-core: config & favorites (YAML with line numbers),
                comment-preserving YAML edits, time parsing, state machine
                (engine), SQLite offline queue, aw-server client & writer,
                reminders, IPC protocol, service (daemon core)
crates/cli      aw-manual: the CLI (sync IPC client, no GUI dependencies)
crates/daemon   aw-manual-daemon: Tauri v2 host (tray, popover and settings
                windows, hotkeys incl. mouse buttons and the Wayland portal,
                notifications, file watcher, settings backend)
ui              Svelte 5 + Vite: popover and settings pages
schemas         JSON schemas of config.yaml and favorites files
nix             Nix package, NixOS and Home Manager modules
scripts         build helpers (CLI sidecar for installers)
```

`cargo build` / `cargo test` at the root build the core and the CLI only
(`default-members`); the daemon needs the built UI:

```sh
(cd ui && npm ci && npm run build)
cargo build -p aw-manual-daemon
```

Linux needs `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev`.
`nix develop` provides everything.

The daemon serves the built UI from `ui/dist` (feature `custom-protocol`, on by
default). For UI development with hot reload, run `npm run dev` in `ui` and
start the daemon with `cargo run -p aw-manual-daemon --no-default-features`; it
then loads the UI from the Vite dev server (`http://localhost:5174`).

## Tests

```sh
cargo test --workspace                     # unit and integration tests
(cd ui && npm test && npm run check)       # UI logic (vitest) and type check
```

Integration tests against real aw-servers run when `AW_TEST_SERVERS` lists
their URLs (the CI starts aw-server-rust and aw-server-python from the
ActivityWatch release):

```sh
aw-server-rust --testing --port 5701 &
aw-server --testing --port 5702 &
AW_TEST_SERVERS=http://localhost:5701,http://localhost:5702 cargo test -p aw-manual-core --test aw_server
```

For manual testing without touching your real setup, point aw-manual to other
folders: `AW_MANUAL_CONFIG_DIR`, `AW_MANUAL_DATA_DIR`, `AW_MANUAL_SOCKET`.
`cargo run -p aw-manual-core --example headless` runs a daemon without UI.

## Packaging

```sh
node scripts/prepare-sidecar.mjs            # builds the CLI as Tauri sidecar
cd crates/daemon
../../ui/node_modules/.bin/tauri build --config tauri.release.conf.json --bundles deb
```

- macOS: `--target universal-apple-darwin` (run `prepare-sidecar.mjs --target
  universal-apple-darwin` first). Signing and notarization use the standard
  Tauri environment variables (`APPLE_CERTIFICATE`, `APPLE_SIGNING_IDENTITY`,
  `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`).
- Windows: `--bundles msi,nsis`. Both installers put the CLI on `PATH`
  (`crates/daemon/windows/`). Code signing: `bundle.windows.signCommand`.
- Nix: `nix build`.

The GitHub workflow builds all of them and attaches them to a draft release
for `v*` tags.

## Design notes

- The daemon is the only component that talks to aw-server and holds state.
  CLI, popover, tray and hotkeys are clients. IPC: newline-delimited JSON over
  a Unix socket / named pipe (see [CLI reference](cli.md#ipc)).
- State changes and the operations they cause are committed to SQLite in one
  transaction; a writer task replays the queue to aw-server in order with
  exponential backoff.
- The first heartbeat of a run has `pulsetime = 0` and a duration of 1 ms, and
  timestamps never go backwards. This keeps the running event the latest and
  last-ending event in the bucket, which both aw-server implementations rely
  on when merging heartbeats. Retroactive entries that overlap the running
  activity reset the server's heartbeat cache.
