import { parentId } from "./tree";
import type { Favorite } from "./types";

/** An item of the scroll-mode list. */
export type NavItem =
  | { kind: "stop" }
  | { kind: "cancel" }
  | { kind: "heading"; fav: Favorite; depth: number }
  | { kind: "leaf"; fav: Favorite; depth: number };

/** What confirming an item asks the popover to do. */
export type NavAction =
  | { kind: "none" }
  | { kind: "start"; fav: Favorite }
  | { kind: "stop" }
  | { kind: "close" };

/**
 * The whole favorites tree, expanded, for scroll mode: folders are shown as
 * headings, scrolling steps through the leaves only (plus "■ Stop" and
 * "✕ Cancel"). "Back" jumps to the first leaf of the current folder, or of
 * the previous folder when already there. Scrolling wraps around at both ends.
 */
export class ExpandedTree {
  index = 0;

  constructor(
    private roots: Favorite[],
    private runningId: string | null,
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
    this.index = sel[(pos + dir + sel.length) % sel.length];
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
