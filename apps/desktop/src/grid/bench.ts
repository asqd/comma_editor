// Times scrolling in the app itself: frames while scrolling slowly and fast, and how long the
// rows take to appear after jumping far. The numbers decide whether the grid is fast enough on
// a system (stage 1 of docs/PORTING.md).

import type { Grid } from "./Grid";
import { percentile } from "./stats";

export interface ScrollRun {
  name: string;
  frames: number;
  /** Frame intervals, ms. */
  p50: number;
  p95: number;
  /** Frames over 1.5 times the usual frame time. */
  dropped: number;
  /** Frames drawn before the rows in view arrived. */
  empty: number;
}

export interface JumpRun {
  name: string;
  jumps: number;
  /** Time until every row in view is drawn, ms. */
  p50: number;
  p95: number;
  max: number;
}

export interface BenchResult {
  scrolls: ScrollRun[];
  jumps: JumpRun[];
  drawP95: number;
  requestP95: number;
}

const nextFrame = () => new Promise<number>((resolve) => requestAnimationFrame(resolve));

/** The usual frame time, measured on frames that do nothing. */
async function idleFrame(): Promise<number> {
  const intervals: number[] = [];
  let last = await nextFrame();
  for (let i = 0; i < 30; i += 1) {
    const now = await nextFrame();
    intervals.push(now - last);
    last = now;
  }
  return percentile(intervals, 0.5);
}

async function scroll(grid: Grid, name: string, rowsPerFrame: number, frames: number, usual: number): Promise<ScrollRun> {
  grid.scrollTo(grid.maxTop / 3);
  grid.flush();
  await nextFrame();
  const intervals: number[] = [];
  let empty = 0;
  let last = await nextFrame();
  for (let i = 0; i < frames; i += 1) {
    grid.scrollTo(grid.scrollTop + rowsPerFrame * grid.rowHeight);
    if (!grid.flush()) empty += 1;
    const now = await nextFrame();
    intervals.push(now - last);
    last = now;
  }
  return {
    name,
    frames,
    p50: percentile(intervals, 0.5),
    p95: percentile(intervals, 0.95),
    dropped: intervals.filter((interval) => interval > usual * 1.5).length,
    empty,
  };
}

/** Jumps to each position and waits until the rows in view are drawn. */
async function jump(grid: Grid, name: string, positions: number[]): Promise<JumpRun> {
  const times: number[] = [];
  for (const position of positions) {
    const started = performance.now();
    grid.scrollTo(position);
    while (!grid.flush() && performance.now() - started < 5000) await nextFrame();
    times.push(performance.now() - started);
    await nextFrame();
  }
  return { name, jumps: positions.length, p50: percentile(times, 0.5), p95: percentile(times, 0.95), max: Math.max(...times) };
}

export async function runBench(grid: Grid): Promise<BenchResult> {
  grid.stats.drawing.clear();
  grid.stats.requests.clear();
  const usual = await idleFrame();
  const scrolls = [
    await scroll(grid, "медленная прокрутка, 1 строка за кадр", 1, 240, usual),
    await scroll(grid, "колесо, 5 строк за кадр", 5, 240, usual),
    await scroll(grid, "быстрая, 60 строк за кадр", 60, 240, usual),
  ];
  // The same positions on every run, spread over the whole table.
  let seed = 12345;
  const random = () => ((seed = (Math.imul(seed, 1103515245) + 12345) >>> 0) / 2 ** 32);
  const jumps = [
    await jump(grid, "переход в случайное место", Array.from({ length: 40 }, () => random() * grid.maxTop)),
    await jump(grid, "в начало и в конец", [0, grid.maxTop, 0, grid.maxTop, 0, grid.maxTop]),
  ];
  return {
    scrolls,
    jumps,
    drawP95: grid.stats.drawing.percentile(0.95),
    requestP95: grid.stats.requests.percentile(0.95),
  };
}

export function formatBench(result: BenchResult, context: string): string {
  const ms = (value: number) => value.toFixed(1);
  const lines = [context, ""];
  for (const run of result.scrolls) {
    lines.push(`${run.name}: кадр p50 ${ms(run.p50)} мс, p95 ${ms(run.p95)} мс; пропущено ${run.dropped} из ${run.frames}; без строк ${run.empty}`);
  }
  for (const run of result.jumps) {
    lines.push(`${run.name} (${run.jumps}): до строк p50 ${ms(run.p50)} мс, p95 ${ms(run.p95)} мс, худший ${ms(run.max)} мс`);
  }
  lines.push(`рисование кадра p95 ${ms(result.drawP95)} мс; запрос строк p95 ${ms(result.requestP95)} мс`);
  return lines.join("\n");
}
