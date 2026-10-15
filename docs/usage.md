# Usage

aw-manual consists of a daemon (`aw-manual-daemon`) and its clients: the
popover, the tray icon, global hotkeys, and the CLI (`aw-manual`). Only the
daemon talks to aw-server and holds the state; all inputs go through it.

**At most one activity runs at any time.** Starting another activity ends the
running one. Only leaves of the favorites tree can be started; folders only
organize.

## Daemon

```text
aw-manual-daemon               start and show the popover (first start: the settings window)
aw-manual-daemon --background  start without showing anything (autostart, CLI)
aw-manual-daemon --headless    no tray, popover or hotkeys; IPC and tracking only
```

Starting the daemon while it is already running brings the running instance's
popover to the front. "Start aw-manual at login" in the settings window enables
autostart (LaunchAgent on macOS, registry on Windows, XDG autostart on Linux).
With Nix, the NixOS and Home Manager modules start it as a systemd user service.

After a reboot or crash, a previously running activity is **not** resumed. It
ends with its last heartbeat (at most one heartbeat interval is lost), and the
popover asks what you did in between and what to continue with (see
[After a restart or unlocking the screen](#after-a-restart-or-unlocking-the-screen)).

## Popover

Open it with the popover hotkey (default **Ctrl+Alt+Space**), a click on the
tray icon, or `aw-manual popover`. The window is kept alive in the background,
so it appears immediately. It shows the running activity with its duration, a
Stop button, and a ⚙ button that opens the settings window (also **Ctrl+,**).

### Scroll mode (hotkey + scroll wheel)

The popover opens in scroll mode. It can be used without keyboard, click or
touch; the reference device is a regular mouse, with the hotkey on a side
button (e.g. `hotkeys.popover: Mouse4`).

- The list shows the **whole favorites tree, expanded** (`layout: tree`, the
  default): folders are headings that stay pinned at the top while you scroll
  through their contents, so the hierarchy of the highlighted item is always
  visible, and **scrolling** moves the highlight through
  the leaves only, in the configured order, so every favorite is reachable
  without opening folders. With `layout: levels`, the list shows one folder
  level at a time instead. The highlighted item is shown with its full path at
  the bottom.
- **Confirm** by pressing the hotkey again (`popover.scroll.confirm_with_hotkey`,
  on by default) and/or by **dwell time**: keep the highlight on an item for
  `dwell_time` (default 2 s, `dwell_confirm: true`). A progress bar shows the
  countdown; any scroll restarts it. Dwell only starts after the first scroll
  movement, so opening the popover alone never triggers anything.
- Confirming a **leaf** starts the activity and closes the popover; the
  **running activity** just closes the popover. With `layout: levels`,
  confirming a **folder** opens it.
- Navigation items: "■ Stop" (first item while an activity runs), "✕ Cancel"
  (last item), and with `layout: levels` "‹ Back" (first item of every
  sub-level). In the tree layout, Back jumps to the first leaf of the current
  folder, then to the previous folder.
- Without input, the popover closes after `timeout` (default 10 s; it must be
  longer than the dwell time).
- The highlight starts on the running activity (`start_at: running`) or on the
  first favorite (`start_at: first`).
- Each notch of a mouse wheel is one step. High-resolution wheels and touchpads
  are accumulated (one step per threshold, scaled by `sensitivity`), so they
  don't race through the list. Scrolling past the last item continues at the first one and vice versa
  (`wrap_around`, on by default; also for the scroll wheel in keyboard mode).
- `horizontal: true`: scrolling left (or wheel tilt) is Back, right is Confirm.
- With `confirm_with_hotkey: false`, the hotkey closes the open popover.

Keys in scroll mode: **↑/↓** move, **Enter/→** confirm, **Backspace/←** back,
**Esc** cancel. The same commands are available as
`aw-manual nav next|prev|confirm|back|cancel`, for devices that send key
presses, or when scroll events don't reach the popover.

Scroll events go to the window under the mouse pointer, so the popover opens
**under the pointer** (`open_at_cursor`, macOS, Windows, X11). On Wayland,
windows cannot place themselves, see [Platform notes](#platform-notes).

Scroll mode only starts and stops activities. Notes and retroactive entries
need the keyboard.

### Keyboard mode

Typing any letter switches to keyboard mode and searches; **Tab** switches to
the favorites tree (all folders expanded, unless `layout: levels`).

| Key | Action |
|---|---|
| typing | fuzzy search over all favorites and their path (`acm web dev`); results are leaves only |
| ↑ / ↓, PgUp / PgDn | move the highlight |
| scroll wheel | move the highlight through the leaves (search results or tree); confirm with Enter, a click, or the popover hotkey |
| Enter | start the highlighted favorite (or open/close a folder in the tree) |
| Shift+Enter, right click | ask for a note, then start the favorite with it (both also work in scroll mode) |
| → / ← | expand / collapse folders (tree, empty search) |
| Esc | clear the search, then close |
| Ctrl+N | note field |
| Ctrl+R | retroactive mode (time range field) |
| Ctrl+S | stop the running activity |
| Ctrl+, | settings |

**Notes.** Enter a note before starting an activity, and it is stored with the
entry. Shift+Enter or a right click on a favorite asks for a note first, and
favorites with `_ask_note: true` (e.g. a generic favorite that needs a ticket
number) always ask, also from the tray menu and hotkeys (see
[Configuration](configuration.md#asking-for-a-note)). While an activity runs, Enter in the note field applies the note to it:
the running activity ends now and starts again with the new note (notes of
existing entries are never changed). Starting the running favorite again with a
different note does the same; without a note it changes nothing.

**Retroactive entries.** Ctrl+R shows a time field with a live preview. Then
pick a favorite and press Enter. Supported ranges:

| Input | Meaning |
|---|---|
| `-30m`, `30m`, `last 1h30m` | the last 30 minutes / 1½ hours until now |
| `10:00-11:15`, `10:00 to 11:15` | absolute range today |
| `since 9:30` | from 9:30 until now |
| `10:00 +45m`, `10:00 for 45m` | start plus duration |
| `yesterday 16:00-17:30`, `2026-10-09 10:00-11:00` | another day |
| `9am-10:30am`, `9-10` | 12-hour clock, bare hours |
| `10:03:27-10:45:05` | with seconds |

A clock time without a date means its most recent occurrence: if the start
lies in the future today, the range moves to yesterday (`23:00-01:00` early in
the morning). An end before the start means the next day. Ranges must not end
in the future.

A retroactive entry takes precedence: overlapping aw-manual entries are trimmed
or split without asking. If it overlaps the running activity, the running
activity restarts at the end of the entry.

### After a restart or unlocking the screen

Time in which you couldn't track anything (fixing a computer problem, an
on-site meeting, helping a colleague, a break) is easy to fill in afterwards:

1. **What were you doing?** The popover opens with the gap prefilled as a
   retroactive range ("aw-manual wasn't running from 09:42 to 10:15" or "The
   screen was locked from …"). The activity from before the gap is
   preselected, so Enter logs it. Pick another favorite, or press Esc to skip
   logging.
2. **Continue with … from now?** Shift+Y continues the activity you just picked. It
   is also highlighted in the list, so Enter continues it too; move the
   highlight (or search) and press Enter to start another activity instead. If you skipped step 1, it offers the activity from before the gap:
   after a restart it is resumed with its note; after unlocking it simply keeps
   running.
3. **What are you doing now?** If you answer No (Shift+N), pick the activity to
   start from now on. Plain letters always go to the search (it ignores case),
   so searching for "yellow" or "nothing" works. Esc at any step leaves things as they are.

Nothing is recorded unless you pick something. After unlocking, a skipped gap
stays part of the running activity, as aw-manual never pauses on its own.

- `notifications.after_restart` (default `1m`): after a (re-)start of
  aw-manual or the computer, ask what you did meanwhile if aw-manual was
  stopped for at least that long. The daemon records every 15 s that it is
  running, so quitting and starting it again works the same as a reboot or a
  crash. After a shorter stop, it only offers to continue the activity that was
  running. `false` disables it.
- `notifications.after_unlock` (default `1m`): after unlocking the screen, ask
  what you did meanwhile if it was locked for at least that long. `false`
  disables it.

Both take any [duration](configuration.md#durations), e.g. `10s`, `1m` or
`5m`. The prefilled range includes seconds (`2026-10-15 09:42:10-09:42:52`);
all time ranges accept seconds (`10:03:27-10:45`).

**Screen lock detection.** The screen counts as locked if any source says so:

| Platform | Sources |
|---|---|
| Linux | logind's `LockedHint` of your sessions (GNOME, KDE, lockers started via `loginctl lock-session` that report to logind); the screen saver D-Bus services (`org.freedesktop.ScreenSaver`, GNOME, Cinnamon, MATE, Xfce); a running locker process (`hyprlock`, `swaylock`, `gtklock`, `waylock`, `i3lock`, `xsecurelock`, `slock`, `physlock`, `xlock`) |
| Windows | the lock screen (input desktop) |
| macOS | `CGSSessionScreenIsLocked` |

If the prompt doesn't appear after unlocking:

1. Run `aw-manual-daemon --check-lock` in a terminal (a running daemon can
   stay running), lock the screen for a few seconds and unlock it. It prints
   every source whenever something changes; one of them must say `locked`
   while the screen is locked. If none does, your locker isn't covered:
   please report which one you use.
2. Look at the log (`~/.local/share/aw-manual/logs/` on Linux, see
   [Configuration](configuration.md#locations)). The daemon logs at `info`
   level which sources it found at startup, `screen locked (detected by: …)`,
   and on unlock why it asks or not (`not asking (minimum 1m)`, `prompt
   disabled`, …). The restart prompt logs `restart prompt: …` the same way.

## Tray / menu bar

- Icon states: idle (ring), active (filled, tinted in the favorite's color if
  `tray.color_icon`), offline or rejected entries (orange badge).
- Left click opens the popover (macOS, Windows). On Linux, use "Open popover"
  in the menu (AppIndicator icons only have menus).
- Menu: running activity with its elapsed time ("Running for 1h 23m (since
  09:30)", also when the text next to the icon is clipped), offline/pending state, Open popover, Stop, Leave a note… (opens the
  popover's note field for the running activity), Resume
  *last activity* (with its full path, since the same title can appear in
  several folders), Favorites (nested, in configured order), Settings…, Quit.
- Text next to the icon (`tray.display`): `icon` (none), `elapsed` (`1h 23m`),
  `since` (`since 09:30`), `title`, or `template` with `{title}`, `{path}`,
  `{elapsed}`, `{since}`, `{note}` (default `{title} · {elapsed}`).
  `elapsed_format` is `1h 23m`, `1:23`, `1h 23m 45s` or `1:23:45`; with seconds
  the text updates every second, otherwise every minute. `1h 23m` shows seconds
  during the first minute (`42s` instead of `0m`). `time_format` is `24h`
  or `12h`.
- The tooltip always shows path, start time, duration and note.

Text next to the icon works on macOS (menu bar) and Linux with AppIndicator
labels (GNOME, KDE). Windows cannot show text in the notification area; only
the tooltip applies. For Waybar and similar bars, use the CLI (below).

## Hotkeys

```yaml
hotkeys:
  popover: Ctrl+Alt+Space   # open the popover; while open: confirm
  stop: Ctrl+Alt+S
  resume: Ctrl+Alt+R        # resume the last stopped activity
  favorites:                # optional: start individual favorites
    acme/website/development: Ctrl+Alt+1
```

Keys: modifiers `Ctrl`, `Alt`/`Option`, `Shift`, `Super`/`Cmd`,
`CmdOrCtrl` (Cmd on macOS, Ctrl elsewhere), then one key (`A`…`Z`, `0`…`9`,
`F1`…`F24`, `Space`, `Enter`, `Tab`, arrows, …). Mouse buttons: `Mouse4` (back
side button), `Mouse5` (forward side button), `Mouse6`…`Mouse9` (further
buttons; Windows only has Mouse4 and Mouse5). Mouse hotkeys are consumed, so
the side button doesn't also navigate back in your browser.

| Platform | Keyboard | Mouse buttons |
|---|---|---|
| Windows | registered directly | low-level mouse hook |
| macOS | registered directly | event tap; allow aw-manual under System Settings → Privacy & Security → Accessibility |
| Linux X11 | registered directly | passive button grab |
| Linux Wayland | XDG GlobalShortcuts portal (KDE Plasma, newer GNOME, Hyprland with xdg-desktop-portal-hyprland); otherwise bind in the compositor | bind in the compositor |

Compositor bindings call the CLI, for example in `hyprland.conf`:

```ini
bind = SUPER, T, exec, aw-manual popover
bind = SUPER SHIFT, T, exec, aw-manual stop
bind = , mouse:276, exec, aw-manual popover        # forward side button
```

`aw-manual popover` behaves like the hotkey: it opens the popover, or confirms
when it is already open.

## Notifications

Each one can be turned off in `notifications`:

- **idle_reminder** (default 15m): no activity has been running that long, only
  within `work_hours` (days and times). Shown once per idle period.
- **long_running** (default 3h): the same activity has been running that long;
  actions Continue, Stop, Open popover. Shown once per run.
- **after_restart**, **after_unlock** (durations): open the popover and ask what you did
  meanwhile (see [above](#after-a-restart-or-unlocking-the-screen)).
- **offline**: aw-server is unreachable and entries are buffered; once per
  offline period.

Notification actions are available on Linux. On macOS and Windows,
notifications have no buttons; use the tray menu (Resume, Stop) instead.

## Status bars (Waybar, …)

`aw-manual status --format TEMPLATE --follow` prints a new line whenever the
text changes (and the elapsed time with it), and an empty line when nothing
runs. Waybar custom module:

```json
"custom/aw-manual": {
  "exec": "aw-manual status --format '{title} · {elapsed}' --follow",
  "on-click": "aw-manual popover",
  "on-click-right": "aw-manual stop"
}
```

`aw-manual watch` streams the full status as JSON lines.

## Platform notes

- **GNOME**: tray icons need the *AppIndicator and KStatusNotifierItem Support*
  extension.
- **Hyprland / Waybar**: the tray needs a bar with a tray module, e.g. Waybar's
  `tray`.
- **Wayland**: windows cannot position themselves, so the popover opens where
  the compositor puts it. Use a window rule to float it and, for scroll mode,
  place it under the cursor, e.g. Hyprland:

  ```ini
  windowrulev2 = float, title:^(aw-manual)$
  windowrulev2 = move onscreen cursor -50% -20%, title:^(aw-manual)$
  windowrulev2 = pin, title:^(aw-manual)$
  ```

  If scroll events don't reach the popover, map the wheel to
  `aw-manual nav next|prev` with your compositor or a tool such as `keyd` or
  `input-remapper`.
- **macOS**: mouse-button hotkeys need the Accessibility permission (macOS asks
  on first use).
- **Windows**: no text next to the tray icon (tooltip only).
