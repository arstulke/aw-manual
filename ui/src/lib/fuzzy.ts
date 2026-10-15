import type { Favorite } from "./types";
import { PATH_SEPARATOR } from "./format";

/** Lowercase and strip diacritics ("Büro" → "buro"). */
export function normalize(s: string): string {
  return s.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();
}

const BOUNDARY = /[\s›\/\-_.:()[\]]/;

/**
 * Scores one token against a text: all token characters must appear in order.
 * Consecutive characters and matches at word starts score higher.
 * Returns null if the token doesn't match.
 */
export function tokenScore(token: string, text: string): number | null {
  if (token.length === 0) return 0;
  let best: number | null = null;
  for (let start = text.indexOf(token[0]); start !== -1; start = text.indexOf(token[0], start + 1)) {
    let score = 0;
    let ti = 0;
    let prev = -2;
    for (let i = start; i < text.length && ti < token.length; i++) {
      if (text[i] !== token[ti]) continue;
      score += 1;
      if (prev === i - 1) score += 3;
      if (i === 0 || BOUNDARY.test(text[i - 1])) score += 4;
      if (prev >= 0) score -= Math.min(3, (i - prev - 1) * 0.2);
      prev = i;
      ti++;
    }
    if (ti === token.length) {
      score -= start * 0.01;
      if (best === null || score > best) best = score;
    }
  }
  return best;
}

/** Scores a query ("acm web dev") against a text; every token must match. */
export function fuzzyScore(query: string, text: string): number | null {
  const tokens = normalize(query).split(/\s+/).filter(Boolean);
  if (tokens.length === 0) return 0;
  const t = normalize(text);
  let total = 0;
  for (const token of tokens) {
    const s = tokenScore(token, t);
    if (s === null) return null;
    total += s;
  }
  return total;
}

/** All leaves in configured order. */
export function leaves(nodes: Favorite[]): Favorite[] {
  const out: Favorite[] = [];
  const walk = (ns: Favorite[]) => {
    for (const n of ns) {
      if (n.type === "leaf") out.push(n);
      else walk(n.children ?? []);
    }
  };
  walk(nodes);
  return out;
}

/** Searches leaves by path; results are sorted by score, ties keep the configured order. */
export function search(nodes: Favorite[], query: string): Favorite[] {
  const scored: { fav: Favorite; score: number; order: number }[] = [];
  leaves(nodes).forEach((fav, order) => {
    const score = fuzzyScore(query, fav.path.join(PATH_SEPARATOR));
    if (score !== null) scored.push({ fav, score, order });
  });
  scored.sort((a, b) => b.score - a.score || a.order - b.order);
  return scored.map((s) => s.fav);
}
