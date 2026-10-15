import { describe, expect, it } from "vitest";
import { WheelSteps } from "./wheel";

describe("wheel steps", () => {
  it("one notch is one step, whatever the delta", () => {
    const w = new WheelSteps();
    expect(w.vertical(4, 0, 0)).toBe(1);
    expect(w.vertical(120, 0, 1000)).toBe(1);
    expect(w.vertical(-53, 0, 2000)).toBe(-1);
    expect(w.vertical(3, 1, 3000)).toBe(1);
  });

  it("accumulates high-resolution bursts", () => {
    const w = new WheelSteps();
    const steps: number[] = [];
    let t = 0;
    for (let i = 0; i < 40; i++) steps.push(w.vertical(5, 0, (t += 16)));
    // First event steps immediately, then one step per 40px.
    expect(steps.filter((s) => s !== 0).length).toBe(1 + Math.floor((39 * 5) / 40));
  });

  it("sensitivity scales the threshold", () => {
    const w = new WheelSteps(2);
    let t = 0;
    let n = 0;
    for (let i = 0; i < 40; i++) n += Math.abs(w.vertical(5, 0, (t += 16)));
    expect(n).toBe(1 + Math.floor((39 * 5) / 80));
  });

  it("direction changes reset the accumulator", () => {
    const w = new WheelSteps();
    expect(w.vertical(5, 0, 0)).toBe(1);
    expect(w.vertical(30, 0, 10)).toBe(0);
    expect(w.vertical(-30, 0, 20)).toBe(0);
    expect(w.vertical(-15, 0, 30)).toBe(-1);
  });

  it("horizontal gestures trigger once", () => {
    const w = new WheelSteps();
    expect(w.horizontal(-10, 0, 0)).toBe(-1);
    expect(w.horizontal(-10, 0, 50)).toBe(0);
    expect(w.horizontal(10, 0, 1000)).toBe(1);
  });
});
