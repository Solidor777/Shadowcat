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
| Data exfiltration | no network/filesystem/clock imports; the only output channel is the ≤ 512-byte reason, visible to the writer who already holds the data |
| Denial of writes to the table | GM opt-in per world per module; the GM can turn it off; auto-disable on faults; the GM's own writes are subject to it too (a validator that refuses everything is visible on the first write) |
| Malicious reason text | carried on `ServerMsg::Reject.detail`, rendered as a text node only; length + control-char stripping |
| Supply chain (a validator swapped on disk) | out of scope here — a future signing/SRI mechanism; this doc records the gap rather than papering over it |
