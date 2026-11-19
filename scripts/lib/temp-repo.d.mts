/**
 * Creates a scratch repository holding `files`, indexing only the paths in `tracked` (every file
 * when omitted). A path written but not indexed is an untracked file.
 * @param files - repository-relative path to file text.
 * @param tracked - paths to `git add`; defaults to every key of `files`.
 * @returns the repository directory.
 * @example
 * const dir = makeRepo({ "src/a.rs": "fn a() {}\n" }, []);
 */
export function makeRepo(files: Record<string, string>, tracked?: string[]): string;
