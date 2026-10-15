<script lang="ts">
  import { onMount, tick } from "svelte";
  import { api, errorText, onClosed, onNav, onOpened, onStateChanged, type Gap, type Opened } from "../lib/api";
  import { formatClock, formatElapsed, iconGlyph, parseDuration, PATH_SEPARATOR } from "../lib/format";
  import { search } from "../lib/fuzzy";
  import { DrillDown, ExpandedTree, type NavItem, type ScrollNav } from "../lib/nav";
  import { ancestorIds, parentId, visibleRows, type Row } from "../lib/tree";
  import type { Favorite, NavCommand, Snapshot } from "../lib/types";
  import { WheelSteps } from "../lib/wheel";

  type Mode = "scroll" | "keyboard";

  let snap = $state<Snapshot | null>(null);
  let mode = $state<Mode>("scroll");
  let query = $state("");
  let note = $state("");
  let noteOpen = $state(false);
  /** A favorite waiting for its note (ask_note, Shift+Enter, right click). */
  let noteFor = $state<Favorite | null>(null);
  /**
   * Question after a restart or unlock: "gap" logs what happened meanwhile,
   * "resume" offers to continue an activity from now, "choose" asks for the
   * activity from now on.
   */
  type Continue = { id: string; path: string[]; note: string | null; last: boolean };
  let prompt = $state<{ step: "gap" | "resume" | "choose"; gap: Gap; next: Continue | null } | null>(null);
  let handledSeq = 0;
  let retro = $state(false);
  let range = $state("");
  let preview = $state<{ text?: string; error?: string } | null>(null);
  let error = $state("");
  let busy = $state(false);
  let now = $state(Date.now());
  let expanded = $state<string[]>([]);
  let kbIndex = $state(0);
  let navVersion = $state(0);
  let dwellArmed = $state(false);
  let dwellKey = $state(0);

  let drill: ScrollNav = new DrillDown([], null, false);
  const wheel = new WheelSteps();
  let dwellTimer: ReturnType<typeof setTimeout> | undefined;
  let timeoutTimer: ReturnType<typeof setTimeout> | undefined;

  let searchEl = $state<HTMLInputElement>();
  let noteEl = $state<HTMLInputElement>();
  let rangeEl = $state<HTMLInputElement>();
  let listEl = $state<HTMLElement>();

  const running = $derived(snap?.status.running ?? null);
  const dwellMs = $derived(snap ? parseDuration(snap.scroll.dwell_time) : 2000);
  const timeoutMs = $derived(snap ? parseDuration(snap.scroll.timeout) : 10000);
  const results = $derived(snap && query.trim() ? search(snap.favorites, query) : []);
  const rows = $derived<Row[]>(snap ? visibleRows(snap.favorites, new Set(expanded)) : []);
  const kbList = $derived<Row[]>(query.trim() ? results.map((fav) => ({ fav, depth: 0, expanded: false })) : rows);
  const navItems = $derived.by<NavItem[]>(() => {
    void navVersion;
    return drill.items();
  });
  /** Index of each heading / leaf in navItems (tree layout rendering). */
  const navIndexOf = $derived(
    new Map(navItems.flatMap((it, i) => (it.kind === "heading" || it.kind === "leaf" ? [[`${it.kind}:${it.fav.id}`, i] as [string, number]] : []))),
  );
  /** Height of a heading row; sticky headings stack by depth. */
  const HEADING_PX = 24;

  function rowStyle(item: NavItem): string | undefined {
    if (item.kind === "back" || item.kind === "stop" || item.kind === "cancel" || !item.depth) {
      return item.kind === "heading" ? `top: 0; z-index: 20` : undefined;
    }
    const pad = `padding-left: ${10 + item.depth * 18}px`;
    if (item.kind === "heading") return `${pad}; top: ${item.depth * HEADING_PX}px; z-index: ${20 - item.depth}`;
    return pad;
  }

  const navIndex = $derived.by(() => {
    void navVersion;
    return drill.index;
  });
  const highlightedPath = $derived.by(() => {
    void navVersion;
    if (mode === "scroll") return drill.highlightedPath();
    return kbList[kbIndex]?.fav.path ?? [];
  });
  const source = $derived(mode === "scroll" ? "popover-scroll" : "popover");

  function elapsedText(): string {
    if (!running || !snap) return "";
    return formatElapsed((now - Date.parse(running.start)) / 1000, snap.tray.elapsed_format);
  }

  async function refresh() {
    try {
      snap = await api.snapshot();
      drill.update(snap.favorites, snap.status.running?.id ?? null);
      wheel.setSensitivity(snap.scroll.sensitivity);
      navVersion++;
    } catch (e) {
      error = errorText(e);
    }
  }

  function clearTimers() {
    clearTimeout(dwellTimer);
    clearTimeout(timeoutTimer);
    dwellArmed = false;
  }

  /** Closes the popover after the timeout without input (scroll mode only). */
  function restartTimeout() {
    clearTimeout(timeoutTimer);
    if (mode === "scroll" && snap) timeoutTimer = setTimeout(() => close(), timeoutMs);
  }

  /** Dwell confirmation starts only after a scroll movement. */
  function armDwell() {
    clearTimeout(dwellTimer);
    dwellArmed = false;
    if (!snap?.scroll.dwell_confirm || mode !== "scroll") return;
    dwellArmed = true;
    dwellKey++;
    dwellTimer = setTimeout(() => {
      dwellArmed = false;
      confirmScroll();
    }, dwellMs);
  }

  function findFavorite(nodes: Favorite[], id: string): Favorite | null {
    for (const f of nodes) {
      if (f.id === id) return f;
      const c = findFavorite(f.children ?? [], id);
      if (c) return c;
    }
    return null;
  }

  async function reset(opened?: Opened | null) {
    if (opened) {
      if (opened.seq <= handledSeq) return;
      handledSeq = opened.seq;
    }
    clearTimers();
    prompt = null;
    mode = "scroll";
    query = "";
    note = "";
    noteOpen = false;
    noteFor = null;
    retro = false;
    range = "";
    preview = null;
    error = "";
    busy = false;
    expanded = [];
    kbIndex = 0;
    await refresh();
    if (!snap) return;
    const Nav = snap.scroll.layout === "levels" ? DrillDown : ExpandedTree;
    drill = new Nav(snap.favorites, snap.status.running?.id ?? null, snap.scroll.wrap_around);
    drill.startAt(snap.scroll.start_at);
    navVersion++;
    const fav = opened?.favorite ? findFavorite(snap.favorites, opened.favorite) : null;
    if (fav && fav.type === "leaf") {
      await askNote(fav);
      return;
    }
    if (opened?.note && snap.status.running) {
      await openNote();
      return;
    }
    if (opened?.gap) {
      await startGap(opened.gap);
      return;
    }
    restartTimeout();
    await tick();
    listEl?.focus();
  }

  async function close() {
    clearTimers();
    try {
      await api.hide();
    } catch (e) {
      error = errorText(e);
    }
  }

  function lastActivity(gap: Gap): Continue | null {
    return gap.last ? { id: gap.last.id, path: gap.last.path, note: gap.last.note, last: true } : null;
  }

  /** Step 1: what happened during the gap (a retroactive entry). */
  async function startGap(gap: Gap) {
    if (!gap.range) {
      // Too short to log anything: only ask what to continue with.
      const last = lastActivity(gap);
      prompt = { step: "gap", gap, next: null };
      if (last) await askContinue(last);
      else await chooseNow();
      return;
    }
    prompt = { step: "gap", gap, next: null };
    await enterKeyboard();
    retro = true;
    range = gap.range;
    // Preselect the activity from before the gap: Enter logs it.
    if (gap.last) {
      const id = gap.last.id;
      if (!expanded.includes(id)) expanded = [...new Set([...expanded, ...ancestorIds(id)])];
      await tick();
      const i = kbList.findIndex((r) => r.fav.id === id);
      if (i !== -1) kbIndex = i;
    }
    await tick();
    searchEl?.focus();
  }

  /**
   * Esc in step 1: nothing logged. Offer the activity from before the gap
   * (after unlocking: the one still running), otherwise ask what's next.
   */
  async function skipGap() {
    if (!prompt) return;
    const last = lastActivity(prompt.gap);
    if (last) await askContinue(last);
    else await chooseNow();
  }

  /** Step 2: continue with this activity from now? */
  async function askContinue(next: Continue) {
    if (!prompt) return;
    prompt = { ...prompt, step: "resume", next };
    retro = false;
    range = "";
    query = "";
    note = "";
    noteOpen = false;
    noteFor = null;
    // Highlight the activity to continue: Enter continues it, or move the
    // highlight to start another one.
    if (!expanded.includes(next.id)) expanded = [...new Set([...expanded, ...ancestorIds(next.id)])];
    await tick();
    const i = kbList.findIndex((r) => r.fav.id === next.id);
    if (i !== -1) kbIndex = i;
    await tick();
    searchEl?.focus();
  }

  async function continueYes() {
    const next = prompt?.next;
    if (!next || busy) return;
    busy = true;
    error = "";
    try {
      // The last activity before a restart is resumed as it was (also ad-hoc
      // activities); a favorite picked for the gap starts from now.
      if (next.last && prompt?.gap.reason === "restart") await api.resume();
      else if (next.last && running?.id === next.id) {
        // After unlocking: the activity is still running, nothing to change.
      } else await api.start(next.id, next.note, "popover");
      await close();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  /** Step 3: what are you doing from now on? */
  async function chooseNow() {
    if (!prompt) return;
    prompt = { ...prompt, step: "choose", next: null };
    retro = false;
    range = "";
    note = "";
    await enterKeyboard();
  }

  function gapText(gap: Gap): string {
    const fmt = snap?.tray.time_format ?? "24h";
    const from = formatClock(new Date(gap.start), fmt);
    const to = formatClock(new Date(gap.end), fmt);
    const len = formatElapsed(gap.seconds, "1h 23m");
    const what = gap.reason === "restart" ? "aw-manual wasn't running" : "The screen was locked";
    return `${what} from ${from} to ${to} (${len}).`;
  }

  /** Switches to keyboard mode with the note field focused. */
  async function openNote() {
    await enterKeyboard();
    noteOpen = true;
    await tick();
    noteEl?.focus();
  }

  /** Asks for a note before starting the favorite (Enter in the note field starts it). */
  async function askNote(fav: Favorite) {
    if (fav.type !== "leaf") return;
    noteFor = fav;
    await openNote();
  }

  async function startFavorite(fav: Favorite, asked = false) {
    if (busy) return;
    if (fav.ask_note && !asked && !(mode === "keyboard" && note.trim())) {
      await askNote(fav);
      return;
    }
    busy = true;
    error = "";
    clearTimers();
    try {
      const n = mode === "keyboard" ? note.trim() || null : null;
      if (retro && mode === "keyboard") {
        if (!range.trim()) {
          error = "Enter a time range first (e.g. -30m or 10:00-11:15).";
          rangeEl?.focus();
          return;
        }
        await api.log(fav.id, range, n);
      } else {
        await api.start(fav.id, n, source);
      }
      if (prompt?.step === "gap") {
        // Logged what happened meanwhile; next: continue with it from now?
        await askContinue({ id: fav.id, path: fav.path, note: n, last: false });
        return;
      }
      await close();
    } catch (e) {
      error = errorText(e);
      restartTimeout();
    } finally {
      busy = false;
    }
  }

  async function stopRunning() {
    if (busy || !running) return;
    busy = true;
    try {
      await api.stop(source);
      await close();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function applyNote() {
    if (noteFor) {
      const fav = noteFor;
      await startFavorite(fav, true);
      return;
    }
    if (running && !retro) {
      busy = true;
      try {
        await api.setNote(note.trim() || null);
        await close();
      } catch (e) {
        error = errorText(e);
      } finally {
        busy = false;
      }
    } else {
      searchEl?.focus();
    }
  }

  async function openSettings() {
    clearTimers();
    try {
      await api.openSettings();
    } catch (e) {
      error = errorText(e);
    }
  }

  function confirmScroll() {
    const action = drill.confirm();
    navVersion++;
    restartTimeout();
    switch (action.kind) {
      case "start":
        startFavorite(action.fav);
        break;
      case "stop":
        stopRunning();
        break;
      case "close":
        close();
        break;
    }
  }

  /** Abstract navigation commands (scroll wheel, keys, `aw-manual nav`, hotkey). */
  function nav(cmd: NavCommand) {
    if (busy) return;
    if (prompt?.step === "resume" && cmd === "cancel") {
      close();
      return;
    }
    if (mode === "scroll") {
      switch (cmd) {
        case "next":
          drill.next();
          navVersion++;
          armDwell();
          restartTimeout();
          break;
        case "prev":
          drill.prev();
          navVersion++;
          armDwell();
          restartTimeout();
          break;
        case "confirm":
          clearTimeout(dwellTimer);
          dwellArmed = false;
          confirmScroll();
          break;
        case "back":
          clearTimeout(dwellTimer);
          dwellArmed = false;
          drill.back();
          navVersion++;
          restartTimeout();
          break;
        case "cancel":
          close();
          break;
      }
      return;
    }
    switch (cmd) {
      case "next":
        kbIndex = Math.min(kbIndex + 1, kbList.length - 1);
        break;
      case "prev":
        kbIndex = Math.max(kbIndex - 1, 0);
        break;
      case "confirm":
        activateKeyboard();
        break;
      case "back":
        collapseOrParent();
        break;
      case "cancel":
        close();
        break;
    }
  }

  function activateKeyboard() {
    const row = kbList[kbIndex];
    if (!row) return;
    if (row.fav.type === "folder") toggle(row.fav.id);
    else if (prompt?.step === "resume" && row.fav.id === prompt.next?.id) continueYes();
    else startFavorite(row.fav);
  }

  /** IDs of all folders (the tree layout shows everything expanded). */
  function allFolderIds(nodes: Favorite[]): string[] {
    return nodes.flatMap((f) => (f.type === "folder" ? [f.id, ...allFolderIds(f.children ?? [])] : []));
  }

  function toggle(id: string) {
    expanded = expanded.includes(id) ? expanded.filter((x) => x !== id) : [...expanded, id];
  }

  function expandOrChild() {
    const row = kbList[kbIndex];
    if (!row || row.fav.type !== "folder" || query.trim()) return;
    if (!row.expanded) toggle(row.fav.id);
    else kbIndex = Math.min(kbIndex + 1, kbList.length - 1);
  }

  function collapseOrParent() {
    const row = kbList[kbIndex];
    if (!row || query.trim()) return;
    if (row.fav.type === "folder" && row.expanded) {
      toggle(row.fav.id);
      return;
    }
    const p = parentId(row.fav.id);
    const i = p ? kbList.findIndex((r) => r.fav.id === p) : -1;
    if (i !== -1) kbIndex = i;
  }

  /** Switches to keyboard mode (search, tree, note, retroactive entries). */
  async function enterKeyboard(initial?: string) {
    if (mode !== "keyboard") {
      mode = "keyboard";
      clearTimers();
      // Continue where scroll mode was: open its folders and highlight the item.
      const item = drill.current();
      const id = item.kind === "folder" || item.kind === "leaf" ? item.fav.id : running?.id;
      if (snap?.scroll.layout !== "levels") expanded = allFolderIds(snap?.favorites ?? []);
      if (id) {
        if (snap?.scroll.layout === "levels") expanded = ancestorIds(id);
        await tick();
        const i = rows.findIndex((r) => r.fav.id === id);
        kbIndex = i === -1 ? 0 : i;
      }
    }
    if (initial !== undefined) query = initial;
    await tick();
    searchEl?.focus();
  }

  function onKeydown(e: KeyboardEvent) {
    const ctrl = e.ctrlKey || e.metaKey;
    const key = e.key.toLowerCase();
    // "Continue with …?": Shift+Y continues, Shift+N asks for another
    // activity (plain letters go to the search, which ignores case), Esc
    // closes while the search is empty. Enter starts the highlighted
    // activity, which is the one to continue unless the highlight was moved.
    if (prompt?.step === "resume" && !ctrl && !e.altKey && e.target !== noteEl) {
      if (e.shiftKey && key === "y") {
        e.preventDefault();
        continueYes();
        return;
      }
      if (e.shiftKey && key === "n") {
        e.preventDefault();
        chooseNow();
        return;
      }
      if (e.key === "Escape" && !query) {
        e.preventDefault();
        close();
        return;
      }
    }
    if (prompt?.step === "gap" && e.key === "Escape" && !query && e.target !== noteEl) {
      e.preventDefault();
      skipGap();
      return;
    }
    if (ctrl && key === "s") {
      e.preventDefault();
      stopRunning();
      return;
    }
    if (ctrl && key === "n") {
      e.preventDefault();
      openNote();
      return;
    }
    if (ctrl && key === "r") {
      e.preventDefault();
      enterKeyboard().then(async () => {
        retro = !retro;
        await tick();
        (retro ? rangeEl : searchEl)?.focus();
      });
      return;
    }
    if (ctrl && e.key === ",") {
      e.preventDefault();
      openSettings();
      return;
    }
    if (e.key === "F5") {
      e.preventDefault();
      return;
    }

    if (mode === "scroll") {
      switch (e.key) {
        case "ArrowDown":
          e.preventDefault();
          nav("next");
          return;
        case "ArrowUp":
          e.preventDefault();
          nav("prev");
          return;
        case "Enter":
        case "ArrowRight":
          e.preventDefault();
          if (e.key === "Enter" && e.shiftKey) {
            // Shift+Enter: ask for a note first, then start the highlighted leaf.
            const item = drill.current();
            if (item.kind === "leaf") askNote(item.fav);
            return;
          }
          nav("confirm");
          return;
        case "Backspace":
        case "ArrowLeft":
          e.preventDefault();
          nav("back");
          return;
        case "Escape":
          e.preventDefault();
          nav("cancel");
          return;
        case "Tab":
          e.preventDefault();
          enterKeyboard();
          return;
      }
      if (e.key.length === 1 && !ctrl && !e.altKey) {
        e.preventDefault();
        enterKeyboard(e.key);
      }
      return;
    }

    // Keyboard mode.
    const target = e.target as HTMLElement | null;
    if (target === noteEl) {
      if (e.key === "Enter") {
        e.preventDefault();
        applyNote();
      } else if (e.key === "Escape") {
        e.preventDefault();
        noteOpen = false;
        noteFor = null;
        note = "";
        searchEl?.focus();
      }
      return;
    }
    if (target === rangeEl) {
      if (e.key === "Enter") {
        e.preventDefault();
        searchEl?.focus();
      } else if (e.key === "Escape") {
        e.preventDefault();
        retro = false;
        range = "";
        searchEl?.focus();
      }
      return;
    }
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        nav("next");
        break;
      case "ArrowUp":
        e.preventDefault();
        nav("prev");
        break;
      case "PageDown":
        e.preventDefault();
        kbIndex = Math.min(kbIndex + 8, kbList.length - 1);
        break;
      case "PageUp":
        e.preventDefault();
        kbIndex = Math.max(kbIndex - 8, 0);
        break;
      case "Enter":
        e.preventDefault();
        if (e.shiftKey) {
          const row = kbList[kbIndex];
          if (row) askNote(row.fav);
        } else {
          nav("confirm");
        }
        break;
      case "ArrowRight":
        if (!query) {
          e.preventDefault();
          expandOrChild();
        }
        break;
      case "ArrowLeft":
        if (!query) {
          e.preventDefault();
          collapseOrParent();
        }
        break;
      case "Escape":
        e.preventDefault();
        if (query) {
          query = "";
          kbIndex = 0;
        } else {
          close();
        }
        break;
    }
  }

  /** Wheel in keyboard mode: moves the highlight through the leaves (search results or tree). */
  function wheelKeyboard(dir: 1 | -1) {
    const n = kbList.length;
    const wrap = snap?.scroll.wrap_around ?? true;
    for (let k = 1; k < n; k++) {
      const i = kbIndex + dir * k;
      if (!wrap && (i < 0 || i >= n)) return;
      const j = ((i % n) + n) % n;
      if (kbList[j].fav.type === "leaf") {
        kbIndex = j;
        return;
      }
    }
  }

  function onWheel(e: WheelEvent) {
    e.preventDefault();
    if (busy) return;
    const t = performance.now();
    if (mode === "keyboard") {
      const step = wheel.vertical(e.deltaY, e.deltaMode, t);
      if (step !== 0) wheelKeyboard(step > 0 ? 1 : -1);
      return;
    }
    if (snap?.scroll.horizontal && Math.abs(e.deltaX) > Math.abs(e.deltaY)) {
      const h = wheel.horizontal(e.deltaX, e.deltaMode, t);
      if (h < 0) nav("back");
      else if (h > 0) nav("confirm");
      return;
    }
    const step = wheel.vertical(e.deltaY, e.deltaMode, t);
    if (step > 0) nav("next");
    else if (step < 0) nav("prev");
  }

  function clickNav(i: number) {
    if (navItems[i]?.kind === "heading") return;
    drill.index = i;
    navVersion++;
    clearTimeout(dwellTimer);
    dwellArmed = false;
    confirmScroll();
  }

  function clickRow(i: number) {
    kbIndex = i;
    activateKeyboard();
  }

  // Keep the highlighted item visible.
  $effect(() => {
    void navIndex;
    void kbIndex;
    void mode;
    tick().then(() => {
      const el = listEl?.querySelector<HTMLElement>(".hl");
      if (!el || !listEl) return;
      // Leave room for the sticky headings above the item (tree layout).
      const covered = Number(el.dataset.depth ?? 0) * HEADING_PX;
      const list = listEl.getBoundingClientRect();
      const r = el.getBoundingClientRect();
      if (r.top - list.top < covered) listEl.scrollTop -= covered - (r.top - list.top);
      else if (r.bottom > list.bottom) listEl.scrollTop += r.bottom - list.bottom;
    });
  });

  // New search: highlight the best match.
  $effect(() => {
    void query;
    kbIndex = 0;
  });

  // Preview of the retroactive time range.
  $effect(() => {
    const r = range.trim();
    if (!retro || !r) {
      preview = null;
      return;
    }
    const handle = setTimeout(async () => {
      try {
        preview = { text: (await api.previewRange(r)).text };
      } catch (e) {
        preview = { error: errorText(e) };
      }
    }, 120);
    return () => clearTimeout(handle);
  });

  onMount(() => {
    const unlisten = [
      onOpened((o) => reset(o)),
      onClosed(() => clearTimers()),
      onNav((cmd) => nav(cmd)),
      onStateChanged(() => refresh()),
    ];
    const ticker = setInterval(() => (now = Date.now()), 1000);
    // The page may finish loading after the first "opened" event.
    api.lastOpened().then((o) => reset(o), () => reset());
    window.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      clearInterval(ticker);
      window.removeEventListener("wheel", onWheel);
      unlisten.forEach((p) => p.then((u) => u()));
    };
  });

  function dot(fav: Favorite) {
    return fav.color?.value ?? "transparent";
  }
</script>

{#snippet navRow(item: NavItem, i: number)}
<!-- svelte-ignore a11y_click_events_have_key_events -->
<li
  role="option"
  aria-selected={i === navIndex}
  class:hl={i === navIndex}
  class={item.kind}
  style={rowStyle(item)}
  data-depth={item.kind === "leaf" && snap?.scroll.layout !== "levels" ? item.depth : undefined}
  onclick={() => clickNav(i)}
  oncontextmenu={(e) => {
    e.preventDefault();
    if (item.kind === "leaf") askNote(item.fav);
  }}
>
  {#if item.kind === "back"}
    <span class="label">‹ Back</span>
  {:else if item.kind === "stop"}
    <span class="label">■ Stop</span>
  {:else if item.kind === "cancel"}
    <span class="label">✕ Cancel</span>
  {:else if item.kind === "heading"}
    <span class="dot small" style={`background: ${dot(item.fav)}`}></span>
    <span class="icon">{iconGlyph(item.fav.icon?.value)}</span>
    <span class="label">{item.fav.title}</span>
  {:else}
    <span class="dot small" style={`background: ${dot(item.fav)}`}></span>
    <span class="icon">{iconGlyph(item.fav.icon?.value)}</span>
    <span class="label">{item.fav.title}</span>
    {#if item.kind === "folder"}<span class="chev">›</span>{/if}
    {#if item.kind === "leaf" && item.fav.ask_note}<span class="tag" title="Asks for a note">note</span>{/if}
    {#if item.kind === "leaf" && item.fav.id === running?.id}<span class="tag">running</span>{/if}
    {#if item.kind === "leaf" && item.fav.kind === "status"}<span class="tag">status</span>{/if}
  {/if}
  {#if i === navIndex && dwellArmed}
    {#key dwellKey}<span class="dwell"></span>{/key}
  {/if}
</li>
{/snippet}

{#snippet branch(nodes: Favorite[])}
  {#each nodes as f (f.id)}
    {#if f.type === "folder"}
      {@const i = navIndexOf.get("heading:" + f.id) ?? -1}
      <li class="group" role="presentation">
        <ul role="group">
          {#if i !== -1}{@render navRow(navItems[i], i)}{/if}
          {@render branch(f.children ?? [])}
        </ul>
      </li>
    {:else}
      {@const i = navIndexOf.get("leaf:" + f.id) ?? -1}
      {#if i !== -1}{@render navRow(navItems[i], i)}{/if}
    {/if}
  {/each}
{/snippet}

<svelte:window onkeydown={onKeydown} />

<div class="pop" class:keyboard={mode === "keyboard"} style={`--dwell: ${dwellMs}ms`}>
  <header>
    {#if running}
      <div class="running">
        <span class="dot" style={`background: ${running.color ?? "var(--accent)"}`}></span>
        <div class="what">
          <div class="title">
            {#if iconGlyph(running.icon)}<span class="icon">{iconGlyph(running.icon)}</span>{/if}
            {running.path.join(PATH_SEPARATOR)}
            {#if running.kind === "status"}<span class="tag">status</span>{/if}
          </div>
          <div class="sub muted">
            since {snap ? formatClock(new Date(running.start), snap.tray.time_format) : ""} · {elapsedText()}
            {#if running.note}· {running.note}{/if}
          </div>
        </div>
        <button onclick={stopRunning} disabled={busy} title="Stop (Ctrl+S)">■ Stop</button>
        <button class="settings" onclick={openSettings} title="Settings (Ctrl+,)" aria-label="Settings">⚙</button>
      </div>
    {:else}
      <div class="running idle">
        <span class="what muted">No activity running</span>
        <button class="settings" onclick={openSettings} title="Settings (Ctrl+,)" aria-label="Settings">⚙</button>
      </div>
    {/if}
    {#if prompt}
      <div class="prompt">
        {#if prompt.step === "gap"}
          <div class="q">{gapText(prompt.gap)} What were you doing?</div>
          <div class="muted">
            {#if prompt.gap.last}
              Pick a favorite to log this time (preselected: {prompt.gap.last.path.join(PATH_SEPARATOR)}), or press <kbd>Esc</kbd> to skip.
            {:else}
              Pick a favorite to log this time, or press <kbd>Esc</kbd> to skip.
            {/if}
          </div>
        {:else if prompt.step === "resume" && prompt.next}
          <div class="q">Continue with {prompt.next.path.join(PATH_SEPARATOR)} from now?</div>
          <div class="buttons">
            <button class="primary" onclick={continueYes} disabled={busy}>Yes, continue <kbd>Shift</kbd>+<kbd>Y</kbd></button>
            <button onclick={chooseNow} disabled={busy}>No, something else <kbd>Shift</kbd>+<kbd>N</kbd></button>
            <button onclick={close} disabled={busy}>Nothing <kbd>Esc</kbd></button>
          </div>
          <div class="muted">Or pick another activity below and press <kbd>Enter</kbd> to start it instead.</div>
        {:else if prompt.step === "choose"}
          <div class="q">What are you doing now?</div>
        {/if}
      </div>
    {/if}
    {#if snap && snap.status.server.online === false}
      <div class="offline">aw-server unreachable – {snap.status.server.pending} entries pending</div>
    {/if}
  </header>

  <div class="search">
    <input
      bind:this={searchEl}
      bind:value={query}
      placeholder={mode === "scroll" ? "Type to search…" : "Search favorites…"}
      onfocus={() => enterKeyboard()}
      spellcheck="false"
      autocomplete="off"
    />
    {#if retro}<span class="tag retro">retroactive</span>{/if}
  </div>

  {#if retro}
    <div class="retro-row">
      <input bind:this={rangeEl} bind:value={range} placeholder="-30m · 10:00-11:15 · since 9:30 · 10:00 +45m" spellcheck="false" />
      <span class="preview" class:error={preview?.error}>{preview?.text ?? preview?.error ?? ""}</span>
    </div>
  {/if}

  <ul class="list" bind:this={listEl} tabindex="-1" role="listbox" aria-label="Favorites">
    {#if mode === "scroll" && !query && snap?.scroll.layout !== "levels"}
      <!-- Tree layout: nested groups, so the headings of the highlighted item stay visible (sticky). -->
      {#if navItems[0]?.kind === "stop"}{@render navRow(navItems[0], 0)}{/if}
      {@render branch(snap?.favorites ?? [])}
      {@render navRow(navItems[navItems.length - 1], navItems.length - 1)}
    {:else if mode === "scroll" && !query}
      {#each navItems as item, i (i + ":" + (item.kind === "folder" || item.kind === "leaf" || item.kind === "heading" ? item.fav.id : item.kind))}
        {@render navRow(item, i)}
      {/each}
    {:else}
      {#each kbList as row, i (row.fav.id)}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <li role="option" aria-selected={i === kbIndex} class:hl={i === kbIndex} class={row.fav.type} style={`padding-left: ${10 + row.depth * 18}px`} onclick={() => clickRow(i)} oncontextmenu={(e) => {
            e.preventDefault();
            askNote(row.fav);
          }}>
          {#if row.fav.type === "folder"}<span class="chev tree">{row.expanded ? "▾" : "▸"}</span>{/if}
          <span class="dot small" style={`background: ${dot(row.fav)}`}></span>
          <span class="icon">{iconGlyph(row.fav.icon?.value)}</span>
          <span class="label">
            {#if query.trim()}
              {#if row.fav.path.length > 1}<span class="muted">{row.fav.path.slice(0, -1).join(PATH_SEPARATOR) + PATH_SEPARATOR}</span>{/if}{row.fav.title}
            {:else}
              {row.fav.title}
            {/if}
          </span>
          {#if row.fav.ask_note}<span class="tag" title="Asks for a note">note</span>{/if}
          {#if row.fav.id === running?.id}<span class="tag">running</span>{/if}
          {#if row.fav.kind === "status"}<span class="tag">status</span>{/if}
        </li>
      {:else}
        <li class="empty muted" role="option" aria-selected="false">{query.trim() ? "No matching favorite" : "No favorites defined – add some in the settings"}</li>
      {/each}
    {/if}
  </ul>

  {#if noteOpen}
    <div class="note-row">
      <input
        bind:this={noteEl}
        bind:value={note}
        placeholder={noteFor
          ? `Note for ${noteFor.path.join(PATH_SEPARATOR)} – Enter ${retro ? "logs" : "starts"} it`
          : running && !retro
            ? "New note – Enter applies it to the running activity"
            : "Note for the activity you start"}
      />
    </div>
  {/if}

  <footer>
    {#if error}
      <div class="error">{error}</div>
    {:else if highlightedPath.length > 0}
      <div class="path">{highlightedPath.join(PATH_SEPARATOR)}</div>
    {/if}
    <div class="hints muted">
      {#if mode === "scroll"}
        scroll to select · hotkey/<kbd>Enter</kbd> confirm · type to search · <kbd>Tab</kbd> tree
      {:else}
        <kbd>↑↓</kbd>/scroll select · <kbd>Enter</kbd> {retro ? "log" : "start"} · <kbd>Shift</kbd>+<kbd>Enter</kbd>/right click with note · <kbd>Ctrl</kbd>+<kbd>N</kbd> note · <kbd>Ctrl</kbd>+<kbd>R</kbd> retroactive · <kbd>Ctrl</kbd>+<kbd>S</kbd> stop · <kbd>Esc</kbd> close
      {/if}
    </div>
  </footer>
</div>

<style>
  .pop {
    display: flex;
    flex-direction: column;
    height: 100vh;
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
    background: var(--bg);
  }
  header {
    padding: 10px 12px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-soft);
  }
  .running {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 36px;
  }
  .running.idle .what {
    text-align: center;
  }
  .settings {
    flex: none;
    padding-left: 8px;
    padding-right: 8px;
  }
  .what {
    flex: 1;
    min-width: 0;
  }
  .title {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub {
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .prompt {
    margin-top: 8px;
    font-size: 13px;
  }
  .prompt .q {
    font-weight: 600;
    color: var(--accent);
    margin-bottom: 2px;
  }
  .prompt .buttons {
    display: flex;
    gap: 6px;
    margin-top: 6px;
    flex-wrap: wrap;
  }
  .offline {
    margin-top: 6px;
    font-size: 12px;
    color: var(--warn);
  }
  .dot {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    flex: none;
  }
  .dot.small {
    width: 9px;
    height: 9px;
    border: 1px solid var(--border);
  }
  .search,
  .retro-row,
  .note-row {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 8px 12px 0;
  }
  .search input,
  .retro-row input,
  .note-row input {
    flex: 1;
    min-width: 0;
  }
  .preview {
    font-size: 12px;
    max-width: 45%;
  }
  .list {
    flex: 1;
    list-style: none;
    margin: 8px 0 0;
    padding: 0 6px;
    overflow-y: auto;
    outline: none;
  }
  .list li {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    border-radius: var(--radius);
    cursor: pointer;
    overflow: hidden;
  }
  .list li:hover {
    background: var(--bg-hover);
  }
  .list li.hl {
    background: var(--highlight);
    font-weight: 600;
  }
  .list li.back .label,
  .list li.stop .label,
  .list li.cancel .label {
    color: var(--fg-soft);
  }
  .list li.stop .label {
    color: var(--danger);
  }
  .list li.empty {
    cursor: default;
  }
  .list li.heading {
    cursor: default;
    color: var(--fg-soft);
    font-size: 12px;
    box-sizing: border-box;
    height: 24px;
    padding-top: 0;
    padding-bottom: 0;
    border-radius: 0;
    position: sticky;
    background: var(--bg);
  }
  .list li.group {
    display: block;
    padding: 0;
    overflow: visible;
    cursor: default;
    border-radius: 0;
  }
  .list li.group:hover {
    background: none;
  }
  .list ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .list li.heading:hover {
    background: none;
  }
  .label {
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .icon {
    width: 1.3em;
    text-align: center;
    flex: none;
  }
  .chev {
    color: var(--fg-soft);
  }
  .chev.tree {
    width: 1em;
  }
  .tag {
    font-size: 11px;
    font-weight: 500;
    padding: 1px 6px;
    border-radius: 10px;
    background: var(--bg-soft);
    border: 1px solid var(--border);
    color: var(--fg-soft);
  }
  .tag.retro {
    color: var(--accent);
    border-color: var(--accent);
  }
  .dwell {
    position: absolute;
    left: 0;
    bottom: 0;
    height: 3px;
    width: 0;
    background: var(--accent);
    animation: dwell var(--dwell) linear forwards;
  }
  @keyframes dwell {
    from {
      width: 0;
    }
    to {
      width: 100%;
    }
  }
  footer {
    padding: 8px 12px 10px;
    border-top: 1px solid var(--border);
    font-size: 12px;
  }
  .path {
    font-weight: 600;
    margin-bottom: 4px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .hints {
    line-height: 1.8;
  }
</style>
