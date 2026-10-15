import { describe, expect, it } from "vitest";
import { fuzzyScore, leaves, normalize, search } from "./fuzzy";
import type { Favorite } from "./types";

function leaf(path: string[], id: string): Favorite {
  return { id, title: path[path.length - 1], path, type: "leaf", kind: "activity", color: null, icon: null, ask_note: false, file: "", line: 0, generated: false };
}
function folder(title: string, id: string, children: Favorite[]): Favorite {
  return { id, title, path: [title], type: "folder", color: null, icon: null, ask_note: false, file: "", line: 0, generated: false, children };
}

const tree: Favorite[] = [
  folder("ACME", "acme", [
    folder("Website", "acme/website", [
      leaf(["ACME", "Website", "Development"], "acme/website/development"),
      leaf(["ACME", "Website", "Code Review"], "acme/website/code-review"),
    ]),
    leaf(["ACME", "Deployment"], "acme/deployment"),
  ]),
  leaf(["Büro"], "buro"),
  leaf(["Break"], "break"),
];

describe("fuzzy search", () => {
  it("matches tokens across the path", () => {
    // Subsequence matching (like fzf): "dev" also matches "Code Review", ranked lower.
    expect(search(tree, "acm web dev").map((f) => f.id)).toEqual(["acme/website/development", "acme/website/code-review"]);
    expect(search(tree, "acm web devel").map((f) => f.id)).toEqual(["acme/website/development"]);
  });
  it("returns leaves only, in configured order for empty queries", () => {
    expect(search(tree, "").map((f) => f.id)).toEqual(leaves(tree).map((f) => f.id));
    expect(search(tree, "acme").every((f) => f.type === "leaf")).toBe(true);
  });
  it("prefers word starts and consecutive characters", () => {
    const r = search(tree, "dep").map((f) => f.id);
    expect(r[0]).toBe("acme/deployment");
    expect(search(tree, "rev")[0].id).toBe("acme/website/code-review");
  });
  it("ignores case and diacritics", () => {
    expect(normalize("Büro")).toBe("buro");
    expect(search(tree, "BURO").map((f) => f.id)).toEqual(["buro"]);
    expect(search(tree, "bür").map((f) => f.id)).toContain("buro");
  });
  it("rejects non-matching queries", () => {
    expect(fuzzyScore("xyz", "ACME › Website")).toBeNull();
    expect(search(tree, "acme xyz")).toEqual([]);
  });
});
