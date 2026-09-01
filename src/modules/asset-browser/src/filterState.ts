// The browser's listing filter model. Lives in a plain module (not a
// `.svelte` module script) so type-only imports resolve everywhere the
// tooling compiles — a named export from an SFC resolves only through the
// ambient `*.svelte` default-export shim.

/** The browser's listing filter, mapped 1:1 onto `queryAssets` params. */
export interface FilterState {
  /** Name filter text (substring, or a Rust-syntax regex when `nameIsRegex`). */
  name: string;
  /** Whether `name` is sent as `name_regex` instead of a substring. */
  nameIsRegex: boolean;
  /** Tag chips; every listed tag must match (explicit or derived). */
  tags: string[];
  /** Kind filter, or undefined = all kinds. */
  kind?: "image" | "other";
  /** Sort key. */
  sort: "name" | "created" | "size";
}
