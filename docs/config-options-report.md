# Configuration options report

A fresh inventory of every configuration option aw-manual has after the
reduction in [#15](https://github.com/arstulke/aw-manual/issues/15). It was
built again from the code, not from the earlier report (that version is in the
git history).

Sources checked: `crates/core/src/config.rs` (parser, defaults, validation,
generated `config.yaml`), `crates/core/src/favorites.rs`,
`crates/core/src/{service,display,reminders,paths,ipc}.rs`,
`crates/daemon/src/{app,commands,hotkeys,popover,settings,tray,notifications,main}.rs`,
`crates/cli/src/main.rs`, `ui/src/settings/Settings.svelte`,
`ui/src/popover/Popover.svelte`, `nix/*.nix`, `schemas/config.schema.json`.

## Legend

- **Source**
  - `config` = `config.yaml`
  - `fav-file` = top-level key of a favorites file
  - `fav-node` = property of a single favorite (compact `_x` / explicit `x`)
  - `UI-only` = can only be set in the settings window, not in any file
  - `env` = environment variable
  - `CLI flag` = command-line flag of `aw-manual-daemon`
  - `nix` = option of the NixOS / Home Manager module
- **UI**: whether the settings window can edit the option (✔ = editable,
  👁 = shown read-only, ✘ = not shown).
- **Used**: whether code reads the value at runtime, and where.
- **Live**: whether a change applies without restarting the daemon.
- **Req.**: `required` or `optional`. Every option in `config.yaml` has a
  default, so they are all optional. The whole file may be absent.

## 1. Base configuration (`config.yaml`)

All 30 options below appear in the `config.yaml` that the daemon writes on its
first start, with their defaults.

### General / server

| Key | UI | Used | Live | Req. | Type | Default | Description |
|---|---|---|---|---|---|---|---|
| `schema_version` | ✘ | ✔ (validation only) | – | optional | integer, 1 | (none; `1` in the generated file) | File format version. Values other than 1 are rejected. |
| `server.url` | ✔ | ✔ (writer, status) | ✔ | optional | string starting with `http://` or `https://`, with a host | `http://localhost:5600` | aw-server address. A remote server works too. |
| `server.bucket_id` | ✔ | ✔ (writer, engine) | ✔ | optional | non-empty string without `/`; `{hostname}` placeholder | `aw-manual_{hostname}` | ActivityWatch bucket the entries are written to. |
| `favorites.dirs` | ✔ (one folder per line) | ✔ (loader, file watcher, `config check`) | ✔ | optional | list of non-empty paths; relative to the config folder, `~` = home | `[favorites.d]` | Favorites folders, loaded in this order. |
| `favorites.default_file` | ✔ | ✔ (settings window) | ✔ | optional | file name ending in `.yaml`/`.yml`, no `/` or `\`, not hidden, not `*~`/`*.tmp` | `00-ui.yaml` | File in the first favorites folder that receives new top-level favorites from the settings window. |
| `cli.autostart_daemon` | ✔ | ✔ (CLI) | ✔ (read on every CLI call) | optional | bool | `true` | The CLI starts the daemon when a command needs it. |
| `log.level` | ✔ | ✔ (daemon logging) | ✘ (read once at daemon start) | optional | `error` \| `warn` \| `info` \| `debug` \| `trace` | `info` | Log level of the daemon. `AW_MANUAL_LOG` replaces it completely. |
| `openaction.toggle_running` | ✘ | ✘ (reserved for phase 2) | – | optional | `stop` \| `none` | `stop` | What pressing the running favorite's key on the OpenAction (Stream Deck) plugin does. Parsed and validated; the plugin doesn't exist yet. |

### Hotkeys

| Key | UI | Used | Live | Req. | Type | Default | Description |
|---|---|---|---|---|---|---|---|
| `hotkeys.popover` | ✔ | ✔ | ✔ (re-registered on reload) | optional | hotkey (`Ctrl+Alt+Space`, `F13`, `Mouse4` … `Mouse9`); empty, `null` or `false` disables | `Ctrl+Alt+Space` | Opens the popover. While it is open in scroll mode, it confirms (or closes it, with `confirm_with_hotkey: false`). |
| `hotkeys.stop` | ✔ | ✔ | ✔ | optional | hotkey or empty/`false` | `Ctrl+Alt+S` | Stops the running activity. |
| `hotkeys.resume` | ✔ | ✔ | ✔ | optional | hotkey or empty/`false` | `Ctrl+Alt+R` | Resumes the last stopped activity. |
| `hotkeys.favorites` | 👁 | ✔ | ✔ | optional | map: favorite full ID → hotkey | `{}` (an empty key with a commented example) | Starts one favorite per hotkey; asks for a note first if the favorite has `ask_note`. A hotkey may be bound only once across all four keys. |

### Tray

| Key | UI | Used | Live | Req. | Type | Default | Description |
|---|---|---|---|---|---|---|---|
| `tray.display` | ✔ | ✔ (tray text) | ✔ | optional | `icon` \| `elapsed` \| `since` \| `title` \| `template` | `template` | Text next to the tray icon. No effect on Windows, which shows no text there. |
| `tray.template` | ✔ (only while `display: template`) | ✔ | ✔ | optional | string with `{title}` `{path}` `{elapsed}` `{since}` `{note}`; unknown placeholders are errors | `{title} · {elapsed}` | Text when `display: template`. |
| `tray.color_icon` | ✔ | ✔ | ✔ | optional | bool | `true` | Tints the active tray icon in the running favorite's color. |

### Format

| Key | UI | Used | Live | Req. | Type | Default | Description |
|---|---|---|---|---|---|---|---|
| `format.elapsed` | ✔ (General) | ✔ (tray, popover, CLI, retroactive preview) | ✔ | optional | `1h 23m` \| `1:23` \| `1h 23m 45s` \| `1:23:45` | `1h 23m` | How durations are shown. With seconds, the tray text updates every second. |
| `format.time` | ✔ (General) | ✔ (tray, popover, CLI, notifications, retroactive preview) | ✔ | optional | `24h` \| `12h` | `24h` | How clock times are shown. |

### Notifications

| Key | UI | Used | Live | Req. | Type | Default | Description |
|---|---|---|---|---|---|---|---|
| `notifications.idle_reminder` | ✔ | ✔ | ✔ | optional | duration > 0, or `false` / `off` | `15m` | Reminds you, within working hours, when nothing has been running this long. |
| `notifications.long_running` | ✔ | ✔ | ✔ | optional | duration > 0, or `false` / `off` | `3h` | Notifies once when one activity has been running this long. |
| `notifications.after_restart` | ✔ | ✔ | (only checked at daemon start) | optional | duration > 0, or `false` / `off` | `1m` | After a (re-)start, asks what you did meanwhile if aw-manual was stopped at least this long. |
| `notifications.after_unlock` | ✔ | ✔ | ✔ | optional | duration > 0, or `false` / `off` | `1m` | After unlocking the screen, asks what you did meanwhile if it was locked at least this long. |
| `notifications.offline` | ✔ | ✔ | ✔ | optional | bool | `true` | Notifies when entries are buffered because aw-server is unreachable. |
| `notifications.work_hours.days` | ✔ | ✔ (idle reminder only) | ✔ | optional | list of `mon` … `sun` (long names allowed, no duplicates); empty list or `null` = no days | `[mon, tue, wed, thu, fri]` | Working days. |
| `notifications.work_hours.from` | ✔ | ✔ (idle reminder only) | ✔ | optional | quoted `"HH:MM"`, earlier than `to` | `"08:00"` | Start of working hours. |
| `notifications.work_hours.to` | ✔ | ✔ (idle reminder only) | ✔ | optional | quoted `"HH:MM"` | `"18:00"` | End of working hours. |

### Popover scroll mode

| Key | UI | Used | Live | Req. | Type | Default | Description |
|---|---|---|---|---|---|---|---|
| `popover.scroll.confirm_with_hotkey` | ✔ | ✔ (daemon) | ✔ | optional | bool | `true` | Pressing the popover hotkey again confirms the highlighted item. With `false` it closes the popover. |
| `popover.scroll.dwell_time` | ✔ (checkbox + duration) | ✔ (popover) | ✔ (next time the popover opens) | optional | duration > 0, or `false` / `off` | `false` | Confirms when an item stays highlighted this long. Scroll mode closes after 10 s without input, or dwell time + 2 s if that is longer. |
| `popover.scroll.open_at_cursor` | ✔ | ✔ (daemon; ignored on Wayland) | ✔ | optional | bool | `true` | Opens the popover under the mouse pointer. |
| `popover.scroll.start_at` | ✔ | ✔ (popover) | ✔ | optional | `running` \| `first` | `running` | Item highlighted when the popover opens. |
| `popover.scroll.horizontal` | ✔ | ✔ (popover, scroll mode only) | ✔ | optional | bool | `false` | Horizontal scrolling / wheel tilt: right = confirm (Enter), left = confirm with a note (Shift+Enter). |

## 2. Favorites files (`favorites.d/*.yaml`)

Loaded: files ending in `.yaml`/`.yml` directly in each folder of
`favorites.dirs`; hidden files, `*~`, `*.tmp` and subfolders are skipped.

### File level

| Key | UI | Used | Req. | Type | Default | Description |
|---|---|---|---|---|---|---|
| `favorites` | ✔ (through the favorites editor) | ✔ | **required** (a completely empty file is allowed) | mapping (compact notation) or list (explicit notation); empty = no favorites | (none) | The favorites in this file. |
| `schema_version` | ✘ | ✔ (validation only) | optional | integer, 1 | (none) | File format version. |
| `generated` | ✘ (respected: such files are read-only in the UI) | ✔ | optional | bool | `false` | Marks a file as written by a script. The Home Manager module sets it on the files it writes. |
| `parent` | ✘ | ✔ | optional | full ID of a folder or leaf defined in an earlier-loaded file | top level | Attaches the file's favorites below that node. A leaf used as `parent` becomes a folder. |

### Favorite node (compact `_key` / explicit `key`)

| Key | UI | Used | Req. | Type | Default | Description |
|---|---|---|---|---|---|---|
| title (compact: the mapping key) / `title` | ✔ | ✔ | **required** | non-empty string, not starting with `_`; unique in its folder | (none) | Display name. |
| `_id` / `id` | ✔ | ✔ | compact: optional; explicit: **required** | segment: letters, digits, `.` `_` `-`, starting with a letter or digit | compact: slug of the title | Stable ID segment. The full ID joins the segments with `/`. |
| `_color` / `color` | ✔ | ✔ | optional | quoted hex color `"#RGB"` / `"#RRGGBB"` | inherited, else none | Color in the popover and the tray icon. Inherited downwards. |
| `_icon` / `icon` | ✔ | ✔ | optional | icon name (`briefcase`, `code`, …) or an emoji | inherited, else none | Icon. Inherited downwards. |
| `_kind` / `kind` | ✔ | ✔ | optional, leaves only | `activity` \| `status` | `activity` | Type of the entry (status = break, meeting, …). Not inherited. |
| `_ask_note` / `ask_note` | ✔ | ✔ | optional | bool | inherited, else `false` | Asks for a note when started from the popover, the tray menu or a hotkey (never from the CLI). |
| children (compact: nested keys or the list shorthand `[A, B]`) / `children` | ✔ (one node at a time) | ✔ | optional | mapping or list of nodes | none (= leaf) | Child nodes. A node with children is a folder; only leaves can be started. |

## 3. Options outside the config files

| Key | Source | Used | Req. | Type | Default | Description |
|---|---|---|---|---|---|---|
| "Start aw-manual at login" | UI-only (OS login item through `tauri-plugin-autostart`) | ✔ | optional | bool | off | Registers the daemon to start at login. Stored by the OS, not in any file. |
| "File" when adding a top-level favorite | UI-only (per action) | ✔ | optional | one of the writable favorites files | `favorites.default_file` | Overrides `favorites.default_file` for a single new favorite. Not persisted. |
| `AW_MANUAL_CONFIG_DIR` | env | ✔ | optional | path | platform config folder + `aw-manual` | Overrides the config folder. |
| `AW_MANUAL_DATA_DIR` | env | ✔ | optional | path | platform local data folder + `aw-manual` | Overrides the folder for the offline queue, state and logs. |
| `AW_MANUAL_SOCKET` | env | ✔ | optional | socket path (Unix) / pipe name (Windows) | `$XDG_RUNTIME_DIR/aw-manual/daemon.sock` (or a temp folder), pipe `aw-manual-<user>` | IPC endpoint between the CLI and the daemon. |
| `AW_MANUAL_DAEMON` | env | ✔ | optional | path | daemon next to the CLI, then on `PATH` | Daemon binary the CLI starts with `cli.autostart_daemon`. |
| `AW_MANUAL_LOG` | env | ✔ | optional | `tracing` filter, e.g. `debug` | derived from `log.level` | Replaces `log.level` completely. |
| `--background` | CLI flag (daemon) | ✔ | optional | flag | off | Start without showing the popover (used by autostart, the CLI and the Nix services). |
| `--headless` | CLI flag (daemon) | ✔ | optional | flag | off | Run without tray, popover and hotkeys (IPC, tracking and writing only). |
| `--check-lock` | CLI flag (daemon) | ✔ | optional | flag | off | Print the screen lock detection live, for debugging. |
| `programs.aw-manual.{enable,package,autostart}` | nix (NixOS) | ✔ | optional | bool / package / bool | `false` / flake package / `true` | Installs the package; `autostart` adds a systemd user service in the graphical session. |
| `services.aw-manual.{enable,package,autostart}` | nix (Home Manager) | ✔ | optional | bool / package / bool | `false` / flake package / `true` | Same, for Home Manager. |
| `services.aw-manual.settings` | nix (Home Manager) | ✔ | optional | attrs → `config.yaml` | `{}` (file not managed) | Writes `config.yaml` from Nix. Changes from the settings window are not kept. |
| `services.aw-manual.favorites` | nix (Home Manager) | ✔ | optional | attrs of files → `favorites.d/<name>.yaml` | `{}` | Writes favorites files with `generated: true`. |

## 4. Observations for the review

Facts found during this analysis that may matter for further decisions. None
of them are decisions.

1. **Reserved option:** `openaction.toggle_running` is still not read by any
   code. It stays on purpose for the phase-2 OpenAction plugin.
2. **Not editable in the settings window:** `schema_version`,
   `hotkeys.favorites` (shown read-only) and `openaction.toggle_running`.
   Everything else in `config.yaml` (27 of 30 options) is editable there.
3. **Overlapping ways to switch something off:**
   - `notifications.work_hours.days: []` disables the idle reminder just like
     `idle_reminder: false`.
   - `tray.display: template` can express `elapsed`, `since` and `title`
     (`{elapsed}`, `since {since}`, `{title}`); `display: icon` is the same as
     an empty template.
4. **Options that only matter together:** `tray.template` only with
   `display: template`. `notifications.work_hours.*` only while
   `idle_reminder` is enabled. `popover.scroll.dwell_time` and `horizontal`
   only in scroll mode. `confirm_with_hotkey` only with a `hotkeys.popover`
   binding.
5. **Platform limits:** `tray.display` and `tray.template` have no visible
   effect on Windows (no text in the notification area; the tooltip doesn't
   use them). `popover.scroll.open_at_cursor` is ignored on Wayland.
6. **Not live:** `log.level` is only read when the daemon starts.
   `notifications.after_restart` is only evaluated at start by design.
7. **Not checked up front:** the favorite IDs in `hotkeys.favorites` aren't
   checked against the favorites, neither while parsing nor by
   `aw-manual config check`. A wrong ID only shows up as an error when the
   hotkey is pressed.
8. **Edge case with `favorites.dirs: []`:** an empty list is allowed. No
   favorites are loaded then, but the settings window still writes new
   top-level favorites to `favorites.d/<default_file>`, which isn't loaded, so
   they don't appear.
9. **Edge case with `favorites.default_file`:** if that file has a `parent`,
   the settings window computes the new favorite's ID without the parent (for
   the duplicate check and the returned ID), while the loader attaches it
   below the parent. Picking the same file explicitly in the "File" dropdown
   handles `parent` correctly.
10. **CLI with an invalid `config.yaml`:** the CLI then silently uses the
    defaults for `format.*` and `cli.autostart_daemon`; the daemon keeps the
    last valid configuration instead.

## Counts

| Group | Options |
|---|---|
| `config.yaml` | 30 (including `schema_version`); 27 editable in the settings window |
| Favorites, file level | 4 |
| Favorites, per node | 7 |
| UI-only | 2 |
| Environment variables | 5 |
| Daemon flags | 3 |
| Nix module options | 6 (`enable`, `package`, `autostart` in both modules; `settings`, `favorites` in Home Manager only) |
