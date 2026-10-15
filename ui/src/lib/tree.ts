import type { Favorite } from "./types";

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

/**
 * "Back" over leaves in tree order (keyboard mode): the first leaf of the
 * current folder, or of the previous folder when already there.
 */
export function backIndex(leaves: Favorite[], index: number): number {
  if (!leaves[index]) return index;
  const group = (i: number) => parentId(leaves[i].id) ?? "";
  const g = group(index);
  let start = index;
  while (start > 0 && group(start - 1) === g) start--;
  if (start !== index || start === 0) return start;
  let i = start - 1;
  const prev = group(i);
  while (i > 0 && group(i - 1) === prev) i--;
  return i;
}
