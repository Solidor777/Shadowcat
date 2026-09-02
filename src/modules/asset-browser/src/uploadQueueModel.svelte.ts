// Headless, sequential upload queue over `startChunkedUpload`. Stable-ref
// (constructed once per browser instance, mutated in place); the rendering
// half is `UploadQueue.svelte`.

import { startChunkedUpload, ChunkedUploadError } from "@shadowcat/core";
import type { Asset } from "@shadowcat/types";

/** One queued file's lifecycle state. */
export type UploadStatus = "queued" | "uploading" | "done" | "error";

/** One file in the queue. */
export interface UploadEntry {
  /** The file being uploaded. */
  file: File;
  /** Destination folder (`null` = world root). */
  folderId: string | null;
  /** Bytes the server has accepted so far. */
  sent: number;
  /** The file's size. */
  total: number;
  /** Lifecycle state. */
  status: UploadStatus;
  /** Failure text (an `error` entry) or the done-with-warning note. */
  error?: string;
  /** The asset created when only the placement step failed — it exists
   * server-side; repair or delete it, never re-upload. */
  partial?: Asset;
  /** Aborts the in-flight upload (an `uploading` entry only). */
  controller?: AbortController;
}

/** Sequential upload queue: one in-flight upload, per-entry progress/retry/
 * abort, listing refresh after every server-side creation. */
export class UploadQueue {
  /** The queue, oldest first; entries persist (as done/error) for the UI. */
  entries = $state<UploadEntry[]>([]);

  /** The owning world id. */
  readonly world: string;
  /** Called whenever an asset was created server-side (success OR partial). */
  readonly onCreated: () => void;
  /** Whether the sequential runner loop is active. */
  #running = false;

  /** Builds the queue for one world.
   * @param world - The owning world id.
   * @param onCreated - Listing-refresh callback.
   * @example
   * ```ts
   * import { UploadQueue } from "@shadowcat/module-asset-browser";
   *
   * const q = new UploadQueue("w1", () => {});
   * q.entries.length; // 0
   * ```
   */
  constructor(world: string, onCreated: () => void) {
    this.world = world;
    this.onCreated = onCreated;
  }

  /** Appends files and starts the runner if idle.
   * @param files - The files to upload, in order.
   * @param folderId - Their destination folder (`null` = root).
   * @example
   * ```ts
   * import { UploadQueue } from "@shadowcat/module-asset-browser";
   *
   * const q = new UploadQueue("w1", () => {});
   * q.enqueue([new File([new Uint8Array([1])], "x.png")], null);
   * ```
   */
  enqueue(files: File[], folderId: string | null): void {
    for (const file of files) {
      this.entries.push({
        file,
        folderId,
        sent: 0,
        total: file.size,
        status: "queued",
      });
    }
    void this.#run();
  }

  /** Re-queues a failed entry (no-op for any other state, and for a partial —
   * that asset exists server-side and must not be uploaded again).
   * @param index - The entry's position in `entries`.
   * @example
   * ```ts
   * import { UploadQueue } from "@shadowcat/module-asset-browser";
   *
   * const q = new UploadQueue("w1", () => {});
   * q.retry(0); // no-op on an empty queue
   * ```
   */
  retry(index: number): void {
    const e = this.entries[index];
    if (!e || e.status !== "error" || e.partial) return;
    e.status = "queued";
    e.error = undefined;
    e.sent = 0;
    void this.#run();
  }

  /** Removes a queued entry, or aborts an uploading one (which then settles
   * as an `error` entry).
   * @param index - The entry's position in `entries`.
   * @example
   * ```ts
   * import { UploadQueue } from "@shadowcat/module-asset-browser";
   *
   * const q = new UploadQueue("w1", () => {});
   * q.cancel(0); // no-op on an empty queue
   * ```
   */
  cancel(index: number): void {
    const e = this.entries[index];
    if (!e) return;
    if (e.status === "queued") {
      this.entries.splice(index, 1);
      return;
    }
    if (e.status === "uploading") e.controller?.abort();
  }

  /** The sequential runner: one queued entry at a time, in order.
   * @example
   * ```
   * // private method; kicked by enqueue/retry above
   * declare const q: UploadQueue;
   * void q.enqueue([], null);
   * ```
   */
  async #run(): Promise<void> {
    if (this.#running) return;
    this.#running = true;
    try {
      for (;;) {
        const entry = this.entries.find((e) => e.status === "queued");
        if (!entry) break;
        entry.status = "uploading";
        const controller = new AbortController();
        entry.controller = controller;
        try {
          await startChunkedUpload(this.world, entry.file, {
            folderId: entry.folderId,
            onProgress: (sent, total) => {
              entry.sent = sent;
              entry.total = total;
            },
            signal: controller.signal,
          });
          entry.status = "done";
          entry.sent = entry.total;
          this.onCreated();
        } catch (err) {
          if (err instanceof ChunkedUploadError && err.partial) {
            // The asset WAS created; only placement failed. Surface it as
            // done-with-warning — re-uploading would duplicate it.
            entry.status = "done";
            entry.partial = err.partial;
            entry.error = err.message;
            this.onCreated();
          } else {
            entry.status = "error";
            entry.error = String(err instanceof Error ? err.message : err);
          }
        } finally {
          entry.controller = undefined;
        }
      }
    } finally {
      this.#running = false;
    }
  }
}
