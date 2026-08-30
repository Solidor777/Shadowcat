import type { Asset, CreateUploadResponse } from "@shadowcat/types";
import { patchAsset, restErrorText, uploadAsset } from "./asset-rest";

// The chunked-upload client: opens a session, PUTs fixed-size chunks at
// explicit offsets with per-chunk retry, and completes it. Resumable across
// a dropped connection (the session lives on the server until completed,
// aborted, or idle-swept), not across a page reload. Framework-neutral —
// plain fetch, injectable for tests.

/** Files at or under this many bytes go through the single-shot
 * `uploadAsset` route; larger ones open a chunked session. Mirrors the
 * server's fixed chunk size — anything that fits one chunk gains nothing
 * from a session. */
export const CHUNK_THRESHOLD_BYTES = 8 * 1024 * 1024;

/** Options for `startChunkedUpload`. */
export interface ChunkedUploadOptions {
  /** Destination folder (`null`/absent = world root). */
  folderId?: string | null;
  /** Explicit tags to record on the asset. */
  tags?: string[];
  /** Progress callback, called after each accepted chunk (and once for a single-shot upload).
   * @param sent Bytes the server has accepted so far.
   * @param total The file's size.
   */
  onProgress?(sent: number, total: number): void;
  /** Abort: an in-flight session is aborted on the server (`DELETE`) and the
   * returned promise rejects with the signal's reason. */
  signal?: AbortSignal;
  /** `fetch` replacement (tests). Defaults to the global `fetch`. */
  fetchImpl?: typeof fetch;
  /** Attempts per chunk before giving up (a network error or 5xx retries at
   * the SAME offset; a 4xx never retries). Default 3. */
  retries?: number;
}

/** Thrown when an upload cannot complete. */
export class ChunkedUploadError extends Error {
  /** HTTP status of the refusing response, if there was one. */
  readonly status: number | undefined;
  /** The asset that WAS created when only the follow-up placement step failed
   * (single-shot path): it exists server-side, unfiled/untagged. A caller
   * repairs it with `patchAsset` or deletes it — retrying the whole upload
   * would create a duplicate. `undefined` when nothing was created. */
  readonly partial: Asset | undefined;
  /** Build the error.
   * @param message What failed.
   * @param status The refusing response's status, when there was a response.
   * @param partial The asset created before the failure, if any.
   * @example
   * ```ts
   * import { ChunkedUploadError } from "@shadowcat/core";
   *
   * throw new ChunkedUploadError("upload session refused: HTTP 413", 413);
   * ```
   */
  constructor(message: string, status?: number, partial?: Asset) {
    super(message);
    this.name = "ChunkedUploadError";
    this.status = status;
    this.partial = partial;
  }
}

/** Whether a chunk `PUT` may be retried at the same offset: a transport failure (no
 * response) or a server-side 5xx; any 4xx is final.
 * @param status The response status, or `undefined` when no response arrived.
 * @returns `true` to retry the same offset.
 * @example
 * ```ts
 * retryable(undefined); // true
 * retryable(503); // true
 * retryable(409); // false
 * ```
 */
function retryable(status: number | undefined): boolean {
  return status === undefined || status >= 500;
}

/**
 * Upload `file` into `world`: single-shot (`uploadAsset`) when it fits one
 * chunk, otherwise through a resumable chunked session. GM-only either way
 * (`require_gm`); a non-GM gets a 403 at session create. Folder and tags
 * are applied at completion for the chunked path, and via a follow-up
 * `patchAsset` for the single-shot path.
 *
 * Retry rule: a chunk that fails with a network error or a 5xx is re-sent
 * at the same offset (the server accepts exactly the next byte, so a chunk
 * it never received is retried cleanly, and one it DID receive answers 409
 * — which is treated as an unrecoverable desync: the session is aborted and
 * the promise rejects rather than guessing an offset).
 * @param world The world id to upload into.
 * @param file The file to upload.
 * @param opts Placement, progress, abort and transport options.
 * @returns The created asset record.
 * @example
 * ```ts
 * import { startChunkedUpload } from "@shadowcat/core";
 *
 * declare const file: File;
 * const asset = await startChunkedUpload("00000000-0000-0000-0000-000000000001", file, {
 *   folderId: null,
 *   tags: ["map"],
 *   onProgress: (sent, total) => console.info(`${sent}/${total}`),
 * });
 * ```
 */
export async function startChunkedUpload(
  world: string,
  file: File,
  opts: ChunkedUploadOptions = {},
): Promise<Asset> {
  const fetchImpl = opts.fetchImpl ?? fetch;
  const retries = Math.max(1, opts.retries ?? 3);
  const total = file.size;
  const hasPlacement = opts.folderId != null || (opts.tags?.length ?? 0) > 0;

  if (total <= CHUNK_THRESHOLD_BYTES) {
    const created = await uploadAsset(world, file);
    opts.onProgress?.(total, total);
    if (!hasPlacement) return created;
    try {
      return await patchAsset(created.id, {
        folder_id: opts.folderId ?? null,
        tags: opts.tags ?? [],
      });
    } catch (e) {
      // The upload itself succeeded; hand the caller the asset so it can be
      // repaired or removed rather than re-uploaded.
      throw new ChunkedUploadError(
        `placement failed after upload: ${e instanceof Error ? e.message : String(e)}`,
        undefined,
        created,
      );
    }
  }

  opts.signal?.throwIfAborted();
  const createRes = await fetchImpl(`/api/worlds/${world}/assets/uploads`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      name: file.name,
      content_type: file.type || "application/octet-stream",
      byte_size: total,
      folder_id: opts.folderId ?? null,
      tags: opts.tags ?? [],
    }),
    signal: opts.signal,
  });
  if (!createRes.ok) {
    throw new ChunkedUploadError(
      `upload session refused: ${await restErrorText(createRes)}`,
      createRes.status,
    );
  }
  const session = (await createRes.json()) as CreateUploadResponse;
  const id = String(session.upload_id);
  const chunkSize = Number(session.chunk_size);
  // Abort the server session; best effort — the server sweeps an orphaned session itself.
  const abort = async (): Promise<void> => {
    try {
      await fetchImpl(`/api/assets/uploads/${id}`, { method: "DELETE" });
    } catch {
      // Best effort: the server sweeps an orphaned session on its own.
    }
  };

  let sent = 0;
  try {
    while (sent < total) {
      opts.signal?.throwIfAborted();
      const chunk = file.slice(sent, Math.min(sent + chunkSize, total));
      let lastFailure: ChunkedUploadError | undefined;
      let accepted = false;
      for (let attempt = 0; attempt < retries && !accepted; attempt++) {
        let status: number | undefined;
        try {
          const res = await fetchImpl(`/api/assets/uploads/${id}/${sent}`, {
            method: "PUT",
            body: chunk,
            signal: opts.signal,
          });
          status = res.status;
          if (res.ok) {
            accepted = true;
            break;
          }
          lastFailure = new ChunkedUploadError(
            `chunk at ${sent} refused: ${await restErrorText(res)}`,
            res.status,
          );
        } catch (e) {
          if (opts.signal?.aborted) throw e;
          lastFailure = new ChunkedUploadError(
            `chunk at ${sent} failed: ${e instanceof Error ? e.message : String(e)}`,
          );
        }
        if (!retryable(status)) break;
      }
      if (!accepted) {
        throw lastFailure ?? new ChunkedUploadError(`chunk at ${sent} failed`);
      }
      sent += chunk.size;
      opts.onProgress?.(sent, total);
    }
    const done = await fetchImpl(`/api/assets/uploads/${id}/complete`, {
      method: "POST",
      signal: opts.signal,
    });
    if (!done.ok) {
      throw new ChunkedUploadError(`upload complete refused: ${await restErrorText(done)}`, done.status);
    }
    return (await done.json()) as Asset;
  } catch (e) {
    await abort();
    throw e;
  }
}
