// The app's own commands for `--bench` runs and for the file named on its command line.

import { invoke } from "@tauri-apps/api/core";

/**
 * The files named on the command line or handed over before the first window was ready, and
 * whether `--bench` asks to time the first and quit.
 */
export function launch(): Promise<{ paths: string[]; bench: boolean }> {
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
