# Creating a validator

A validator is a `wasm32-unknown-unknown` module a GM opts a world into running, server-side,
over ONE document type's `system` band. Unlike a module's own client code (untrusted-but-
admin-installed, see [Creating a module](/guides/creating-a-module)), a validator runs inside a
sandbox: fuel-metered, memory-capped, and able to do exactly one thing — refuse a write with a
reason. It cannot mutate, read other documents, observe time, or reach the network.

Every code sample on this page is imported from `examples/validator-rust/` in the Shadowcat
repository, which `src/server/tests/sandbox.rs` builds and runs on every push.

## Declaring a validator

Add a `validators` array to your module's `module.json`:

```jsonc
"validators": [{ "docType": "actor", "wasm": "validator.wasm" }]
```

`wasm` is a path relative to your module's own install folder; a path escaping that folder is
refused at scan time.

## The guest ABI

`wasm32-unknown-unknown`, no WASI, no imports except `env.log(ptr: i32, len: i32)` (debug-level
tracing, ≤ 1 KiB per call, ≤ 16 calls per validation — the 17th+ call is silently ignored).
Required exports:

```
memory                                // your linear memory
alloc(len: i32) -> i32                // the host asks you for a buffer to write the input into
validate(ptr: i32, len: i32) -> i32   // 0 = accept; non-zero = refuse
reason_ptr() -> i32                   // after a non-zero validate return
reason_len() -> i32
```

The host writes a UTF-8 JSON object at the pointer `alloc` returns:

```jsonc
{
  "docType": "actor",
  "op": "create",
  "system": { "hp": -1 },
  "prior": null,
  "name": "Goblin",
  "worldId": "…",
  "moduleId": "example-validator"
}
```

Nothing else reaches you: no `engine` band, no permissions, no other documents, no clock, no
randomness. Your validator is a pure function of this input.

## Limits

| Limit | Value |
|---|---|
| Module size | 4 MiB |
| Input size | 1 MiB |
| Guest memory | 16 MiB |
| Wall clock per call (measured, post-return) | 50 ms — a slower call still runs to completion, but counts as a fault |
| Wall clock hang guard (never-returning call) | 250 ms — the call is abandoned and counts as a fault |
| Consecutive faults before auto-disable | 5 |

## Building

No build script; a raw `cargo build` invocation:

```bash
cargo build --target wasm32-unknown-unknown --release
```

`examples/validator-rust/` is `no_std`, dependency-free, with a tiny bump allocator — see its
`src/lib.rs` for the complete `alloc`/`validate`/`reason_ptr`/`reason_len` implementation, which
refuses an `actor` whose `system.hp` is negative.

## Opting a world in

A GM enables your module, then — only once it declares at least one validator — a second
toggle, "Run sandboxed validators", appears in Settings → Installed modules. Both must be on;
disabling the module also stops its validators regardless of the toggle's own state.

See `docs/design/sandboxed-validators.md` in the Shadowcat repository for what the sandbox
does and does not protect against.
