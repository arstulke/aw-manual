import { describe, expect, it } from "vitest";
import { ExpandedTree } from "./nav";
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

describe("scroll mode navigation", () => {
  const ids = (d: ExpandedTree) => d.items().map((i) => (i.kind === "leaf" || i.kind === "heading" ? `${i.kind}:${i.fav.id}` : i.kind));

  it("lists all folders expanded, with Stop (when running) and Cancel", () => {
    expect(ids(new ExpandedTree(roots, null))).toEqual([
      "heading:acme",
      "heading:acme/website",
      "leaf:acme/website/development",
      "leaf:acme/website/code-review",
      "leaf:break",
      "cancel",
    ]);
    expect(ids(new ExpandedTree(roots, "break"))[0]).toBe("stop");
  });

  it("scrolls through the leaves only and starts them", () => {
    const d = new ExpandedTree(roots, "break");
    d.startAt("first");
    expect(d.current()).toMatchObject({ kind: "leaf", fav: dev, depth: 2 });
    d.prev();
    expect(d.current().kind).toBe("stop");
    d.prev();
    expect(d.current().kind).toBe("cancel"); // wraps around
    d.next();
    d.next();
    d.next();
    expect(d.confirm()).toEqual({ kind: "start", fav: review });
    expect(d.highlightedPath()).toEqual(["ACME", "Website", "Code Review"]);
    d.next();
    expect(d.confirm()).toEqual({ kind: "close" }); // the running activity
    d.next();
    expect(d.confirm()).toEqual({ kind: "close" }); // Cancel
  });

  it("starts at the running activity and wraps around", () => {
    const d = new ExpandedTree(roots, "acme/website/code-review");
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
    const d = new ExpandedTree(roots, null);
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
    const d = new ExpandedTree(roots, null);
    d.startAt("first");
    d.next();
    d.update(roots, "break");
    expect(d.current()).toMatchObject({ fav: review });
    d.update([brk], "break");
    expect(d.current().kind).not.toBe("heading");
  });
});
