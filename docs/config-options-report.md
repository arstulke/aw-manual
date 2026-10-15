# Configuration options report

Step 1 of [#15](https://github.com/arstulke/aw-manual/issues/15). This is an
inventory of every configuration option aw-manual has today. It changes no
code. It is the input for step 2, the human review that decides which options
to remove.

The tables below describe the state **before** step 2. The decisions made in
the review are listed in [Step 2: decisions](#step-2-decisions) at the end.

Sources checked: `crates/core/src/config.rs` (parser, defaults, validation),
`crates/core/src/favorites.rs`, `ui/src/settings/Settings.svelte`,
`crates/daemon/src/*`, `crates/cli/src/main.rs`, `crates/core/src/paths.rs`,
`crates/core/src/ipc.rs`, `nix/*.nix`, `schemas/*.json`, `docs/configuration.md`.

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
- **Used**: whether code reads the value at runtime.
- **Req.**: `required` or `optional`. Every option in `config.yaml` has a
  default, so they are all optional. The whole file may be absent.

## 1. Base configuration (`config.yaml`)

### General / server

| Key | UI | Used | Req. | Type | Default | Description |
|---|---|---|---|---|---|---|
| `schema_version` | ✘ | ✔ (validation only) | optional | integer ≤ 1 | (none) | File format version. Values above 1 are rejected. |
| `server.url` | ✔ | ✔ | optional | string, `http://` or `https://` with a host | `http://localhost:5600` | aw-server address. A remote server works too. |
| `server.bucket_id` | ✔ | ✔ | optional | non-empty string without `/`, `{hostname}` placeholder | `aw-manual_{hostname}` | ActivityWatch bucket the entries are written to. |
| `server.bucket_type` | ✘ | ✔ | optional | non-empty string | `manual.activity` | `type` of the bucket when aw-manual creates it. Not in the generated `config.yaml`. |
| `server.heartbeat_interval` | ✔ | ✔ | optional | duration, `5s` … `10m` | `60s` | How often the running activity is sent as a heartbeat. The AW pulsetime is 1.5 × this value. |
| `favorites.dirs` | ✘ | ✔ | optional | list of folder paths (relative to the config folder, `~` = home) | `[favorites.d]` | Favorites folders, loaded in this order. |
| `log.level` | ✔ | ✔ (read once at daemon start, not live) | optional | `error` \| `warn` \| `info` \| `debug` \| `trace` | `info` | Log level of the daemon. `AW_MANUAL_LOG` overrides it. Not in the generated `config.yaml`. |
| `cli.autostart_daemon` | ✔ | ✔ | optional | bool | `true` | The CLI starts the daemon when a command needs it. Not in the generated `config.yaml`. |
| `openaction.toggle_running` | ✘ | ✘ (**never read**) | optional | `stop` \| `none` | `stop` | Placeholder for the planned phase-2 OpenAction (Stream Deck) plugin: what pressing the running favorite's key does. Not in the generated `config.yaml`. |

### Hotkeys

| Key | UI | Used | Req. | Type | Default | Description |
|---|---|---|---|---|---|---|
| `hotkeys.popover` | ✔ | ✔ | optional | hotkey string (`Ctrl+Alt+Space`, `Mouse4` … `Mouse9`); empty or `false` disables | `Ctrl+Alt+Space` | Opens the popover. In scroll mode it also confirms. |
| `hotkeys.stop` | ✔ | ✔ | optional | hotkey string or empty/`false` | `Ctrl+Alt+S` | Stops the running activity. |
| `hotkeys.resume` | ✔ | ✔ | optional | hotkey string or empty/`false` | `Ctrl+Alt+R` | Resumes the last activity. |
| `hotkeys.favorites` | 👁 | ✔ | optional | map: favorite full ID → hotkey | `{}` | Starts one favorite per hotkey. The same hotkey may not be bound twice. |

### Tray

| Key | UI | Used | Req. | Type | Default | Description |
|---|---|---|---|---|---|---|
| `tray.display` | ✔ | ✔ | optional | `icon` \| `elapsed` \| `since` \| `title` \| `template` | `template` | Text next to the tray icon. |
| `tray.template` | ✔ | ✔ | optional | string with `{title}` `{path}` `{elapsed}` `{since}` `{note}` | `{title} · {elapsed}` | Text when `display: template`. |
| `tray.elapsed_format` | ✔ | ✔ (tray, CLI, popover) | optional | `1h 23m` \| `1:23` \| `1h 23m 45s` \| `1:23:45` | `1h 23m` | Duration format. The CLI and the popover use it too, not only the tray. |
| `tray.time_format` | ✔ | ✔ (tray, CLI, popover, notifications) | optional | `24h` \| `12h` | `24h` | Clock format. Also used outside the tray. |
| `tray.color_icon` | ✔ | ✔ | optional | bool | `true` | Tints the active tray icon in the favorite's color. |

### Notifications

| Key | UI | Used | Req. | Type | Default | Description |
|---|---|---|---|---|---|---|
| `notifications.idle_reminder` | ✔ | ✔ | optional | duration > 0 or `false` (`off`) | `15m` | Reminds you within working hours when nothing has been running this long. |
| `notifications.long_running` | ✔ | ✔ | optional | duration > 0 or `false` | `3h` | Notifies once when one activity has been running this long. |
| `notifications.after_restart` | ✔ | ✔ | optional | duration > 0, `false`, or legacy `true` (= default) | `1m` | After a (re-)start, asks what you did if aw-manual was stopped at least this long. |
| `notifications.after_unlock` | ✔ | ✔ | optional | duration > 0 or `false` | `1m` | After unlocking the screen, asks what you did if it was locked at least this long. |
| `notifications.offline` | ✔ | ✔ | optional | bool | `true` | Notifies when entries are buffered because aw-server is unreachable. |
| `notifications.work_hours.days` | ✔ | ✔ | optional | list of `mon` … `sun` (long names allowed) | `[mon, tue, wed, thu, fri]` | Working days. Only the idle reminder uses them. |
| `notifications.work_hours.from` | ✔ | ✔ | optional | quoted time `"HH:MM"`, earlier than `to` | `"08:00"` | Start of working hours (idle reminder only). |
| `notifications.work_hours.to` | ✔ | ✔ | optional | quoted time `"HH:MM"` | `"18:00"` | End of working hours (idle reminder only). |

### Popover scroll mode

| Key | UI | Used | Req. | Type | Default | Description |
|---|---|---|---|---|---|---|
| `popover.scroll.layout` | ✔ | ✔ | optional | `tree` \| `levels` | `tree` | `tree`: everything expanded, scroll through the leaves. `levels`: one folder at a time. |
| `popover.scroll.confirm_with_hotkey` | ✔ | ✔ | optional | bool | `true` | Pressing the popover hotkey again confirms the highlight. |
| `popover.scroll.dwell_confirm` | ✔ | ✔ | optional | bool | `false` | Confirms when the highlight rests on an item for `dwell_time`. |
| `popover.scroll.dwell_time` | ✔ | ✔ | optional | duration > 0 | `2s` | Dwell time for `dwell_confirm`. |
| `popover.scroll.timeout` | ✔ | ✔ | optional | duration > `dwell_time` | `10s` | Closes the popover in scroll mode after this long without input. |
| `popover.scroll.sensitivity` | ✔ | ✔ | optional | number, 0 < x ≤ 100 | `1.0` | Factor for the scroll threshold per step. |
| `popover.scroll.wrap_around` | ✔ | ✔ (scroll **and** keyboard navigation) | optional | bool | `true` | Wraps from the end of the list back to the start. |
| `popover.scroll.open_at_cursor` | ✔ | ✔ (not on Wayland) | optional | bool | `true` | Opens the popover under the mouse pointer. |
| `popover.scroll.start_at` | ✔ | ✔ | optional | `running` \| `first` | `running` | Item highlighted when the popover opens. |
| `popover.scroll.horizontal` | ✔ | ✔ | optional | bool | `false` | Horizontal scroll / wheel tilt: left = back, right = confirm. Not in the generated `config.yaml`. |

## 2. Favorites files (`favorites.d/*.yaml`)

### File level

| Key | UI | Used | Req. | Type | Default | Description |
|---|---|---|---|---|---|---|
| `favorites` | ✔ (through the favorites editor) | ✔ | **required** | mapping (compact notation) or list (explicit notation) | (none) | The favorites in this file. |
| `schema_version` | ✘ | ✔ (validation only) | optional | integer ≤ 1 | (none) | File format version. |
| `generated` | ✘ (respected: such files are read-only in the UI) | ✔ | optional | bool | `false` | Marks a file as written by a script. |
| `parent` | ✘ | ✔ | optional | full ID of an earlier-loaded folder or leaf | top level | Attaches the file's favorites below that node. |

### Favorite node (compact `_key` / explicit `key`)

| Key | UI | Used | Req. | Type | Default | Description |
|---|---|---|---|---|---|---|
| title (compact: the mapping key) / `title` | ✔ | ✔ | **required** | non-empty string, not starting with `_` | (none) | Display name. |
| `_id` / `id` | ✔ | ✔ | compact: optional; explicit: **required** | slug segment (letters, digits, `.` `_` `-`) | compact: slug of the title | Stable ID segment. Full ID = segments joined with `/`. |
| `_color` / `color` | ✔ | ✔ | optional | quoted hex color `"#RGB"` / `"#RRGGBB"` | inherited, else none | Color in the popover and the tray. Inherited downwards. |
| `_icon` / `icon` | ✔ | ✔ | optional | icon name (`briefcase`, `code`, …) or emoji | inherited, else none | Icon. Inherited downwards. |
| `_kind` / `kind` | ✔ | ✔ | optional, leaves only | `activity` \| `status` | `activity` | Type of the entry (status = break, meeting, …). Not inherited. |
| `_ask_note` / `ask_note` | ✔ | ✔ | optional | bool | inherited, else `false` | Asks for a note when started from the popover, tray or a hotkey. |
| children (compact: nested keys or list shorthand) / `children` | ✔ (one node at a time) | ✔ | optional | mapping/list of nodes | none (= leaf) | Child nodes. A node with children is a folder. |

## 3. Options outside the config files

| Key | Source | Used | Req. | Type | Default | Description |
|---|---|---|---|---|---|---|
| "Start aw-manual at login" | UI-only (OS autostart through `tauri-plugin-autostart`) | ✔ | optional | bool | off (on with Nix `autostart`) | Registers the daemon as a login item. Stored by the OS, not in `config.yaml`. |
| Favorites editor "file" | UI-only (per action) | ✔ | optional | file in `favorites.d/` | `00-ui.yaml` | Target file for new top-level favorites. |
| `AW_MANUAL_CONFIG_DIR` | env | ✔ | optional | path | platform config folder + `aw-manual` | Overrides the config folder. |
| `AW_MANUAL_DATA_DIR` | env | ✔ | optional | path | platform local data folder + `aw-manual` | Overrides the folder for the queue, state and logs. |
| `AW_MANUAL_SOCKET` | env | ✔ | optional | socket path / pipe name | `$XDG_RUNTIME_DIR/aw-manual/daemon.sock`, pipe `aw-manual-<user>` | IPC endpoint between the CLI and the daemon. |
| `AW_MANUAL_DAEMON` | env | ✔ | optional | path | daemon next to the CLI, then `PATH` | Daemon binary the CLI autostarts. |
| `AW_MANUAL_LOG` | env | ✔ | optional | `tracing` filter | derived from `log.level` | Overrides `log.level` completely. |
| `--background` / `--headless` / `--check-lock` | CLI flag (daemon) | ✔ | optional | flag | off | Start without the popover / without any UI / print lock detection for debugging. |
| `enable`, `package`, `autostart` | nix (NixOS + HM) | ✔ | optional | bool / package / bool | `false` / flake package / `true` | Installs the package and starts the daemon with the graphical session. |
| `settings` | nix (HM) | ✔ | optional | attrs → `config.yaml` | `{}` | Manages `config.yaml` (the file is then read-only for the UI). |
| `favorites` | nix (HM) | ✔ | optional | attrs of files → `favorites.d/<name>.yaml` | `{}` | Manages favorites files. |

## 4. Observations for the review

These are facts that may help the decision in step 2. They are not decisions.

1. **Dead option:** `openaction.toggle_running` is parsed and validated but
   no code reads it. It only exists for the planned phase-2 plugin.
2. **Hidden options** (not in the UI and not in the generated `config.yaml`):
   `server.bucket_type`, `favorites.dirs`, `openaction.toggle_running`. Plus
   `log.level`, `cli.autostart_daemon` and `popover.scroll.horizontal`, which
   the UI shows but the generated file leaves out.
3. **Options whose name doesn't match their scope:**
   `tray.elapsed_format` is also used by the CLI and the popover.
   `tray.time_format` is also used by the CLI, the popover and notifications. `popover.scroll.wrap_around` also affects keyboard
   navigation.
4. **Options that only matter together:** `dwell_time` only matters with
   `dwell_confirm: true`. `tray.template` only matters with
   `display: template`. `notifications.work_hours.*` only matters while
   `idle_reminder` is enabled.
5. **Overlapping ways to disable a feature:** `tray.display: template` with
   a custom template can express `elapsed`, `since` and `title` too
   (`{elapsed}`, `since {since}`, `{title}`). `display: icon` is the same as an
   empty template.
6. **Fine-tuning knobs with little expected use:** `server.heartbeat_interval`,
   `popover.scroll.sensitivity`, `popover.scroll.timeout`,
   `server.bucket_type`, `schema_version` (no other version exists yet).
7. **Legacy value:** `notifications.after_restart: true` is still accepted
   for older configurations.
8. **Not live:** `log.level` is only read when the daemon starts.
9. **Documentation drift:** `docs/configuration.md` gives the
   `popover.scroll.sensitivity` range as "greater than 0". The parser also
   requires ≤ 100.

## Counts

| Group | Options |
|---|---|
| `config.yaml` | 36 (including `schema_version`) |
| Favorites, file level | 4 |
| Favorites, per node | 7 |
| UI-only | 2 |
| Environment variables | 5 |
| Daemon flags | 3 |
| Nix module options | 6 |

## Step 2: decisions

| Option | Decision |
|---|---|
| `server.bucket_type` | Removed. Always `manual.activity`. |
| `server.heartbeat_interval` | Removed. Always 60 s. |
| `favorites.dirs` | Kept. It can now also be edited in the settings window (General tab). |
| Favorites editor "file" | New option `favorites.default_file` (default `00-ui.yaml`, in the first favorites folder), also editable in the settings window. |
| `tray.elapsed_format`, `tray.time_format` | Renamed to `format.elapsed` and `format.time`, since the tray, popover, CLI and notifications all use them. |
| `notifications.after_restart: true` | The legacy value is no longer accepted. |
| `popover.scroll.layout` | Removed. Always the tree layout; the `levels` code is gone. |
| `popover.scroll.dwell_confirm` | Removed. `popover.scroll.dwell_time` is now a duration or `false` (default `false` = no dwell confirmation). |
| `popover.scroll.timeout` | Removed. Always 10 s, or longer when the dwell time needs it. |
| `popover.scroll.sensitivity` | Removed. Always the base threshold. |
| `popover.scroll.wrap_around` | Removed. Always wraps around. |
| `popover.scroll.horizontal` | Kept. Right = confirm (Enter), left = confirm with a note (Shift+Enter), instead of back. |
| All others | Unchanged. |

Old keys are now reported as unknown settings; there is no migration.

The `config.yaml` created on first start now lists every setting with its
default, including `cli`, `log`, `openaction`, `popover.scroll.horizontal` and
an example for `hotkeys.favorites`.
