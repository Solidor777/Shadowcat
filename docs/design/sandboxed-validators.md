# Sandboxed third-party validators — threat model

Opt-in, per-world, per-module server-side validators over the `system` band only, running
`wasm32-unknown-unknown` code inside `wasmi` (a pure-Rust interpreter, no JIT). Never the
default path: a GM must install a module declaring validators AND explicitly opt this world
into running them (`WorldModuleEntry.validators_enabled`).

| Threat | Mitigation |
|---|---|
| CPU exhaustion (infinite loop) | fuel cap per call; fault counter auto-disables after 5 |
| Slow-but-under-fuel validator throttling every world (the write pool is a single connection server-wide) | validators run OUTSIDE the write transaction against a read-only pre-image; a 50 ms wall-clock budget per call is a fault; 5 faults auto-disable |
| Many simultaneous intents each running a validator | bounded by the single-writer pool: one intent reaches the write path at a time per server, so at most one validator per in-flight intent; stated here so a future pool change re-examines it |
| Memory exhaustion | `StoreLimits` 16 MiB; module ≤ 4 MiB; input ≤ 1 MiB |
| Escaping the sandbox | `wasmi` is a pure-Rust interpreter with no JIT and no host imports beyond `log`; no WASI |
| Data exfiltration | no network/filesystem/clock imports; the only output channel is the ≤ 512-byte reason. The validator's `prior` (pre-image) band is stored content, so it is WITHHELD unless the writer holds whole-document READ on the document — a write-without-read writer's validator judges the post-image only |
| Denial of writes to the table | GM opt-in per world per module; the GM can turn it off; auto-disable on faults; the GM's own writes are subject to it too (a validator that refuses everything is visible on the first write) |
| Auto-disable as a side effect of non-interactive paths | only `Room::commit_ops_locked`'s error arm (the one funnel every guarded write path shares) may auto-disable; Room-less paths (`import_world`, `create_world`) record fault streaks only — a bulk import must not flip a world's settings by being read in |
| Hostile bundle arriving opted in | an imported world's enablement record is persisted with every `validators_enabled` forced `false` — opting into third-party code is the GM's own act. The import operation itself IS judged by the bundle's declared validators |
| Malicious reason text | carried on `ServerMsg::Reject.detail`, rendered as a text node only; length + control-char stripping |
| Supply chain (a validator swapped on disk) | the compiled-validator cache invalidates on the `.wasm` file's own mtime (an in-place swap takes effect on the next call). A validator swapped to something hostile is otherwise out of scope here — a future signing/SRI mechanism; this doc records the gap rather than papering over it |
