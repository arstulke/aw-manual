import { parentId } from "./tree";
import type { Favorite } from "./types";

/** An item of the scroll-mode list. `depth` is set in the expanded tree layout. */
export type NavItem =
  | { kind: "back" }
  | { kind: "stop" }
  | { kind: "cancel" }
  | { kind: "folder"; fav: Favorite; depth?: number }
  | { kind: "heading"; fav: Favorite; depth: number }
  | { kind: "leaf"; fav: Favorite; depth?: number };

/** What confirming an item asks the popover to do. */
export type NavAction =
  | { kind: "none" }
  | { kind: "start"; fav: Favorite }
  | { kind: "stop" }
  | { kind: "close" };

/** Finds the folders leading to `id` (excluding the node itself). */
export function ancestors(roots: Favorite[], id: string): Favorite[] | null {
  const walk = (nodes: Favorite[], trail: Favorite[]): Favorite[] | null => {
    for (const n of nodes) {
      if (n.id === id) return trail;
      if (n.type === "folder") {
        const found = walk(n.children ?? [], [...trail, n]);
        if (found) return found;
      }
    }
    return null;
  };
  return walk(roots, []);
}

/** Navigation of the scroll-mode list, driven by next / prev / confirm / back / cancel. */
export interface ScrollNav {
  index: number;
  items(): NavItem[];
  current(): NavItem;
  next(): void;
  prev(): void;
  back(): void;
  confirm(): NavAction;
  startAt(mode: "running" | "first"): void;
  highlightedPath(): string[];
  update(roots: Favorite[], runningId: string | null): void;
}

/**
 * Hierarchical list for scroll mode: one level at a time, with "‹ Back",
 * "■ Stop" and "✕ Cancel" as regular items, driven by the abstract commands
 * next / prev / confirm / back / cancel.
 */
export class DrillDown implements ScrollNav {
  /** Opened folders, from the top level down. */
  stack: Favorite[] = [];
  index = 0;

  constructor(
    private roots: Favorite[],
    private runningId: string | null,
    private wrap: boolean,
  ) {}

  items(): NavItem[] {
    const level = this.stack.length > 0 ? this.stack[this.stack.length - 1].children ?? [] : this.roots;
    const items: NavItem[] = [];
    if (this.stack.length > 0) items.push({ kind: "back" });
    else if (this.runningId) items.push({ kind: "stop" });
    for (const f of level) items.push(f.type === "folder" ? { kind: "folder", fav: f } : { kind: "leaf", fav: f });
    items.push({ kind: "cancel" });
    return items;
  }

  current(): NavItem {
    const items = this.items();
    return items[Math.min(this.index, items.length - 1)];
  }

  /** Index of the first favorite on the current level (skipping Back/Stop). */
  private firstFavorite(): number {
    const i = this.items().findIndex((it) => it.kind === "folder" || it.kind === "leaf");
    return i === -1 ? 0 : i;
  }

  next(): void {
    const n = this.items().length;
    if (this.index < n - 1) this.index++;
    else if (this.wrap) this.index = 0;
  }

  prev(): void {
    const n = this.items().length;
    if (this.index > 0) this.index--;
    else if (this.wrap) this.index = n - 1;
  }

  back(): void {
    const left = this.stack.pop();
    if (!left) return;
    const i = this.items().findIndex((it) => it.kind === "folder" && it.fav.id === left.id);
    this.index = i === -1 ? this.firstFavorite() : i;
  }

  confirm(): NavAction {
    const item = this.current();
    switch (item.kind) {
      case "back":
        this.back();
        return { kind: "none" };
      case "stop":
        return { kind: "stop" };
      case "cancel":
        return { kind: "close" };
      case "folder":
        this.stack.push(item.fav);
        this.index = this.firstFavorite();
        return { kind: "none" };
      case "leaf":
        if (item.fav.id === this.runningId) return { kind: "close" };
        return { kind: "start", fav: item.fav };
      case "heading":
        return { kind: "none" };
    }
  }

  /** Initial highlight: the running activity (opening its folders) or the first favorite. */
  startAt(mode: "running" | "first"): void {
    this.stack = [];
    if (mode === "running" && this.runningId) {
      const trail = ancestors(this.roots, this.runningId);
      if (trail) {
        this.stack = trail;
        const i = this.items().findIndex((it) => it.kind === "leaf" && it.fav.id === this.runningId);
        if (i !== -1) {
          this.index = i;
          return;
        }
        this.stack = [];
      }
    }
    this.index = this.firstFavorite();
  }

  /** Path of the highlighted item for display ("ACME › Website › Development"). */
  highlightedPath(): string[] {
    const item = this.current();
    if (item.kind === "folder" || item.kind === "leaf") return item.fav.path;
    return this.stack.length > 0 ? this.stack[this.stack.length - 1].path : [];
  }

  /** Updates favorites and the running activity, keeping the position where possible. */
  update(roots: Favorite[], runningId: string | null): void {
    const current = this.current();
    const currentId = current.kind === "folder" || current.kind === "leaf" ? current.fav.id : null;
    const stackIds = this.stack.map((f) => f.id);
    this.roots = roots;
    this.runningId = runningId;
    this.stack = [];
    for (const id of stackIds) {
      const level = this.stack.length > 0 ? this.stack[this.stack.length - 1].children ?? [] : roots;
      const f = level.find((x) => x.id === id && x.type === "folder");
      if (!f) break;
      this.stack.push(f);
    }
    const items = this.items();
    const i = currentId ? items.findIndex((it) => (it.kind === "folder" || it.kind === "leaf") && it.fav.id === currentId) : -1;
    if (i !== -1) this.index = i;
    else if (current.kind === "cancel") this.index = items.length - 1;
    else this.index = Math.min(this.index, items.length - 1);
  }
}

/**
 * The whole favorites tree, expanded, for scroll mode: folders are shown as
 * headings, scrolling steps through the leaves only (plus "■ Stop" and
 * "✕ Cancel"). "Back" jumps to the first leaf of the current folder, or of
 * the previous folder when already there.
 */
export class ExpandedTree implements ScrollNav {
  index = 0;

  constructor(
    private roots: Favorite[],
    private runningId: string | null,
    private wrap: boolean,
  ) {}

  items(): NavItem[] {
    const items: NavItem[] = [];
    if (this.runningId) items.push({ kind: "stop" });
    const walk = (nodes: Favorite[], depth: number) => {
      for (const f of nodes) {
        if (f.type === "folder") {
          items.push({ kind: "heading", fav: f, depth });
          walk(f.children ?? [], depth + 1);
        } else {
          items.push({ kind: "leaf", fav: f, depth });
        }
      }
    };
    walk(this.roots, 0);
    items.push({ kind: "cancel" });
    return items;
  }

  /** Indices of the items that can be highlighted (everything but headings). */
  private selectable(): number[] {
    const out: number[] = [];
    this.items().forEach((it, i) => {
      if (it.kind !== "heading") out.push(i);
    });
    return out;
  }

  current(): NavItem {
    const items = this.items();
    return items[Math.min(this.index, items.length - 1)];
  }

  private firstLeaf(): number {
    const i = this.items().findIndex((it) => it.kind === "leaf");
    return i === -1 ? this.selectable()[0] : i;
  }

  private step(dir: 1 | -1): void {
    const sel = this.selectable();
    const pos = sel.indexOf(this.index);
    if (pos === -1) {
      // Not on a selectable item (e.g. after an update): the nearest one in that direction.
      const ahead = dir === 1 ? sel.filter((i) => i > this.index) : sel.filter((i) => i < this.index).reverse();
      this.index = ahead[0] ?? sel[dir === 1 ? sel.length - 1 : 0];
      return;
    }
    let next = pos + dir;
    if (next < 0) next = this.wrap ? sel.length - 1 : 0;
    if (next >= sel.length) next = this.wrap ? 0 : sel.length - 1;
    this.index = sel[next];
  }

  next(): void {
    this.step(1);
  }

  prev(): void {
    this.step(-1);
  }

  back(): void {
    const items = this.items();
    const group = (i: number) => {
      const it = items[i];
      return it?.kind === "leaf" ? parentId(it.fav.id) ?? "" : null;
    };
    const g = group(this.index);
    if (g === null) return;
    // Start of the current group: leaves of the same folder directly above.
    let start = this.index;
    while (start > 0 && group(start - 1) === g) start--;
    if (start !== this.index) {
      this.index = start;
      return;
    }
    // Already there: the first leaf of the previous group.
    let i = start - 1;
    while (i >= 0 && items[i].kind === "heading") i--;
    const prev = group(i);
    if (prev === null) return;
    while (i > 0 && group(i - 1) === prev) i--;
    this.index = i;
  }

  confirm(): NavAction {
    const item = this.current();
    switch (item.kind) {
      case "stop":
        return { kind: "stop" };
      case "cancel":
        return { kind: "close" };
      case "leaf":
        if (item.fav.id === this.runningId) return { kind: "close" };
        return { kind: "start", fav: item.fav };
      default:
        return { kind: "none" };
    }
  }

  startAt(mode: "running" | "first"): void {
    if (mode === "running" && this.runningId) {
      const i = this.items().findIndex((it) => it.kind === "leaf" && it.fav.id === this.runningId);
      if (i !== -1) {
        this.index = i;
        return;
      }
    }
    this.index = this.firstLeaf();
  }

  highlightedPath(): string[] {
    const item = this.current();
    return item.kind === "leaf" ? item.fav.path : [];
  }

  update(roots: Favorite[], runningId: string | null): void {
    const current = this.current();
    this.roots = roots;
    this.runningId = runningId;
    const items = this.items();
    const i =
      current.kind === "leaf"
        ? items.findIndex((it) => it.kind === "leaf" && it.fav.id === current.fav.id)
        : items.findIndex((it) => it.kind === current.kind);
    if (i !== -1) this.index = i;
    else if (current.kind === "stop") this.index = this.firstLeaf();
    else {
      this.index = Math.min(this.index, items.length - 1);
      if (items[this.index].kind === "heading") this.step(1);
    }
  }
}
