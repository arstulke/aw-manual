import type { Favorite } from "./types";

/** A visible row of the expandable favorites tree (keyboard mode). */
export interface Row {
  fav: Favorite;
  depth: number;
  expanded: boolean;
}

/** Rows in configured order; children only for expanded folders. */
export function visibleRows(roots: Favorite[], expanded: ReadonlySet<string>): Row[] {
  const out: Row[] = [];
  const walk = (nodes: Favorite[], depth: number) => {
    for (const fav of nodes) {
      const isOpen = fav.type === "folder" && expanded.has(fav.id);
      out.push({ fav, depth, expanded: isOpen });
      if (isOpen) walk(fav.children ?? [], depth + 1);
    }
  };
  walk(roots, 0);
  return out;
}

/** ID of the parent folder (IDs are hierarchical: "acme/website/dev" → "acme/website"). */
export function parentId(id: string): string | null {
  const i = id.lastIndexOf("/");
  return i === -1 ? null : id.slice(0, i);
}

/** All ancestor IDs of an ID, from the top level down. */
export function ancestorIds(id: string): string[] {
  const out: string[] = [];
  for (let p = parentId(id); p !== null; p = parentId(p)) out.unshift(p);
  return out;
}
