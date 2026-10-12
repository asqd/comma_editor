// The app's own commands for `--bench` runs and for the file named on its command line.

import { invoke } from "@tauri-apps/api/core";

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
