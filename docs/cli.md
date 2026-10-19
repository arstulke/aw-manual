# CLI reference

```text
aw-manual start <favorite-id> [--note TEXT]
aw-manual start --adhoc --id ID --title TITLE [--path A/B] [--kind activity|status] [--note TEXT]
aw-manual note TEXT                 # new note: ends the running activity and restarts it with the note ("" removes it)
aw-manual stop
aw-manual toggle <favorite-id> [--note TEXT]   # start, or stop if it is running
aw-manual resume                    # resume the last stopped activity (with its note)
aw-manual log <favorite-id> (--from T --to T | --from T --duration D | --last D | --range EXPR) [--note TEXT]
aw-manual status [--json] [--format TEMPLATE] [--follow]
aw-manual list [--json]             # favorites with hierarchy and order
aw-manual config check [--strict] [--json]
aw-manual config path               # print the configuration folder
aw-manual devices setup [DRIVER]       # interactive: pick a connected device, test it, save it to config.yaml
aw-manual devices list [--json]     # supported hardware drivers and connected devices (usb_id, nodes)
aw-manual devices monitor DRIVER [--path NODE]   # print the inputs of a device and their actions
aw-manual popover                   # open the popover, or confirm if it is open
aw-manual nav next|prev|confirm|back|cancel
aw-manual watch                     # stream state changes as JSON lines
aw-manual settings                  # open the settings window
```

`--json` (global flag) makes commands print JSON. `aw-manual --help` and
`aw-manual <command> --help` show all options.

## Behavior

- `start`, `toggle` and `log` accept **leaf IDs only**; folder IDs are an error.
- `start` of the running favorite changes nothing, unless `--note` gives a
  different note (then the running activity restarts with it).
- `resume` starts the most recently stopped or replaced activity, with its
  note. A note change doesn't count as stopping. If that favorite was deleted
  in the meantime, its last known title and path are used.
- `start --adhoc` starts an activity that isn't in the configuration
  (`origin: adhoc`). `--path` lists the folders above it; the title is
  appended (`--path ACME/Website --title Deploy` → `ACME › Website › Deploy`).
- `config check` doesn't need the daemon. It validates `config.yaml` and all
  favorites files and prints errors with file and line. `--strict` also warns
  about leaves without an explicit ID (`_id`) and fails on warnings.
- If the daemon isn't running, the CLI starts it (`aw-manual-daemon --background`)
  unless `cli.autostart_daemon: false`. The daemon is looked up next to the CLI,
  then on `PATH` (override with `AW_MANUAL_DAEMON`).

## Times

`--from` / `--to` accept `9:30`, `14:00`, `2:30pm`, `yesterday 16:00`,
`2026-10-09 09:30`, `2026-10-09T09:30`, RFC 3339 timestamps, `-30m` (30
minutes ago) and `now`. `--duration` and `--last` take durations such as
`45m`, `1h30m`, `1.5h`. `--range` takes the expressions of the popover
(`-30m`, `10:00-11:15`, `since 9:30`, `10:00 +45m`, see [Usage](usage.md)).

## Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | other error |
| 2 | invalid command line |
| 3 | the daemon is not running and could not be started |
| 4 | unknown favorite ID |
| 5 | invalid input (folder instead of leaf, invalid time range, …) |
| 6 | not possible in the current state (nothing running, nothing to resume) |
| 7 | configuration check failed |
| 8 | not available (e.g. no popover when the daemon runs headless) |

## JSON output

All JSON objects carry `"schema_version": 1`. The schema only changes in
backwards-compatible ways (new fields) within a version.

`aw-manual status --json`:

```json
{
  "schema_version": 1,
  "running": {
    "id": "acme/website/development",
    "title": "Development",
    "path": ["ACME", "Website", "Development"],
    "kind": "activity",
    "origin": "config",
    "note": "Navigation refactoring",
    "source": "cli",
    "start": "2026-10-09T08:30:00Z",
    "elapsed": 5400,
    "color": "#E4572E",
    "icon": "code"
  },
  "last_stopped": null,
  "server": { "url": "http://localhost:5600", "bucket": "aw-manual_myhost", "online": true, "pending": 0, "failed": 0, "last_error": null },
  "now": "2026-10-09T10:00:00Z"
}
```

`running` and `last_stopped` are `null` when not applicable. `server.online` is
`null` until the first contact. `pending` counts buffered entries; `failed`
counts entries aw-server rejected permanently (kept in the local database).

State-changing commands (`start`, `stop`, `toggle`, `resume`, `note`, `log`)
return `{"change": "...", "status": {...}}` with `change` one of `started`,
`switched`, `stopped`, `note_changed`, `logged`, `unchanged`.

`aw-manual list --json` returns `favorites` (the merged tree; each node has
`id`, `title`, `path`, `type` (`folder`/`leaf`), `kind` (leaves), `color` and
`icon` as `{"value", "source": "own"|"inherited"}`, `file`, `line`, `generated`,
`children`), `files` (status per file: `ok`, `invalid`, or `last_valid` with
`errors`), `warnings` and `config` (state of `config.yaml`).

## Environment variables

| Variable | Purpose |
|---|---|
| `AW_MANUAL_CONFIG_DIR` | configuration folder (default: see [Configuration](configuration.md#locations)) |
| `AW_MANUAL_DATA_DIR` | local state: offline queue, state, logs |
| `AW_MANUAL_SOCKET` | IPC socket path (Unix) or pipe name (Windows) |
| `AW_MANUAL_DAEMON` | daemon binary started by the CLI |
| `AW_MANUAL_LOG` | log filter for the daemon, e.g. `debug` (overrides `log.level`) |

## IPC

The CLI talks to the daemon over a Unix domain socket
(`$XDG_RUNTIME_DIR/aw-manual/daemon.sock`, otherwise
`$TMPDIR/aw-manual-<uid>/daemon.sock`) or a named pipe on Windows
(`\\.\pipe\aw-manual-<user>`), using newline-delimited JSON:

```text
→ {"v":1,"id":1,"method":"start","params":{"target":{"favorite":"acme/dev"},"note":null,"source":"cli"}}
← {"id":1,"ok":true,"result":{"change":"started","status":{...}}}
← {"id":2,"ok":false,"error":{"code":"not_found","message":"unknown favorite \"x\""}}
```

Methods: `ping`, `status`, `list`, `start`, `toggle`, `stop`, `resume`, `note`,
`log`, `popover`, `show_popover`, `nav`, `settings`, `reload`, `quit`, `watch`
(keeps the connection open and pushes `{"event":"status","data":{...}}`).
Error codes: `not_found`, `is_folder`, `invalid`, `not_running`,
`nothing_to_resume`, `internal`, `bad_request`, `unavailable`.
`start` also accepts `{"target":{"adhoc":{"id","title","path","kind","origin"}}}`
with `origin` `adhoc` or `deck`.
