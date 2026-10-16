# Spec: Manual Activity Tracker for ActivityWatch

> Status: v1.0 · implemented in aw-manual 0.1 (MVP) · Date: 2026-10-16
>
> This document describes the intended behavior. The user documentation
> ([README](../README.md), [docs](.)) describes the released state.

## 1. Goal and Context

ActivityWatch (AW) automatically records *what* happens on the computer (windows, AFK, browser …). What's missing is the human component: *what* the user is actually working on (client, project, task). aw-manual extends AW with **fast, manual tracking** through multiple input methods and writes the data into its own AW bucket.

**Principle:** aw-manual only records. Interpretation, reconciliation with other watchers and analysis happen **downstream** via AW plugins, connectors or queries. aw-manual therefore never pauses, trims or corrects activities automatically (e.g. not based on AFK data).

### Non-Goals (MVP)
- No built-in reports, analytics or invoicing
- No synchronization between computers (each host has its own bucket)
- No dependency on `aw-watcher-afk` or other watchers
- No localization (UI is English only)
- No timeline view or visual editing of entries (click-drag); possibly a later extension

## 2. Target Platforms

| Platform | Desktop / Environment | Notes |
|---|---|---|
| Linux | GNOME (Wayland/X11) | Tray requires the AppIndicator extension; hotkeys via GlobalShortcuts portal or system binding → CLI |
| Linux | KDE Plasma | Native tray (StatusNotifierItem); hotkeys via portal/KDE shortcuts → CLI |
| Linux | Hyprland | Tray via a bar (e.g. Waybar); hotkeys via `bind` in `hyprland.conf` → CLI |
| macOS | Current and previous version | Menu bar icon, native global hotkeys |
| Windows | 10/11 | Notification area icon, native global hotkeys |

**Performance targets** (guidelines, to be verified during the project):
- Daemon at idle: < 30 MB RAM, ~0 % CPU
- Popover visible and ready for input < 150 ms after hotkey/click (window is kept alive, not recreated)
- CLI command including response < 50 ms

## 3. Architecture

```
  Hotkey ─────┐       ┌──────────────────────────────┐
  Scroll wheel┤       │       aw-manual daemon        │      ┌───────────────┐
  Tray ───────┼─────▶ │  (Rust, Tauri host process)   │ ───▶ │ aw-server     │
  CLI ────────┤  IPC  │  • State (running activity)   │ HTTP │ (local or     │
              │       │  • Config (YAML)              │      │  remote)      │
  OpenAction ─┘       │  • Offline queue (SQLite)     │      └───────────────┘
  plugin (Phase 2)    │  • Notifications              │
    ▲                 └──────────────────────────────┘
    │ WebSocket
  OpenAction server (OpenDeck, Tacto, Elgato Stream Deck, Mirabox Stream Dock …)
    ▲
  Hardware (Stream Deck, Stream Deck +, smartphone, other supported devices)
```

- **Stack:** Rust + Tauri v2. Popover as a web UI (lightweight, e.g. Svelte/Solid; Svelte is a good fit since the OpenAction property inspector can be built with it too).
- The **daemon** is the only component that talks to aw-server and holds state. All input methods are clients.
- **IPC:** Unix domain socket (Linux/macOS) or named pipe (Windows), JSON messages. Used by the CLI and the OpenAction plugin.
- **Hardware** is connected exclusively through the **OpenAction API** (see 5.5). aw-manual does not speak HID itself; device drivers, key assignments and profiles belong to the OpenAction server.
- **Single instance:** Launching a second instance brings the running instance's popover to the front.

## 4. Data Model

### 4.1 Activity Types
**At most one activity runs at any time.** Starting a new activity automatically ends the running one.

| Type | Behavior |
|---|---|
| **Timer (start/stop)** | Runs from start until stopped or replaced by another activity |
| **Status/context** | e.g. "Break", "Focus", "Meeting". Technically a timer, semantically marked as a status; also replaces the running activity |
| **Retroactive entry (block)** | Record a time span after the fact (start + end, or duration) |

### 4.2 Favorites
- **Hierarchy** of arbitrary depth, typically: *Client › Project › Task*
- Each node has: `id` (stable, slug), `title`, optional `color`, `icon`, `kind` (`activity` | `status`, leaves only)
- `color` and `icon` are **inherited** down the hierarchy and can be overridden on any level (see 7.3)
- **Only leaves can be started.** Nodes with children are pure folders for organization; they cannot be started or used for retroactive entries. Validation rejects start requests for folder IDs.
- Favorites are defined in a **folder of YAML files** that may be hand-written or generated (see 7.3)
- **Order is fixed and manual** (the order in the config files) – no sorting by usage frequency or recency, so positions (including on OpenAction keys) stay stable

### 4.3 Notes
- Each activity can optionally carry a **free-text note**, set at start (or with a retroactive entry).
- The note of an event that has already been written or is running is **never modified**.
- **New note while an activity is running:** The running activity ends at that moment and the same activity immediately starts again with the new note. AW therefore shows two consecutive events.

### 4.4 ActivityWatch Bucket
- **One bucket per host**, default ID: `aw-manual_<hostname>` (configurable)
- Bucket type: `manual.activity` (configurable)
- `client`: `aw-manual`, `hostname`: current host

Example event:
```json
{
  "timestamp": "2026-10-09T08:30:00Z",
  "duration": 5400,
  "data": {
    "id": "acme/website/dev",
    "title": "ACME › Website › Development",
    "path": ["ACME", "Website", "Development"],
    "kind": "activity",
    "note": "Navigation refactoring",
    "source": "popover",
    "entry": "timer",
    "origin": "config"
  }
}
```
- `source`: `popover` | `popover-scroll` | `tray` | `cli` | `openaction` | `hotkey`
- Optional `device`: for `openaction`, the device name provided by the OpenAction server
- `entry`: `timer` | `retro`
- `origin`: `config` (favorite from the config) | `deck` (Custom Activity or orphaned favorite using snapshot data) | `adhoc` (freely defined via CLI)
- Color/icon are **not** written into the event (presentation detail, resolvable via `id`).

### 4.5 Write Strategy
- **Running timer:** The daemon sends periodic heartbeats (e.g. every 60 s, `pulsetime` ≈ 90 s) with identical `data`, so AW merges them into one event. On stop/switch, a final heartbeat with the exact end time is sent. A crash therefore loses at most one interval.
- **Restart (reboot, re-login, daemon crash):** A previously running activity is **not resumed automatically**. It ends with its last heartbeat. After startup the daemon shows a notification "Last activity: … (ended 09:42)" with a "Resume" action, which starts the activity as a new event from now on.
- **Retroactive entry:** Direct insertion of an event with start and duration.
- **Overlapping retroactive entries:** The **retroactive entry takes precedence**. Overlapping manual events in the own bucket are trimmed or split without asking. If the retroactive entry overlaps the running activity, the running activity restarts at the end of the retroactive entry.
- Other buckets are never modified.

### 4.6 Offline Buffer
- If aw-server is unreachable, all write operations are stored in a local SQLite queue and replayed in order once the server is back.
- The tray icon shows an "offline/pending" state with the number of pending entries.
- The bucket is created automatically on the first successful contact.

### 4.7 aw-server Connection
- URL **freely configurable**, default `http://localhost:5600`. This also allows a remote or central aw-server.
- Bucket ID and `hostname` always remain those of the local computer, even with a central server.
- The offline buffer applies regardless of whether the server is local or remote.

## 5. Input Methods

### 5.1 Popover
Opens via global hotkey, tray click or `aw-manual popover`.

**Features (MVP):**
- **Fuzzy search** across all favorites including their path ("acm web dev" → *ACME › Website › Development*); results are leaves only
- **Fully keyboard-operable** (arrow keys, Enter = start, Esc = close, shortcut for stop, shortcut for note)
- Shows the **running activity** with duration and a stop action
- Favorites list in **fixed, configured order**, the whole tree expanded with folders as pinned headings; only leaves can be highlighted
- **Retroactive mode:** relative or absolute time input, e.g. `-30m`, `10:00-11:15`, `since 9:30`
- Optional note field; while an activity is running, a new note creates a new activity (see 4.3)

**Scroll mode (MVP): operation with hotkey + scroll wheel only**
The popover must be fully operable without keyboard, mouse click or touch. **The reference device is a regular desktop mouse** (notched scroll wheel, hotkey e.g. on a mouse side button). Other devices (foot switch + wheel, macro button, rotary knob) are supported through the same mechanisms.

**Scope:** Scroll mode covers **starting and stopping** activities only. Retroactive entries and notes are deliberately not available in scroll mode; they remain in keyboard mode.

- **Open:** Hotkey. Focus is immediately on the favorites list.
- **Layout:** the whole tree expanded, with folders as headings.
- **Select:** Scrolling moves the highlight item by item through the list, in the fixed configured order. Only leaves (and the navigation items) can be highlighted.
- **Confirm**, two ways (can be enabled individually or combined):
  - Press the **hotkey** again (default: on)
  - **Dwell time:** the highlight stays on an item for a configurable time (`popover.scroll.dwell_time`, e.g. 2 s; default: off). A progress bar on the highlighted item shows the countdown; any scroll resets it. Dwell only starts after the first scroll movement, so merely opening the popover triggers nothing.
- **Effect of confirming:**
  - On a leaf (e.g. task): starts the activity and closes the popover
  - On the already running activity: closes the popover without change (no restart)
- **Navigation items** inside the list, also reachable by scrolling:
  - "■ Stop" as the first item of the top level while an activity is running
  - "✕ Cancel" as the last item
- **Abort without input:** The popover closes after 10 s without input (longer when the dwell time needs it).
- **Start position:** On open, the highlight is on the running activity or the first item (configurable).
- **Scroll behavior:**
  - Default: one notch of the mouse wheel = one step
  - Deltas are accumulated, one step per threshold, so high-resolution wheels and touchpads don't "race through"
  - Wrap around at the end of the list
  - Horizontal scrolling or wheel tilt (if available) optionally acts as Confirm (right) and Confirm with a note (left)
- **Readability:** The highlighted item is clearly emphasized and shown with its full path, since search isn't available without a keyboard.

**Input abstraction:** Internally the popover works with abstract navigation commands `next`, `prev`, `confirm`, `back`, `cancel`. They can be triggered by:
- Scroll wheel (when the popover receives the scroll events)
- Keyboard (arrow keys, Enter, Backspace, Esc)
- CLI: `aw-manual nav next|prev|confirm|back|cancel`
- OpenAction rotary knob (see 5.5, Phase 2)

This lets scroll mode work even where a device sends key presses instead of scroll events, or where scroll events don't reach the popover.

**Platform note on scroll events:** Scroll events go to the window under the mouse pointer, not to the focused window. The popover therefore opens **under the mouse pointer** by default (macOS, Windows, X11), which is natural anyway with a mouse as the reference device. On Wayland this is only possible via compositor rules (e.g. Hyprland `windowrule = move cursor …`); the documented alternative is mapping the wheel in the compositor or with a tool (e.g. `keyd`, `input-remapper`) to `aw-manual nav next|prev`.

**Platform note:** On Wayland a window cannot freely position itself. The popover appears centered there or via a compositor rule (e.g. Hyprland `windowrule` float/center); documentation for this is provided.

### 5.2 Tray / Status Bar
- **Icon states:** idle (no activity), active (optionally tinted in the favorite's color), offline/pending
- Click opens the popover; context menu with: Stop, Resume, Favorites (fixed order, nested, only leaves startable), Settings, Quit

**Configurable display next to the icon:**
The user chooses which text appears next to the icon while an activity is running:

| Mode | Example |
|---|---|
| `icon` – app icon only | – |
| `elapsed` – relative duration | `1h 23m` |
| `since` – absolute start time | `since 09:30` |
| `title` – activity | `Development` |
| `template` – free combination | `{title} · {elapsed}` |

- Template placeholders: `{title}`, `{path}`, `{elapsed}`, `{since}`, `{note}`
- Configurable formats: `elapsed` as `1h 23m` or `1:23`; `since` in 24h or 12h format
- Text updates once per minute (or once per second if a format includes seconds)
- The tooltip always shows full information (path, start time, duration, note), regardless of display mode
- **Platform limits:** Text next to the icon is possible on macOS (menu bar) and on Linux with an AppIndicator label (GNOME, KDE). Windows cannot show text in the notification area; only the tooltip applies there. For Waybar and similar bars, `aw-manual status --format '…' --follow` provides the same placeholders as a custom module.

### 5.3 Global Hotkeys (hybrid)
- **macOS, Windows, Linux/X11:** Hotkeys registered directly in the daemon, configurable; besides key combinations also **mouse buttons** (e.g. side buttons), so a regular mouse suffices without a keyboard
- **Linux/Wayland:** XDG GlobalShortcuts portal where available (KDE, newer GNOME versions); otherwise binding in the compositor/system to CLI commands (e.g. Hyprland `bind = SUPER, T, exec, aw-manual popover`, or for mouse buttons `bind = , mouse:276, exec, aw-manual popover`). Example configurations are documented.
- Default actions: open popover, stop, resume last stopped activity; optionally hotkeys for individual favorites
- **The popover hotkey is context-dependent:** closed → open; open → `confirm` (scroll mode). This also applies when the hotkey is triggered via `aw-manual popover` on Wayland.

### 5.4 CLI
The CLI serves scripts, macro tools, status bar modules and hotkey binding on Wayland. Hardware is **not** connected via the CLI but via OpenAction (see 5.5).

```
aw-manual start <favorite-id> [--note TEXT]
aw-manual start --adhoc --id ID --title TITLE [--path A/B/C] [--kind activity|status] [--note TEXT]
aw-manual note TEXT                     # new note: ends the running activity and restarts it with the note
aw-manual stop
aw-manual toggle <favorite-id>          # start, or stop if already running
aw-manual resume                        # resume the last stopped activity
aw-manual log <favorite-id> --from T --to T | --last 30m [--note TEXT]
aw-manual status [--json] [--format TEMPLATE] [--follow]
aw-manual list [--json]                 # favorites including hierarchy and order
aw-manual config check                  # validate config.yaml and all favorites files
aw-manual popover                       # open, or confirm if already open
aw-manual nav next|prev|confirm|back|cancel
aw-manual watch --json                  # stream state changes as JSON lines
```
- `start`, `toggle` and `log` accept leaf IDs only; folder IDs result in an error
- Stable, documented exit codes and JSON output (versioned schema)
- If the daemon isn't running, the CLI starts it (configurable) or reports a clear error

### 5.5 Hardware via OpenAction (Phase 2)
Instead of its own device drivers, aw-manual ships an **OpenAction plugin** (`.sdPlugin` directory). OpenAction is a standardized, cross-platform plugin API for programmable control surfaces and is backwards-compatible with the Elgato Stream Deck SDK.

**Target servers (plugin hosts):**
| Server | Platforms | Note |
|---|---|---|
| OpenDeck (reference implementation) | Linux, macOS, Windows | Primary target; covers Linux and can be extended to more devices |
| Tacto | Linux, macOS, Windows | Smartphone as control surface |
| Elgato Stream Deck software | macOS, Windows | Only without OpenAction-exclusive extensions |
| Mirabox Stream Dock and similar | Per vendor | Best effort |

Unknown or custom hardware (Timebuzzer, programmable cube …) is supported as soon as an OpenAction server offers it as a device; aw-manual itself needs no changes for that.

**Plugin architecture:**
- Rust, based on the official `openaction` crate; compiled binaries per target platform
- The plugin connects to the OpenAction server via WebSocket and to the aw-manual daemon via IPC; it holds no tracking state itself
- Property inspector (per-key settings UI) as a web UI, e.g. Svelte with the official OpenAction Svelte library
- If the daemon isn't running, all keys show an offline state (`showAlert` on press); reconnection is automatic

**Actions:**
| Action | Controller | Behavior |
|---|---|---|
| **Favorite** | Keypad | Bound to a fixed favorite (selected in the property inspector, dropdown with hierarchy, leaves only). The **favorite ID** is stored, not the position: the key always starts the same favorite, regardless of the order in the config. For deleted favorites see below. Pressing again on the running favorite: configurable stop or nothing |
| **Favorite Slot** | Keypad | Shows the *n*-th leaf directly under a chosen level (e.g. slot 3 under "ACME › Website"); folders are skipped when counting. The key layout thereby follows the order in the config automatically. Empty slot: key stays blank |
| **Custom Activity** | Keypad | The activity is **fully defined on the key** (property inspector: ID, title, path, kind, color, icon, optional default note) and is independent of the app config. Works even if no matching favorite exists |
| **Stop** | Keypad | Stops the running activity |
| **Resume** | Keypad | Resumes the last stopped activity |
| **Status** | Keypad | Shows the running activity + duration; pressing opens the popover |
| **Open Popover** | Keypad | Opens the popover |
| **Favorite Dial** | Encoder | Rotating scrolls through the favorites (shown on the touch strip/display), pressing confirms (open folder or start leaf), touch tap = back. Optionally the knob controls the open popover instead (`next`/`prev`/`confirm`) |

**Bound favorites and deleted entries:**
- The **Favorite** action stores, in its instance settings (`setSettings`, persisted by the OpenAction server), a **snapshot** of the favorite alongside its ID: title, path, kind, color, icon. The snapshot is refreshed on every config change as long as the favorite exists.
- **Rename/reorder:** The key follows the ID, so it shows the new title and stays the same despite a new order.
- **ID no longer exists or has become a folder** (favorite deleted, ID changed, children added), configurable per key:
  - `snapshot` (default): the key keeps working with the snapshot data and is marked with a subtle "orphaned" indicator
  - `disabled`: the key is greyed out, pressing triggers `showAlert`
- The property inspector shows orphaned keys and offers to re-link them or convert them into a **Custom Activity**.
- **Custom Activity** is the deliberate choice for keys that should never depend on the app config. If its ID happens to also exist as a favorite, the key's data still applies.
- For both cases the daemon accepts **ad-hoc activities** via IPC (full definition instead of just an ID). The event then contains `"origin": "deck"` instead of `"origin": "config"`, so downstream analysis can identify these entries.

**Display:**
- Title, color and icon come from the config (or, for Custom Activity and orphaned favorites, from the instance settings) and are set via `setTitle`/`setImage`; config changes appear live
- The running activity is highlighted on its keys (state 1 or a dedicated image) and shows the running duration
- Feedback on actions via `showOk` or `showAlert`

**Limitation (consciously accepted):** The OpenAction API does not allow plugins to assign keys themselves or create pages/profiles. Key assignment is done by the user in the OpenAction server. "Favorite Slot" and "Favorite Dial" ensure that content still follows the config automatically. Optionally a preconfigured example profile is shipped if the respective server supports profile import.

**Compatibility:** Besides `CodePaths` (OpenAction extension, multiple architectures), the manifest also sets `CodePathWin`/`CodePathMac`/`CodePathLin` so the plugin runs in the Elgato software too. OpenAction-exclusive features (e.g. `HasSettingsInterface`) are optional and must not be required for core functionality.

### 5.6 Desktop Widget (Phase 3)
- macOS WidgetKit widget: running activity, duration, quick start for a few favorites
- Later options: KDE plasmoid, GNOME extension, Windows widget
- Communicates through the existing daemon interface

## 6. Notifications
Native system notifications, each individually toggleable:
- **No timer running:** Reminder when no activity has been running for more than X minutes (default 15) – only within configurable working hours (weekdays/times), since there is no dependency on AFK data
- **Long-running activity:** Notice when the same activity has been running for more than X hours (default 3 h); notification actions: "Continue", "Stop", "Open popover"
- **After restart / unlocking the screen:** The popover asks what the user did in between (retroactive entry for the gap), then whether to continue with that activity from now, otherwise which activity to start
- **Offline:** One-time notice when entries are being buffered
- Status permanently visible in the tray (see 5.2)

## 7. Configuration
Configuration is written in **YAML** and split into two parts: the **base configuration** (one file) and the **favorites** (a folder with any number of files). The split lets scripts and other tools generate favorites (e.g. from a project management system) without touching hand-written settings.

### 7.1 File Layout
```
<config dir>/aw-manual/
├── config.yaml              # base configuration
└── favorites.d/             # favorites, any number of files
    ├── 10-clients.yaml      # hand-written
    ├── 20-internal.yaml
    ├── 50-jira.yaml         # generated by a script
    └── 90-status.yaml
```
- Config directory per platform:
  - Linux: `$XDG_CONFIG_HOME/aw-manual/`
  - macOS: `~/Library/Application Support/aw-manual/`
  - Windows: `%APPDATA%\aw-manual\`
- Both parts are version-controllable (dotfiles-friendly). A generated file can be excluded from version control on its own.

### 7.2 Base Configuration (`config.yaml`)
```yaml
server:
  url: http://localhost:5600        # remote server also possible
  bucket_id: "aw-manual_{hostname}"

favorites:
  dirs:                             # loaded in this order; default: [favorites.d]
    - favorites.d
    # - ~/Sync/aw-manual-favorites  # e.g. a shared or synced folder
  default_file: 00-ui.yaml          # new top-level favorites from the settings window

hotkeys:
  popover: Ctrl+Alt+Space           # alternatively a mouse button, e.g. Mouse4
  stop: Ctrl+Alt+S

tray:
  display: template                 # icon | elapsed | since | title | template
  template: "{title} · {elapsed}"
  color_icon: true                  # tint the icon in the favorite's color

format:                             # tray, popover, CLI, notifications
  elapsed: 1h 23m                   # "1h 23m" | "1:23"
  time: 24h                         # 24h | 12h

notifications:
  idle_reminder: 15m
  long_running: 3h
  work_hours:
    days: [mon, tue, wed, thu, fri]
    from: "08:00"
    to: "18:00"

popover:
  scroll:
    confirm_with_hotkey: true
    dwell_time: false               # e.g. 2s: confirm by dwell time
    open_at_cursor: true
    start_at: running               # running | first
```

### 7.3 Favorites Files (`favorites.d/*.yaml`)
Each file contains a subtree of favorites under the key `favorites`. Two notations are supported and produce the same tree; each file uses one of them (detected automatically: a mapping means compact, a list means explicit).

#### Compact notation (for hand-written files)
The tree is written as nested keys. **Keys are titles**, nesting is the hierarchy. **Properties start with `_`**, so they can never be confused with a child named e.g. "color". This works the same on folders and leaves.

Example `10-clients.yaml`:
```yaml
favorites:
  ACME:
    _color: "#E4572E"            # applies to ACME and everything below it
    _icon: briefcase
    Website:
      Development:
        _icon: code              # overrides the inherited icon
      Code Review:               # empty value = leaf with inherited color/icon
    Internal Tools:
      _color: "#3B82F6"          # overrides the inherited color for this subtree
      _id: tools                 # explicit ID instead of "internal-tools"
      Maintenance:
  Break:
    _kind: status
    _icon: coffee
```

Rules:
- **Properties:** `_id`, `_color`, `_icon`, `_kind`. Unknown keys starting with `_` are an error, so typos are caught. Titles must not start with `_`.
- **Leaf vs. folder:** A node is a leaf if it has no child keys, i.e. its value is empty (`Code Review:`) or contains only properties (`Development: {_icon: code}`). As soon as it has a child key, it's a folder.
- **List shorthand for leaves:** A folder's children can be given as a list of titles if none of them needs properties:
  ```yaml
  favorites:
    ACME:
      _color: "#E4572E"
      Website: [Development, Code Review, Deployment]
  ```
- **IDs:** Without `_id`, the ID is derived from the title as a slug ("Code Review" → `code-review`, "ACME" → `acme`). Renaming the title therefore changes the ID; anyone who renames often or uses OpenAction keys (see 5.5) should set `_id`. `aw-manual config check` warns about leaves without `_id` on request (`--strict`).
- **Titles** containing YAML special characters (`:`, `#`, leading `-` etc.) must be quoted, e.g. `"WEB-123: Checkout bug":`.
- **Duplicate titles** under the same folder are invalid YAML (duplicate key) and reject the file. Two different titles resulting in the same slug are also an error.
- **Order** is the written order of the keys. The YAML parser must preserve mapping order (requirement for the library choice).

#### Explicit notation (for generators)
Every node is an object with named fields. More verbose, but unambiguous and easy to emit from code. Example `50-jira.yaml`:
```yaml
# generated by jira-sync – do not edit by hand
generated: true                     # settings UI shows this file read-only
parent: acme/website                # optional; default: top level
favorites:
  - id: web-123
    title: "WEB-123 Checkout bug"
  - id: web-140
    title: "WEB-140 New footer"
    icon: bug
    children: []                    # optional; leaves have no or empty children
```
Fields: `id` (required), `title` (required), `color`, `icon`, `kind`, `children`. Resulting IDs: `acme/website/web-123`, `acme/website/web-140`.

#### Inheritance of color and icon (both notations)
- `color` and `icon` are **inherited** from the nearest ancestor that defines them. A child only needs its own value to override it.
- This lets a client's color apply to all its projects and tasks with a single line, which is also how it shows on OpenAction keys and in the tray.
- `kind` is **not** inherited; it's only meaningful on leaves and defaults to `activity`. `kind` on a folder is an error.
- With `parent`, the attached favorites inherit from the referenced folder, even if that folder is defined in another file.
- `aw-manual list --json` outputs the resolved values (after inheritance) and marks each value as `own` or `inherited`.

#### Attaching to an existing folder
A file can set `parent` (full ID of a folder or leaf) to hang its favorites under a folder defined in another file. This lets a generator add tasks below a hand-written client without rewriting it. `parent` works with both notations; `generated: true` likewise.

#### Merge rules
- Files with extension `.yaml` or `.yml` are loaded; hidden files (`.*`) and temp files (`*~`, `*.tmp`) are ignored. Subfolders are not read.
- **Order:** directories in the order of `favorites.dirs`, files within a directory by file name (lexical), favorites within a file in their written order. Numeric prefixes like `10-`, `50-` control the order. The order stays fixed and manual as required in 4.2.
- With `parent`, the file's favorites are appended after the existing children of that folder, again in file-name order.
- **Duplicate full IDs** across files are an error: the later file is rejected as a whole, with a clear message naming both files. The earlier definition stays active.
- A `parent` that doesn't exist rejects that file.
- A `parent` that points to a leaf turns that leaf into a folder once the file adds children to it (e.g. a hand-written rough structure whose final leaves come from a generated file). The former leaf can no longer be started; its `kind` no longer applies.
- **Errors are isolated per file:** an invalid file is skipped (or its last valid version stays active); all other files keep working.

#### Tips for generators (documented)
- Prefer the explicit notation.
- Write atomically (write to a temp file, then rename), so the file watcher never reads a half-written file.
- Keep IDs stable across runs (e.g. derived from the ticket key, not from the title), so OpenAction keys and AW data stay consistent. Removed favorites are handled on OpenAction keys via the snapshot fallback (see 5.5).
- Mark the file with `generated: true`.

### 7.4 Common Behavior
- **JSON Schemas** for `config.yaml` and favorites files are published and versioned. Generators and editors (e.g. via `yaml-language-server`) can validate against them. A `schema_version` key is optional in every file.
- `aw-manual config check` validates all files and prints errors per file and line; `aw-manual list --json` shows the merged result.
- External file changes are applied live (file watcher with short debounce), including on OpenAction keys. Adding, changing and deleting files in `favorites.d/` is supported.
- Validation with clear error messages; with an invalid `config.yaml` the last valid base configuration stays active.
- The **settings UI** reads and writes the same files:
  - Base settings are written into `config.yaml`.
  - Favorites are edited in the file they come from; new top-level favorites go into `favorites.default_file` (`00-ui.yaml`) unless the user picks a file.
  - Files marked `generated: true` are shown read-only.
  - Comments and order are preserved when writing: the UI edits the source text at the spans of the parsed nodes instead of re-serializing the file. Layouts it can't edit safely (e.g. flow mappings) produce an error asking the user to edit the file by hand.

## 8. Distribution & Operations
- **License:** Open source (suggestion: MPL-2.0, like ActivityWatch)
- **Packages:**
  - Linux: `.deb`, **Nix flake** (package + NixOS/Home Manager module), optionally AppImage/`.rpm`
  - macOS: signed and notarized `.dmg` (universal binary), optionally Homebrew Cask
  - Windows: `.msi` and `.exe` (NSIS), signed
- **Autostart** per platform (LaunchAgent, registry/startup, XDG autostart or systemd user service), toggleable in settings
- The CLI is installed alongside and placed on the `PATH`
- **OpenAction plugin** (Phase 2) is published separately as an `.sdPlugin` package (or `.streamDeckPlugin` for the Elgato software) with binaries for `x86_64`/`aarch64` on all three platforms; the goal is a listing on the OpenAction Marketplace. Optionally the app installs the plugin on request into a detected OpenDeck plugin folder.
- CI builds for all platforms (GitHub Actions)

## 9. Quality
- Unit tests for the state machine, time parser (`-30m`, `10:00-11:15`), overlap logic, note split, queue, tray templates, favorites parsing and merge rules (both notations, slugs, inheritance, order, `parent`, duplicate IDs, per-file errors)
- Integration tests against a real aw-server (test mode/port)
- No telemetry; no network access except to the configured aw-server
- Local logs, configurable level

## 10. Phases

**MVP**
- Daemon, IPC, offline queue, bucket handling, configurable server URL
- Popover (search, keyboard, retroactive mode, **scroll mode** for start/stop incl. dwell time)
- Tray with configurable status display
- Hotkeys (hybrid, incl. mouse buttons) and CLI incl. `nav` and `status --format`
- YAML configuration (base file + favorites folder) + basic settings UI
- Notifications
- Packages: deb, Nix flake, dmg, msi/exe

**Phase 2**
- **OpenAction plugin:** keypad actions (Favorite incl. snapshot fallback, Favorite Slot, Custom Activity, Stop, Resume, Status, Open Popover), tested with OpenDeck and the Elgato software; `.sdPlugin` package
- Extended settings UI (reorder favorites via drag & drop)

**Phase 3**
- OpenAction encoder action (Favorite Dial), Marketplace listing
- Native macOS widget, further desktop widgets

**Possible later extensions (not planned)**
- Timeline view with visual editing (click-drag) and context from other AW buckets
- Retroactive entries and notes in scroll mode

## 11. Decisions

| Question | Decision |
|---|---|
| OpenAction plugin in MVP | No, Phase 2 (keypad actions); rotary knob in Phase 3 |
| Startable levels | Leaves only |
| Overlapping retroactive entries | Retroactive entry takes precedence, without asking |
| Editing notes | Not possible; a new note creates a new activity |
| Timeline | Removed, possibly later |
| Resume after restart | Not automatic; the popover asks what happened in between and what to continue with (also after unlocking the screen) |
| Remote aw-server | URL freely configurable |
| Scroll mode confirmation | Hotkey and/or dwell time |
| Scroll mode scope | Start and stop only; no retroactive entries or notes |
| Reference scroll device | Regular desktop mouse |
| Configuration format | YAML; base config and favorites folder separated |
| Favorites notation | Compact (titles as keys, `_`-prefixed properties) and explicit (list of objects); color/icon inherited |
| Project name | aw-manual (bucket `aw-manual_<hostname>`, CLI `aw-manual`) |
| YAML library | `saphyr-parser` with an own document model that keeps order and source positions; comment-preserving edits on the source text (`crates/core/src/yaml_edit.rs`) |
