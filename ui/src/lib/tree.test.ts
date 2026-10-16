import { describe, expect, it } from "vitest";
import { leaves } from "./fuzzy";
import { backIndex, parentId } from "./tree";
import type { Favorite } from "./types";

const f = (id: string, children?: Favorite[]): Favorite => ({
  id,
  title: id,
  path: id.split("/"),
  type: children ? "folder" : "leaf",
  color: null,
  icon: null,
  file: "",
  line: 0,
  ask_note: false,
  generated: false,
  children,
});

const roots = [f("a", [f("a/b", [f("a/b/c")]), f("a/d")]), f("e")];

describe("tree", () => {
  it("parents", () => {
    expect(parentId("a/b/c")).toBe("a/b");
    expect(parentId("a")).toBeNull();
  });
  it("back jumps to the first leaf of the folder, then of the previous folder", () => {
    const tree = [f("a", [f("a/x"), f("a/y")]), f("b", [f("b/p"), f("b/q"), f("b/r")]), f("e")];
    const ls = leaves(tree);
    expect(ls.map((l) => l.id)).toEqual(["a/x", "a/y", "b/p", "b/q", "b/r", "e"]);
    expect(backIndex(ls, 4)).toBe(2);
    expect(backIndex(ls, 2)).toBe(0);
    expect(backIndex(ls, 0)).toBe(0);
    expect(backIndex(ls, 5)).toBe(2);
    expect(backIndex(leaves(roots), 0)).toBe(0);
  });
});
