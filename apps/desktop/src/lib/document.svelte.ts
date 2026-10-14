// The document of a window as the interface sees it: its status, kept current after every
// command, and while a large file is read or a job runs in the background.

import { call, CoreFailure, type EditResult, type Status } from "./api";
import { sentence } from "./errors";

/** How often the status is asked for while something runs in the background. */
const POLL_MS = 100;

export class Document {
  /** `null` until a file is open. */
  status = $state.raw<Status | null>(null);
  /** The last problem, for the status bar. */
  problem = $state("");
  private polling = 0;
  /** Those waiting for the running job to end, and how it ended. */
  private waiting: ((result: { ok?: unknown; error?: unknown } | null) => void)[] = [];

  /**
   * @param changed runs after every new status, with the one before: the grid reloads its
   * rows when they may differ.
   */
  constructor(private readonly changed: (status: Status, previous: Status | null) => void) {}

  get open(): boolean {
    return this.status !== null;
  }

  /** Runs a command that returns the new status, and shows it. */
  async run(command: string, args: object = {}): Promise<Status> {
    return this.show(await call<Status>(command, args));
  }

  /** Runs a command whose reply carries the new status beside other things. */
  async runWith<T extends { status: Status }>(command: string, args: object = {}): Promise<T> {
    const reply = await call<T>(command, args);
    this.show(reply.status);
    return reply;
  }

  /** Runs a command, showing a failure in the status bar instead of throwing it. */
  async attempt(work: () => Promise<unknown>): Promise<boolean> {
    try {
      await work();
      this.problem = "";
      return true;
    } catch (error) {
      this.problem = sentence(error);
      return false;
    }
  }

  /**
   * Makes a change to many rows (`EditRequest` of comma-ffi), waiting for the background job a
   * large table needs; `null` when it was cancelled.
   */
  async edit(request: object): Promise<EditResult | null> {
    const reply = await this.runWith<{ status: Status; result: EditResult | null }>("edit", { request });
    if (reply.result) return reply.result;
    const ended = await this.waitForJob();
    if (ended?.error) throw new CoreFailure(ended.error);
    return (ended?.ok as EditResult | null | undefined) ?? null;
  }

  /** How the running job ends; `null` at once when none runs. */
  waitForJob(): Promise<{ ok?: unknown; error?: unknown } | null> {
    if (!this.status?.job) return Promise.resolve(null);
    return new Promise((resolve) => this.waiting.push(resolve));
  }

  async refresh(): Promise<void> {
    if (this.status) this.show(await call<Status>("status"));
  }

  show(status: Status): Status {
    const previous = this.status;
    this.status = status;
    // Reading a sheet or computing formulas failed after the command that started it.
    if (status.book?.problem) this.problem = sentence(status.book.problem);
    this.changed(status, previous);
    if ((status.indexing !== null || status.job !== null || status.book?.loading) && !this.polling) {
      this.polling = window.setTimeout(() => this.poll(), POLL_MS);
    }
    return status;
  }

  private async poll(): Promise<void> {
    this.polling = 0;
    const before = this.status;
    try {
      const status = await call<Status>("status");
      // A job that ended may have failed; its result says how.
      if (before?.job && !status.job) {
        const result = await call<{ ok?: unknown; error?: unknown } | null>("job_result");
        if (result?.error) this.problem = sentence(result.error);
        for (const resolve of this.waiting.splice(0)) resolve(result);
      }
      this.show(status);
    } catch (error) {
      this.problem = sentence(error);
    }
  }
}
