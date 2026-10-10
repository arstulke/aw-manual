/**
 * Turns wheel events into navigation steps.
 *
 * - The first event after a pause always produces one step, so one notch of
 *   a regular mouse wheel is one step regardless of the platform's delta size.
 * - Within a burst of events (high-resolution wheels, touchpads), deltas are
 *   accumulated and one step is produced per threshold, so they don't race
 *   through the list. `sensitivity` scales the threshold.
 */
export class WheelSteps {
  private acc = 0;
  private last = -Infinity;
  private lastHorizontal = -Infinity;

  constructor(
    private sensitivity = 1,
    private readonly pauseMs = 160,
    private readonly basePx = 40,
  ) {}

  setSensitivity(s: number): void {
    this.sensitivity = s > 0 ? s : 1;
  }

  private px(delta: number, mode: number): number {
    return mode === 1 ? delta * 40 : mode === 2 ? delta * 800 : delta;
  }

  /** Vertical: returns +1 (next), -1 (previous) or 0. */
  vertical(deltaY: number, deltaMode: number, now: number): number {
    const d = this.px(deltaY, deltaMode);
    if (d === 0) return 0;
    const fresh = now - this.last > this.pauseMs;
    this.last = now;
    if (fresh || Math.sign(d) !== Math.sign(this.acc)) this.acc = 0;
    if (fresh) return Math.sign(d);
    this.acc += d;
    const threshold = this.basePx * this.sensitivity;
    if (Math.abs(this.acc) >= threshold) {
      const step = Math.sign(this.acc);
      this.acc = 0;
      return step;
    }
    return 0;
  }

  /** Horizontal scrolling / wheel tilt: one action per gesture. +1 = right, -1 = left. */
  horizontal(deltaX: number, deltaMode: number, now: number): number {
    const d = this.px(deltaX, deltaMode);
    if (d === 0) return 0;
    const fresh = now - this.lastHorizontal > 350;
    this.lastHorizontal = now;
    return fresh ? Math.sign(d) : 0;
  }
}
