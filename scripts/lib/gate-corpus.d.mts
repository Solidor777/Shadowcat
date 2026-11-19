/** Options for {@link repoFiles}; see the implementation in `gate-corpus.mjs` for the semantics. */
export interface RepoFilesOptions {
  /** Repository directory; defaults to the process working directory. */
  cwd?: string;
  /** Top-level path prefixes to keep. */
  roots?: string[];
  /** File name suffixes to keep. */
  exts?: string[];
  /** `"all"` adds untracked, non-ignored files; `"tracked"` reads the index only. */
  include?: "all" | "tracked";
  /** Also keep files directly in the repository root. */
  rootFiles?: boolean;
  /** Receives the `corpus <label>: <N> files` line; a library caller passes a no-op. */
  log?: (line: string) => void;
  /** Names the caller in the log line. */
  label?: string;
}

/** Raised when git cannot list a corpus; a gate catches this class only and exits 2. */
export class CorpusListingError extends Error {}

/**
 * The repository's source corpus: sorted, forward-slash paths relative to `cwd`.
 * Throws a `CorpusListingError` when git cannot list the files.
 * @param options - corpus selection.
 * @returns the matching repo-relative paths.
 * @example
 * const specs = repoFiles({ roots: ["e2e"], exts: [".spec.ts"] });
 */
export function repoFiles(options?: RepoFilesOptions): string[];
