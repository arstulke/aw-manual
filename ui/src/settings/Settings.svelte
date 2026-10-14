<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { errorText, onStateChanged } from "../lib/api";
  import { iconGlyph, PATH_SEPARATOR } from "../lib/format";
  import type { Favorite } from "../lib/types";

  interface FileError {
    line: number | null;
    message: string;
  }
  interface FileReport {
    path: string;
    generated: boolean;
    notation: "compact" | "explicit" | null;
    status: "ok" | "invalid" | "last_valid";
    errors?: FileError[];
  }
  interface SettingsData {
    favorites: Favorite[];
    files: FileReport[];
    warnings: string[];
    config: Record<string, any>;
    config_state: { state: "ok" | "missing" | "invalid"; errors?: FileError[] };
    config_dir: string;
    config_path: string;
    favorite_dirs: string[];
    ui_file: string;
    writable_files: string[];
    hotkey_errors: string[];
    autostart: boolean;
    wayland: boolean;
    os: string;
    version: string;
    log_dir: string;
    status: { server: { url: string; online: boolean | null; pending: number; failed: number; last_error: string | null } };
  }

  type Tab = "general" | "tray" | "hotkeys" | "notifications" | "popover" | "favorites";
  const tabs: [Tab, string][] = [
    ["general", "General"],
    ["tray", "Tray"],
    ["hotkeys", "Hotkeys"],
    ["notifications", "Notifications"],
    ["popover", "Popover"],
    ["favorites", "Favorites"],
  ];
  const DAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

  let tab = $state<Tab>("general");
  let data = $state<SettingsData | null>(null);
  let loadError = $state("");

  // Editable form state: setting path → value.
  let form = $state<Record<string, any>>({});
  let original: Record<string, any> = {};
  let saveErrors = $state<string[]>([]);
  let saved = $state(false);
  let saving = $state(false);

  // Favorites form.
  let favParent = $state("");
  let favFile = $state("");
  let favTitle = $state("");
  let favId = $state("");
  let favKind = $state<"activity" | "status">("activity");
  let favUseColor = $state(false);
  let favColor = $state("#3b82f6");
  let favIcon = $state("");
  let favAskNote = $state(false);
  let favError = $state("");
  let favBusy = $state(false);

  /** Fields of the form: key = dotted path in config.yaml. */
  function fieldsFrom(c: Record<string, any>): Record<string, any> {
    const s = c.popover.scroll;
    return {
      "server.url": c.server.url,
      "server.bucket_id": c.server.bucket_id,
      "server.heartbeat_interval": c.server.heartbeat_interval,
      "cli.autostart_daemon": c.cli.autostart_daemon,
      "log.level": c.log.level,
      "tray.display": c.tray.display,
      "tray.template": c.tray.template,
      "tray.elapsed_format": c.tray.elapsed_format,
      "tray.time_format": c.tray.time_format,
      "tray.color_icon": c.tray.color_icon,
      "hotkeys.popover": c.hotkeys.popover ?? "",
      "hotkeys.stop": c.hotkeys.stop ?? "",
      "hotkeys.resume": c.hotkeys.resume ?? "",
      "notifications.idle_reminder": c.notifications.idle_reminder,
      "popover.afk_prompt": c.popover.afk_prompt,
      "notifications.long_running": c.notifications.long_running,
      "notifications.after_restart": c.notifications.after_restart,
      "notifications.offline": c.notifications.offline,
      "notifications.work_hours.days": [...c.notifications.work_hours.days],
      "notifications.work_hours.from": c.notifications.work_hours.from,
      "notifications.work_hours.to": c.notifications.work_hours.to,
      "popover.scroll.layout": s.layout,
      "popover.scroll.confirm_with_hotkey": s.confirm_with_hotkey,
      "popover.scroll.dwell_confirm": s.dwell_confirm,
      "popover.scroll.dwell_time": s.dwell_time,
      "popover.scroll.timeout": s.timeout,
      "popover.scroll.sensitivity": s.sensitivity,
      "popover.scroll.wrap_around": s.wrap_around,
      "popover.scroll.open_at_cursor": s.open_at_cursor,
      "popover.scroll.start_at": s.start_at,
      "popover.scroll.horizontal": s.horizontal,
    };
  }

  // Optional durations: a checkbox plus the duration text.
  let idleOn = $state(true);
  let idleValue = $state("15m");
  let longOn = $state(true);
  let longValue = $state("3h");
  let afkOn = $state(true);
  let afkValue = $state("15m");

  async function load(resetForm: boolean) {
    try {
      data = await invoke<SettingsData>("get_settings");
      loadError = "";
      if (resetForm) {
        original = fieldsFrom(data.config);
        form = structuredClone(original);
        const idle = data.config.notifications.idle_reminder;
        idleOn = idle !== false;
        idleValue = idle === false ? "15m" : idle;
        const long = data.config.notifications.long_running;
        longOn = long !== false;
        longValue = long === false ? "3h" : long;
        const afk = data.config.popover.afk_prompt;
        afkOn = afk !== false;
        afkValue = afk === false ? "15m" : afk;
      }
    } catch (e) {
      loadError = errorText(e);
    }
  }

  const changes = $derived.by(() => {
    const f = { ...form };
    f["notifications.idle_reminder"] = idleOn ? idleValue : false;
    f["notifications.long_running"] = longOn ? longValue : false;
    f["popover.afk_prompt"] = afkOn ? afkValue : false;
    const out: { path: string[]; value: any }[] = [];
    for (const [k, v] of Object.entries(f)) {
      if (JSON.stringify(v) !== JSON.stringify(original[k])) out.push({ path: k.split("."), value: v });
    }
    return out;
  });

  async function save() {
    saving = true;
    saveErrors = [];
    saved = false;
    try {
      await invoke("save_settings", { changes: $state.snapshot(changes) });
      saved = true;
      await load(true);
      setTimeout(() => (saved = false), 2500);
    } catch (e) {
      saveErrors = Array.isArray(e) ? (e as string[]) : [errorText(e)];
    } finally {
      saving = false;
    }
  }

  function revert() {
    form = structuredClone(original);
    saveErrors = [];
    if (data) {
      const idle = data.config.notifications.idle_reminder;
      idleOn = idle !== false;
      idleValue = idle === false ? "15m" : idle;
      const long = data.config.notifications.long_running;
      longOn = long !== false;
      longValue = long === false ? "3h" : long;
      const afk = data.config.popover.afk_prompt;
      afkOn = afk !== false;
      afkValue = afk === false ? "15m" : afk;
    }
  }

  function toggleDay(d: string) {
    const days: string[] = form["notifications.work_hours.days"];
    form["notifications.work_hours.days"] = days.includes(d)
      ? days.filter((x) => x !== d)
      : DAYS.filter((x) => x === d || days.includes(x));
  }

  async function setAutostart(enabled: boolean) {
    try {
      const now = await invoke<boolean>("set_autostart", { enabled });
      if (data) data.autostart = now;
    } catch (e) {
      saveErrors = [errorText(e)];
    }
  }

  // Favorites.
  const folders = $derived.by(() => {
    const out: Favorite[] = [];
    const walk = (ns: Favorite[]) => {
      for (const n of ns) {
        if (n.type === "folder") {
          if (!n.generated) out.push(n);
          walk(n.children ?? []);
        }
      }
    };
    walk(data?.favorites ?? []);
    return out;
  });

  const flat = $derived.by(() => {
    const out: { fav: Favorite; depth: number }[] = [];
    const walk = (ns: Favorite[], depth: number) => {
      for (const n of ns) {
        out.push({ fav: n, depth });
        walk(n.children ?? [], depth + 1);
      }
    };
    walk(data?.favorites ?? [], 0);
    return out;
  });

  async function addFavorite() {
    favBusy = true;
    favError = "";
    try {
      await invoke("add_favorite", {
        parent: favParent || null,
        file: favParent ? null : favFile || null,
        favorite: {
          title: favTitle,
          id: favId.trim() || null,
          kind: favKind,
          color: favUseColor ? favColor : null,
          icon: favIcon.trim() || null,
          ask_note: favAskNote,
        },
      });
      favTitle = "";
      favId = "";
      favIcon = "";
      favUseColor = false;
      favKind = "activity";
      favAskNote = false;
      await load(false);
    } catch (e) {
      favError = errorText(e);
    } finally {
      favBusy = false;
    }
  }

  async function removeFavorite(f: Favorite) {
    const what = f.type === "folder" ? `the folder “${f.title}” and everything in it` : `“${f.title}”`;
    if (!confirm(`Remove ${what} from ${fileName(f.file)}?`)) return;
    favError = "";
    try {
      await invoke("remove_favorite", { id: f.id });
      await load(false);
    } catch (e) {
      favError = errorText(e);
    }
  }

  function fileName(p: string): string {
    return p.split(/[\\/]/).pop() ?? p;
  }

  onMount(() => {
    load(true);
    const un = onStateChanged(() => load(false));
    return () => {
      un.then((u) => u());
    };
  });
</script>

<div class="layout">
  <nav>
    <div class="brand">aw-manual</div>
    {#each tabs as [id, label]}
      <button class:active={tab === id} onclick={() => (tab = id)}>
        {label}
        {#if id === "favorites" && data?.files.some((f) => f.status !== "ok")}<span class="badge">!</span>{/if}
        {#if id === "hotkeys" && data?.hotkey_errors.length}<span class="badge">!</span>{/if}
      </button>
    {/each}
    <div class="version muted">v{data?.version ?? ""}</div>
  </nav>

  <main>
    {#if loadError}
      <p class="error">{loadError}</p>
    {/if}
    {#if data}
      {#if data.config_state.state === "invalid"}
        <div class="banner error">
          <strong>config.yaml contains errors.</strong> The last valid configuration is active until they are fixed:
          <ul>
            {#each data.config_state.errors ?? [] as e}
              <li>{e.line ? `line ${e.line}: ` : ""}{e.message}</li>
            {/each}
          </ul>
        </div>
      {/if}

      {#if tab === "general"}
        <h2>General</h2>
        <section>
          <label>aw-server URL <input bind:value={form["server.url"]} placeholder="http://localhost:5600" /></label>
          <p class="hint">A local or remote aw-server. Entries are buffered while it is unreachable.</p>
          <label>Bucket ID <input bind:value={form["server.bucket_id"]} /></label>
          <p class="hint"><code>{"{hostname}"}</code> is replaced by this computer's name.</p>
          <label>Heartbeat interval <input bind:value={form["server.heartbeat_interval"]} class="short" /></label>
          <label>Log level
            <select bind:value={form["log.level"]}>
              {#each ["error", "warn", "info", "debug", "trace"] as l}<option value={l}>{l}</option>{/each}
            </select>
          </label>
          <label class="check"><input type="checkbox" bind:checked={form["cli.autostart_daemon"]} /> Start the daemon automatically when the CLI needs it</label>
          <label class="check"><input type="checkbox" checked={data.autostart} onchange={(e) => setAutostart(e.currentTarget.checked)} /> Start aw-manual at login</label>
        </section>
        <h3>Status</h3>
        <section class="info">
          <div>Server: <code>{data.status.server.url}</code> –
            {#if data.status.server.online === true}<span class="ok">connected</span>
            {:else if data.status.server.online === false}<span class="warn">unreachable</span>
            {:else}<span class="muted">checking…</span>{/if}
          </div>
          {#if data.status.server.pending > 0}<div>{data.status.server.pending} entries pending</div>{/if}
          {#if data.status.server.failed > 0}<div class="warn">{data.status.server.failed} entries were rejected by aw-server (see the log in <code>{data.log_dir}</code>)</div>{/if}
          <div>Configuration: <code>{data.config_dir}</code> <button onclick={() => invoke("open_config_dir")}>Open folder</button></div>
          <div class="muted">Logs: <code>{data.log_dir}</code></div>
        </section>
      {:else if tab === "tray"}
        <h2>Tray</h2>
        <section>
          <label>Text next to the icon
            <select bind:value={form["tray.display"]}>
              <option value="icon">Icon only</option>
              <option value="elapsed">Elapsed time (1h 23m)</option>
              <option value="since">Start time (since 09:30)</option>
              <option value="title">Activity title</option>
              <option value="template">Template</option>
            </select>
          </label>
          {#if form["tray.display"] === "template"}
            <label>Template <input bind:value={form["tray.template"]} /></label>
            <p class="hint">Placeholders: <code>{"{title} {path} {elapsed} {since} {note}"}</code></p>
          {/if}
          <label>Elapsed time format
            <select bind:value={form["tray.elapsed_format"]}>
              {#each ["1h 23m", "1:23", "1h 23m 45s", "1:23:45"] as f}<option value={f}>{f}</option>{/each}
            </select>
          </label>
          <label>Clock format
            <select bind:value={form["tray.time_format"]}>
              <option value="24h">24h (09:30)</option>
              <option value="12h">12h (9:30 AM)</option>
            </select>
          </label>
          <label class="check"><input type="checkbox" bind:checked={form["tray.color_icon"]} /> Tint the icon in the favorite's color</label>
          {#if data.os === "windows"}
            <p class="hint">Windows cannot show text in the notification area; the tooltip shows the full information.</p>
          {/if}
        </section>
      {:else if tab === "hotkeys"}
        <h2>Hotkeys</h2>
        <section>
          <p class="hint">Key combinations such as <code>Ctrl+Alt+Space</code>, or mouse buttons <code>Mouse4</code> … <code>Mouse9</code>. Leave empty to disable.</p>
          <label>Open popover / confirm <input bind:value={form["hotkeys.popover"]} class="mono" /></label>
          <label>Stop <input bind:value={form["hotkeys.stop"]} class="mono" /></label>
          <label>Resume last activity <input bind:value={form["hotkeys.resume"]} class="mono" /></label>
          {#if data.config.hotkeys.favorites.length}
            <p class="hint">Hotkeys for individual favorites (edit <code>hotkeys.favorites</code> in config.yaml):</p>
            <ul class="plain">
              {#each data.config.hotkeys.favorites as [id, hk]}<li><code>{hk}</code> → {id}</li>{/each}
            </ul>
          {/if}
          {#if data.wayland}
            <p class="hint">
              Wayland: hotkeys are registered through the global shortcuts portal where available. Otherwise bind keys in
              your compositor, e.g. Hyprland <code>bind = SUPER, T, exec, aw-manual popover</code> or for a mouse button
              <code>bind = , mouse:276, exec, aw-manual popover</code>.
            </p>
          {/if}
          {#each data.hotkey_errors as e}<p class="error">{e}</p>{/each}
        </section>
      {:else if tab === "notifications"}
        <h2>Notifications</h2>
        <section>
          <div class="row">
            <label class="check"><input type="checkbox" bind:checked={idleOn} /> Remind me when no activity has been running for</label>
            <input bind:value={idleValue} class="short" disabled={!idleOn} />
          </div>
          <div class="row">
            <label class="check"><input type="checkbox" bind:checked={longOn} /> Notify when the same activity has been running for</label>
            <input bind:value={longValue} class="short" disabled={!longOn} />
          </div>
          <label class="check"><input type="checkbox" bind:checked={form["notifications.after_restart"]} /> After a restart, offer to resume the last activity</label>
          <label class="check"><input type="checkbox" bind:checked={form["notifications.offline"]} /> Tell me when entries are being buffered (aw-server unreachable)</label>
        </section>
        <h3>Working hours (for the reminder)</h3>
        <section>
          <div class="days">
            {#each DAYS as d}
              <button class:active={form["notifications.work_hours.days"].includes(d)} onclick={() => toggleDay(d)}>{d}</button>
            {/each}
          </div>
          <div class="row">
            <label>From <input bind:value={form["notifications.work_hours.from"]} class="short" /></label>
            <label>To <input bind:value={form["notifications.work_hours.to"]} class="short" /></label>
          </div>
        </section>
      {:else if tab === "popover"}
        <h2>Popover – scroll mode</h2>
        <section>
          <p class="hint">Operate the popover with the hotkey and the scroll wheel only: scroll to select, confirm with the hotkey or by keeping the highlight on an item.</p>
          <label>Layout
            <select bind:value={form["popover.scroll.layout"]}>
              <option value="tree">All folders expanded, scroll through the leaves</option>
              <option value="levels">One folder level at a time</option>
            </select>
          </label>
          <label class="check"><input type="checkbox" bind:checked={form["popover.scroll.confirm_with_hotkey"]} /> Confirm by pressing the hotkey again</label>
          <div class="row">
            <label class="check"><input type="checkbox" bind:checked={form["popover.scroll.dwell_confirm"]} /> Confirm by dwell time</label>
            <input bind:value={form["popover.scroll.dwell_time"]} class="short" />
          </div>
          <label>Close without input after <input bind:value={form["popover.scroll.timeout"]} class="short" /></label>
          <label>Scroll sensitivity (threshold factor) <input type="number" step="0.1" min="0.1" bind:value={form["popover.scroll.sensitivity"]} class="short" /></label>
          <label class="check"><input type="checkbox" bind:checked={form["popover.scroll.wrap_around"]} /> Wrap around at the end of the list</label>
          <label class="check"><input type="checkbox" bind:checked={form["popover.scroll.open_at_cursor"]} /> Open under the mouse pointer</label>
          <label class="check"><input type="checkbox" bind:checked={form["popover.scroll.horizontal"]} /> Horizontal scrolling / wheel tilt: left = back, right = confirm</label>
          <div class="row">
            <label class="check"><input type="checkbox" bind:checked={afkOn} /> After returning from AFK (aw-watcher-afk), ask what I did if I was away for</label>
            <input bind:value={afkValue} class="short" disabled={!afkOn} />
          </div>
          <label>Initial highlight
            <select bind:value={form["popover.scroll.start_at"]}>
              <option value="running">The running activity</option>
              <option value="first">The first favorite</option>
            </select>
          </label>
        </section>
      {:else if tab === "favorites"}
        <h2>Favorites</h2>
        <p class="hint">
          Favorites are read from <code>{data.favorite_dirs.join(", ")}</code>. Changes made here are written to the file
          that defines the favorite; new top-level favorites go to <code>{data.ui_file}</code>. Generated files are read-only.
        </p>
        {#each data.files.filter((f) => f.status !== "ok") as f}
          <div class="banner error">
            <strong>{fileName(f.path)}</strong>
            {f.status === "last_valid" ? "is invalid; its last valid version is used:" : "is invalid and ignored:"}
            <ul>
              {#each f.errors ?? [] as e}<li>{e.line ? `line ${e.line}: ` : ""}{e.message}</li>{/each}
            </ul>
          </div>
        {/each}
        {#each data.warnings as w}<p class="warn">{w}</p>{/each}

        <table class="favs">
          <tbody>
            {#each flat as { fav, depth } (fav.id)}
              <tr>
                <td style={`padding-left: ${depth * 20 + 6}px`}>
                  <span class="swatch" style={`background: ${fav.color?.value ?? "transparent"}`} title={fav.color ? `${fav.color.value} (${fav.color.source})` : ""}></span>
                  {iconGlyph(fav.icon?.value)}
                  <span class:folder={fav.type === "folder"}>{fav.title}</span>
                  {#if fav.kind === "status"}<span class="tag">status</span>{/if}
                </td>
                <td><code class="muted">{fav.id}</code></td>
                <td class="muted small nowrap">{fileName(fav.file)}:{fav.line}{#if fav.generated}<br />generated{/if}</td>
                <td class="actions">
                  {#if !fav.generated}<button class="small" onclick={() => removeFavorite(fav)}>Remove</button>{/if}
                </td>
              </tr>
            {:else}
              <tr><td class="muted">No favorites yet.</td></tr>
            {/each}
          </tbody>
        </table>

        <h3>Add a favorite</h3>
        <section class="add">
          <label>In folder
            <select bind:value={favParent}>
              <option value="">(top level)</option>
              {#each folders as f}<option value={f.id}>{f.path.join(PATH_SEPARATOR)}</option>{/each}
            </select>
          </label>
          {#if !favParent}
            <label>File
              <select bind:value={favFile}>
                <option value="">{data.ui_file}{data.writable_files.some((f) => fileName(f) === data?.ui_file) ? "" : " (new)"}</option>
                {#each data.writable_files.filter((f) => fileName(f) !== data?.ui_file) as f}<option value={f}>{fileName(f)}</option>{/each}
              </select>
            </label>
          {/if}
          <label>Title <input bind:value={favTitle} placeholder="e.g. Development" /></label>
          <label>ID <input bind:value={favId} placeholder="derived from the title" class="mono" /></label>
          <label>Kind
            <select bind:value={favKind}>
              <option value="activity">Activity</option>
              <option value="status">Status (break, meeting, …)</option>
            </select>
          </label>
          <div class="row">
            <label class="check"><input type="checkbox" bind:checked={favUseColor} /> Own color</label>
            <input type="color" bind:value={favColor} disabled={!favUseColor} />
          </div>
          <label>Icon <input bind:value={favIcon} placeholder="e.g. code, coffee or an emoji" /></label>
          <label class="check"><input type="checkbox" bind:checked={favAskNote} /> Always ask for a note when started (e.g. a ticket number)</label>
          <div><button class="primary" disabled={!favTitle.trim() || favBusy} onclick={addFavorite}>Add favorite</button></div>
          {#if favError}<p class="error">{favError}</p>{/if}
        </section>
      {/if}

      {#if tab !== "favorites"}
        <div class="savebar">
          {#each saveErrors as e}<p class="error">{e}</p>{/each}
          <button class="primary" disabled={changes.length === 0 || saving} onclick={save}>Save</button>
          <button disabled={changes.length === 0 || saving} onclick={revert}>Revert</button>
          {#if saved}<span class="ok">Saved to config.yaml</span>{/if}
          {#if changes.length}<span class="muted">{changes.length} unsaved change{changes.length === 1 ? "" : "s"}</span>{/if}
        </div>
      {/if}
    {:else if !loadError}
      <p class="muted">Loading…</p>
    {/if}
  </main>
</div>

<style>
  .layout {
    display: flex;
    height: 100vh;
  }
  nav {
    width: 190px;
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 14px 10px;
    background: var(--bg-soft);
    border-right: 1px solid var(--border);
  }
  nav .brand {
    font-weight: 700;
    font-size: 16px;
    padding: 4px 10px 14px;
  }
  nav button {
    text-align: left;
    border: none;
    background: transparent;
    padding: 8px 10px;
    display: flex;
    justify-content: space-between;
  }
  nav button.active {
    background: var(--highlight);
    font-weight: 600;
  }
  nav .version {
    margin-top: auto;
    padding: 0 10px;
    font-size: 12px;
  }
  .badge {
    color: var(--danger);
    font-weight: 700;
  }
  main {
    flex: 1;
    overflow-y: auto;
    padding: 10px 28px 80px;
  }
  h2 {
    margin: 14px 0 10px;
  }
  h3 {
    margin: 24px 0 8px;
    font-size: 15px;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-width: 640px;
  }
  section.info {
    gap: 6px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-weight: 500;
  }
  label.check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    font-weight: normal;
  }
  .row {
    display: flex;
    gap: 14px;
    align-items: center;
  }
  .row label {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }
  input.short {
    width: 110px;
  }
  .mono,
  code {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12.5px;
  }
  .hint {
    margin: -4px 0 4px;
    font-size: 12.5px;
    color: var(--fg-soft);
  }
  .banner {
    border: 1px solid var(--danger);
    border-radius: 8px;
    padding: 8px 12px;
    margin: 10px 0;
  }
  .banner ul {
    margin: 6px 0 0;
    padding-left: 18px;
  }
  .ok {
    color: var(--ok);
  }
  .warn {
    color: var(--warn);
  }
  .days {
    display: flex;
    gap: 6px;
  }
  .days button.active {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-fg);
  }
  .favs {
    width: 100%;
    border-collapse: collapse;
    margin: 6px 0;
  }
  .favs td {
    padding: 5px 6px;
    border-bottom: 1px solid var(--border);
    vertical-align: middle;
  }
  .favs .actions {
    text-align: right;
    width: 1%;
    white-space: nowrap;
  }
  .favs .folder {
    font-weight: 700;
  }
  .small {
    font-size: 12px;
  }
  .nowrap {
    white-space: nowrap;
  }
  button.small {
    padding: 2px 8px;
  }
  .swatch {
    display: inline-block;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 1px solid var(--border);
    margin-right: 4px;
  }
  .tag {
    font-size: 11px;
    padding: 0 6px;
    border-radius: 10px;
    border: 1px solid var(--border);
    color: var(--fg-soft);
    margin-left: 6px;
  }
  ul.plain {
    margin: 0;
    padding-left: 18px;
  }
  .savebar {
    position: fixed;
    bottom: 0;
    left: 190px;
    right: 0;
    display: flex;
    gap: 10px;
    align-items: center;
    flex-wrap: wrap;
    padding: 10px 28px;
    background: var(--bg);
    border-top: 1px solid var(--border);
  }
  .savebar .error {
    width: 100%;
    margin: 0;
  }
</style>
