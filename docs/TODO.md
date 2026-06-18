# TODO — Deferred Work

Actionable, externally-logged deferrals. Bugs go in `OPEN_BUGS.md`, not here.

## Tooling
- TODO: Extend the ESLint gate to cover TypeScript. The M1 flat config (`eslint.config.js`) registers only `@eslint/js`, with no `typescript-eslint` parser and no `files` glob, so every `.ts` source is skipped and `pnpm lint` can pass green with lint errors present in TypeScript. Add `typescript-eslint` and a `files: ["**/*.ts"]` block once real client logic lands (post-M1).

## Data layer
- TODO: Enforce `validation::validate_system_size` (256 KiB opaque-body cap) and `validate_field_path` on the write path. The pure helpers exist and are unit-tested but are not called from `apply_command`; the cap is unenforced until the write path gains an input guard. Wire them in when commands first carry untrusted input (HTTP/permission layer, M3/M5).
- TODO: `command::set_pointer` is set-only — an Update that conceptually removes a key writes `null` (key stays present as null) rather than removing it. `null` ≠ absent. Resolve removal semantics when the merge engine lands.
