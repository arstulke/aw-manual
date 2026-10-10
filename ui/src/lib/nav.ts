import type { Favorite } from "./types";

/** An item of the scroll-mode list. */
export type NavItem =
  | { kind: "back" }
  | { kind: "stop" }
  | { kind: "cancel" }
  | { kind: "folder"; fav: Favorite }
  | { kind: "leaf"; fav: Favorite };

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

/**
 * Hierarchical list for scroll mode: one level at a time, with "‹ Back",
 * "■ Stop" and "✕ Cancel" as regular items, driven by the abstract commands
 * next / prev / confirm / back / cancel.
 */
export class DrillDown {
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
