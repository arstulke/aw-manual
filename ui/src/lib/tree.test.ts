import { describe, expect, it } from "vitest";
import { ancestorIds, parentId, visibleRows } from "./tree";
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
  it("shows children of expanded folders only", () => {
    expect(visibleRows(roots, new Set()).map((r) => r.fav.id)).toEqual(["a", "e"]);
    expect(visibleRows(roots, new Set(["a"])).map((r) => [r.fav.id, r.depth])).toEqual([
      ["a", 0],
      ["a/b", 1],
      ["a/d", 1],
      ["e", 0],
    ]);
    expect(visibleRows(roots, new Set(["a", "a/b"])).map((r) => r.fav.id)).toEqual(["a", "a/b", "a/b/c", "a/d", "e"]);
    // An expanded folder below a collapsed one stays hidden.
    expect(visibleRows(roots, new Set(["a/b"])).map((r) => r.fav.id)).toEqual(["a", "e"]);
  });
  it("parents", () => {
    expect(parentId("a/b/c")).toBe("a/b");
    expect(parentId("a")).toBeNull();
    expect(ancestorIds("a/b/c")).toEqual(["a", "a/b"]);
  });
});
