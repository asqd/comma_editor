// Where the table's rows come from: the core through Tauri, or made-up rows when the page runs
// in a plain browser, to work on the grid without the app.

import { Channel, invoke } from "@tauri-apps/api/core";
import type { TableSource } from "./Grid";

export const inTauri = "__TAURI_INTERNALS__" in window;

/** A file opened by the core; `rows` and `columns` grow while a large file is indexed. */
export interface OpenedFile extends TableSource {
  rows: number;
  columns: number;
  readonly name: string;
  readonly encoding: string;
  readonly delimiter: string;
  /** Share of the file indexed; 1 when done. */
  indexed: number;
  /** Seconds the core took to open the file and index its first part. */
  readonly openSeconds: number;
}

interface Opened {
  name: string;
  rows: number;
  columns: number;
  header: string[];
  encoding: string;
  delimiter: string;
  indexing: boolean;
  openSeconds: number;
}

interface Indexing {
  fraction: number;
  done: boolean;
  rows: number;
  columns: number;
}

/** Opens a file in this window; `onIndexing` runs after each step of indexing a large file. */
export async function openFile(path: string, onIndexing: () => void): Promise<OpenedFile> {
  const progress = new Channel<Indexing>();
  const opened = await invoke<Opened>("open_file", { path, progress });
  const file: OpenedFile = {
    ...opened,
    indexed: opened.indexing ? 0 : 1,
    fetch: (start, count) => invoke<string[][]>("visible_rows", { start, count }),
  };
  progress.onmessage = (step) => {
    file.rows = step.rows;
    file.columns = step.columns;
    file.indexed = step.done ? 1 : step.fraction;
    onIndexing();
  };
  return file;
}

/** The file named on the command line, and whether `--bench` asks to time it and quit. */
export function launch(): Promise<{ path: string | null; bench: boolean }> {
  return invoke("launch");
}

/** Prints a step of a `--bench` run. */
export function benchStep(step: string): Promise<void> {
  return invoke("bench_step", { step });
}

/** Prints the times of a `--bench` run and quits the app. */
export function benchDone(ok: boolean, report: string): Promise<void> {
  return invoke("bench_done", { ok, report });
}

/** Rows like those of `comma generate`, made on request; `?rows=` and `?latency=` in the URL. */
export function demoFile(): OpenedFile {
  const parameters = new URLSearchParams(location.search);
  const rows = Number(parameters.get("rows") ?? 60_000_000);
  const latency = Number(parameters.get("latency") ?? 0);
  const header = ["id", "name", "email", "city", "date", "balance", "note", "a", "b", "c"];
  const row = (i: number) => {
    const x = Math.imul(i + 1, 0x9e3779b1) >>> 0;
    const pad = (n: number) => String(n).padStart(2, "0");
    return [
      String(i),
      `Name ${i}`,
      `user${i}@example.com`,
      `City ${x % 500}`,
      `2024-${pad((x % 12) + 1)}-${pad((x % 28) + 1)}`,
      `${x % 100_000}.${pad(Math.floor(x / 100_000) % 100)}`,
      i % 1000 === 7 ? `note, ${x % 1000}\nsecond line` : `note, ${x % 1000}`,
      "x",
      "y",
      "z",
    ];
  };
  return {
    name: "Демо",
    rows,
    columns: header.length,
    header,
    encoding: "UTF-8",
    delimiter: ",",
    indexed: 1,
    openSeconds: 0,
    fetch: (start, count) =>
      new Promise((resolve) =>
        setTimeout(() => {
          const end = Math.min(rows, start + count);
          resolve(Array.from({ length: Math.max(0, end - start) }, (_, k) => row(start + k)));
        }, latency),
      ),
  };
}
