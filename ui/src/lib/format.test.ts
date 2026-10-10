import { describe, expect, it } from "vitest";
import { formatClock, formatElapsed, iconGlyph, parseDuration } from "./format";

describe("format", () => {
  it("durations", () => {
    expect(parseDuration("2s")).toBe(2000);
    expect(parseDuration("1h30m")).toBe(5_400_000);
    expect(parseDuration("1500ms")).toBe(1500);
    expect(() => parseDuration("x")).toThrow();
  });
  it("elapsed", () => {
    expect(formatElapsed(83 * 60, "1h 23m")).toBe("1h 23m");
    expect(formatElapsed(5 * 60, "1h 23m")).toBe("5m");
    expect(formatElapsed(83 * 60, "1:23")).toBe("1:23");
    expect(formatElapsed(83 * 60 + 5, "1:23:45")).toBe("1:23:05");
    expect(formatElapsed(45, "1h 23m 45s")).toBe("45s");
  });
  it("clock", () => {
    const d = new Date(2026, 9, 9, 14, 5);
    expect(formatClock(d, "24h")).toBe("14:05");
    expect(formatClock(d, "12h")).toBe("2:05 PM");
    expect(formatClock(new Date(2026, 9, 9, 0, 5), "12h")).toBe("12:05 AM");
  });
  it("icons", () => {
    expect(iconGlyph("coffee")).toBe("☕");
    expect(iconGlyph("🎯")).toBe("🎯");
    expect(iconGlyph("unknown")).toBe("");
    expect(iconGlyph(null)).toBe("");
  });
});
