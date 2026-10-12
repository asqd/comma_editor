// Recent timings of drawing and of requests for rows, for the status bar and the bench.

const KEPT = 600;

export class Timings {
  private readonly values: number[] = [];

  add(milliseconds: number): void {
    this.values.push(milliseconds);
    if (this.values.length > KEPT) this.values.shift();
  }

  clear(): void {
    this.values.length = 0;
  }

  get count(): number {
    return this.values.length;
  }

  /** The value below which `share` of the timings fall, e.g. 0.95. */
  percentile(share: number): number {
    return percentile(this.values, share);
  }
}

export function percentile(values: readonly number[], share: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(share * sorted.length))];
}

export class Stats {
  readonly drawing = new Timings();
  readonly requests = new Timings();

  draw(milliseconds: number): void {
    this.drawing.add(milliseconds);
  }

  fetch(milliseconds: number): void {
    this.requests.add(milliseconds);
  }
}
