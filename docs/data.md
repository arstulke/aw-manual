# Data in ActivityWatch

## Bucket

One bucket per host, created on the first successful contact with aw-server:

| Field | Value |
|---|---|
| ID | `aw-manual_<hostname>` (`server.bucket_id`) |
| type | `manual.activity` (`server.bucket_type`) |
| client | `aw-manual` |
| hostname | the local host name |

The bucket ID and host name stay those of the local computer, also with a
central aw-server. aw-manual never modifies other buckets.

## Events

```json
{
  "timestamp": "2026-10-09T08:30:00Z",
  "duration": 5400,
  "data": {
    "id": "acme/website/development",
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

| Field | Values |
|---|---|
| `id` | full favorite ID (or the ad-hoc ID) |
| `title` | full path, joined with ` › ` (`ACME › Website › Development`), so the ActivityWatch timeline shows where an activity belongs |
| `path` | titles from the top level down |
| `kind` | `activity` or `status` |
| `note` | free text; omitted when empty |
| `source` | `popover`, `popover-scroll`, `tray`, `cli`, `openaction`, `hotkey` |
| `device` | OpenAction device name (phase 2); omitted otherwise |
| `entry` | `timer` (tracked live) or `retro` (retroactive entry) |
| `origin` | `config` (favorite), `deck` (defined on an OpenAction key), `adhoc` (CLI `--adhoc`) |

Color and icon are not written (presentation only; resolvable via `id`).

## Writing

- **Running activity**: the start is written as a heartbeat; then a heartbeat
  every `heartbeat_interval` (default 60 s) and a final one at the exact stop
  time. aw-server merges them into one event. A crash loses at most one
  interval. The heartbeat merge window covers the whole run, so an activity
  keeps running across suspend/resume (aw-manual doesn't pause on its own).
- **Note change** ends the event and starts a new one with the new note.
- **Retroactive entries** are inserted directly. Overlapping aw-manual events
  are trimmed or split; if the entry overlaps the running activity, the
  running activity continues as a new event from the end of the entry.
- **Offline buffer**: every write goes through a local SQLite queue
  (`<data dir>/aw-manual.db`) and is sent in order. While aw-server is
  unreachable, entries stay queued (tray: offline state, pending count) and
  are sent when it is back, also after a restart. Entries aw-server rejects
  permanently (HTTP 4xx) are kept in the database table `dead_letters` and
  reported in `aw-manual status`.

Both aw-server (Python) and aw-server-rust are supported; the writer accounts
for their heartbeat caching (see the comments in
[`crates/core/src/writer.rs`](../crates/core/src/writer.rs)), and the
integration tests run against both.

## Querying

Example AW query for the time per activity:

```text
events = query_bucket("aw-manual_myhost");
events = merge_events_by_keys(events, ["id"]);
RETURN = sort_by_duration(events);
```

Use `["id", "note"]` to split by notes, or filter with
`filter_keyvals(events, "kind", ["activity"], false)` (activities only, without statuses).
