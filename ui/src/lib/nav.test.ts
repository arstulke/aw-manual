import { describe, expect, it } from "vitest";
import { DrillDown, ExpandedTree } from "./nav";
import type { Favorite } from "./types";

function leaf(path: string[], id: string): Favorite {
  return { id, title: path[path.length - 1], path, type: "leaf", kind: "activity", color: null, icon: null, ask_note: false, file: "", line: 0, generated: false };
}
function folder(path: string[], id: string, children: Favorite[]): Favorite {
  return { id, title: path[path.length - 1], path, type: "folder", color: null, icon: null, ask_note: false, file: "", line: 0, generated: false, children };
}

const dev = leaf(["ACME", "Website", "Development"], "acme/website/development");
const review = leaf(["ACME", "Website", "Code Review"], "acme/website/code-review");
const website = folder(["ACME", "Website"], "acme/website", [dev, review]);
const acme = folder(["ACME"], "acme", [website]);
const brk = leaf(["Break"], "break");
const roots = [acme, brk];

const kinds = (d: DrillDown) => d.items().map((i) => (i.kind === "folder" || i.kind === "leaf" ? i.fav.id : i.kind));

describe("scroll mode navigation", () => {
  it("lists the top level with Stop (when running) and Cancel", () => {
    expect(kinds(new DrillDown(roots, null, false))).toEqual(["acme", "break", "cancel"]);
    expect(kinds(new DrillDown(roots, "break", false))).toEqual(["stop", "acme", "break", "cancel"]);
  });

  it("opens folders, starts leaves and goes back", () => {
    const d = new DrillDown(roots, null, false);
    d.startAt("first");
    expect(d.current()).toEqual({ kind: "folder", fav: acme });
    expect(d.confirm()).toEqual({ kind: "none" });
    expect(kinds(d)).toEqual(["back", "acme/website", "cancel"]);
    expect(d.index).toBe(1);
    d.confirm();
    expect(kinds(d)).toEqual(["back", "acme/website/development", "acme/website/code-review", "cancel"]);
    d.next();
    expect(d.confirm()).toEqual({ kind: "start", fav: review });
    expect(d.highlightedPath()).toEqual(["ACME", "Website", "Code Review"]);
    d.back();
    expect(d.current()).toEqual({ kind: "folder", fav: website });
    // "‹ Back" as an item.
    d.prev();
    expect(d.current().kind).toBe("back");
    d.confirm();
    expect(d.current()).toEqual({ kind: "folder", fav: acme });
    // Back on the top level does nothing.
    d.back();
    expect(d.stack.length).toBe(0);
  });

  it("stops at the ends or wraps around", () => {
    const d = new DrillDown(roots, null, false);
    d.startAt("first");
    d.prev();
    expect(d.index).toBe(0);
    d.next();
    d.next();
    d.next();
    expect(d.current().kind).toBe("cancel");
    const w = new DrillDown(roots, null, true);
    w.startAt("first");
    w.prev();
    expect(w.current().kind).toBe("cancel");
    w.next();
    expect(w.current()).toEqual({ kind: "folder", fav: acme });
  });

  it("starts at the running activity; confirming it only closes", () => {
    const d = new DrillDown(roots, dev.id, false);
    d.startAt("running");
    expect(d.stack.map((f) => f.id)).toEqual(["acme", "acme/website"]);
    expect(d.current()).toEqual({ kind: "leaf", fav: dev });
    expect(d.confirm()).toEqual({ kind: "close" });
    const f = new DrillDown(roots, dev.id, false);
    f.startAt("first");
    expect(f.current()).toEqual({ kind: "folder", fav: acme });
  });

  it("offers Stop and Cancel", () => {
    const d = new DrillDown(roots, "break", false);
    d.startAt("first");
    d.prev();
    expect(d.confirm()).toEqual({ kind: "stop" });
    d.index = d.items().length - 1;
    expect(d.confirm()).toEqual({ kind: "close" });
  });

  it("keeps the position when favorites change", () => {
    const d = new DrillDown(roots, null, false);
    d.startAt("first");
    d.confirm();
    d.confirm();
    d.next();
    const extra = leaf(["ACME", "Website", "New"], "acme/website/new");
    const website2 = folder(["ACME", "Website"], "acme/website", [extra, dev, review]);
    expect(d.current()).toEqual({ kind: "leaf", fav: review });
    d.update([folder(["ACME"], "acme", [website2]), brk], "break");
    expect(d.current()).toEqual({ kind: "leaf", fav: review });
    expect(d.index).toBe(3);
    d.update([brk], null);
    expect(d.stack.length).toBe(0);
  });
});

describe("scroll mode navigation, expanded tree", () => {
  const ids = (d: ExpandedTree) => d.items().map((i) => (i.kind === "folder" || i.kind === "leaf" || i.kind === "heading" ? `${i.kind}:${i.fav.id}` : i.kind));

  it("lists all folders expanded, with Stop (when running) and Cancel", () => {
    expect(ids(new ExpandedTree(roots, null, false))).toEqual([
      "heading:acme",
      "heading:acme/website",
      "leaf:acme/website/development",
      "leaf:acme/website/code-review",
      "leaf:break",
      "cancel",
    ]);
    expect(ids(new ExpandedTree(roots, "break", false))[0]).toBe("stop");
  });

  it("scrolls through the leaves only and starts them", () => {
    const d = new ExpandedTree(roots, "break", false);
    d.startAt("first");
    expect(d.current()).toMatchObject({ kind: "leaf", fav: dev, depth: 2 });
    d.prev();
    expect(d.current().kind).toBe("stop");
    d.prev();
    expect(d.current().kind).toBe("stop");
    d.next();
    d.next();
    expect(d.confirm()).toEqual({ kind: "start", fav: review });
    expect(d.highlightedPath()).toEqual(["ACME", "Website", "Code Review"]);
    d.next();
    expect(d.confirm()).toEqual({ kind: "close" }); // the running activity
    d.next();
    expect(d.confirm()).toEqual({ kind: "close" }); // Cancel
    d.next();
    expect(d.current().kind).toBe("cancel");
  });

  it("starts at the running activity and wraps around", () => {
    const d = new ExpandedTree(roots, "acme/website/code-review", true);
    d.startAt("running");
    expect(d.current()).toMatchObject({ kind: "leaf", fav: review });
    d.next();
    d.next();
    expect(d.current().kind).toBe("cancel");
    d.next();
    expect(d.current().kind).toBe("stop");
    d.prev();
    expect(d.current().kind).toBe("cancel");
  });

  it("back jumps to the start of the current, then the previous group", () => {
    const d = new ExpandedTree(roots, null, false);
    d.startAt("first");
    d.next();
    d.next();
    expect(d.current()).toMatchObject({ fav: brk });
    d.back();
    expect(d.current()).toMatchObject({ fav: dev }); // previous group
    d.next();
    d.back();
    expect(d.current()).toMatchObject({ fav: dev });
    d.back();
    expect(d.current()).toMatchObject({ fav: dev });
  });

  it("keeps the highlighted leaf on updates", () => {
    const d = new ExpandedTree(roots, null, false);
    d.startAt("first");
    d.next();
    d.update(roots, "break");
    expect(d.current()).toMatchObject({ fav: review });
    d.update([brk], "break");
    expect(d.current().kind).not.toBe("heading");
  });
});
