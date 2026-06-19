# M6b — Modules + Capabilities (declarative): Design Spec

> Status: **DRAFT for review.** Second of the three M6 sub-milestones (M6a client
> core ✅, M6b modules+capabilities, M6c search). Scope: the framework-neutral TS
> module system (hook bus, service registry, middleware, manifest + loader) in the
> existing `@shadowcat/core`, plus Capability **Phase 2** (declarative,
> data-driven, field-path-scoped capability requirements — the one server-side
> change). No UI, no Svelte (M7); no FTS5 / `Core.search` (M6c).

## 1. Goal

Make `@shadowcat/core` extensible by trusted, GM-activated modules without
introducing any third-party code on the server. A module declares its identity,
dependencies, hooks, and capability requirements in a manifest; the loader
resolves and activates it in dependency order; the module registers behavior
through hooks, services, and middleware against a capability-scoped context.
Capability **Phase 2** generalizes Phase 1's fixed path→capability map into a
per-world, data-driven map that the server enforces structurally and the client
replicates for advisory UI gating.

## 2. Invariants preserved

- **Server runs no module code; structural validation only** (ARCHITECTURE #6).
  Declarative capability requirements reach the server as pure **data** (a
  per-world record), never as module manifests or code. The server enforces
  capability *possession* against field paths — it never interprets the `system`
  body's meaning.
- **Framework-neutral, Svelte-free headless core** (ARCHITECTURE #7). All module
  primitives are plain TS in `src/client/core/`; no Svelte/DOM in the dependency
  closure. The HookBus subscription surface taps the same observer events the M6a
  store already emits.
- **Server-authoritative with rollback** (ARCHITECTURE #1, #3). Client
  capability-awareness is advisory UX only; the server remains the sole authority
  and a denied write returns `Reject{Forbidden}`, handled by M6a's existing
  rollback path.
- **Additive capability model** (Phase 1 carry-over). Declarative requirements
  add required caps on top of the structural base; they never silently widen or
  remove the Phase-1 floor.

## 3. Package & boundaries

All new client code lands in `src/client/core/src/` (the existing
`@shadowcat/core`, Svelte-free, vitest-tested). One Rust change set spans the
data layer and the WS `Welcome` frame. No new client framework dependencies.

| Unit | File | Purpose | Depends on |
|---|---|---|---|
| HookBus | `hooks.ts` | Pub/sub extension points; 3 dispatch kinds; versioned | logger |
| ServiceRegistry | `services.ts` | Named singletons modules provide/consume | — |
| Middleware | `middleware.ts` | Ordered `next()` pipelines (intent-submit, inbound-event) | — |
| ModuleRegistry | `modules.ts` | Manifest validation, topo-sort, semver deps, hot-unload | hooks, services, middleware |
| Loader adapter | `loader.ts` | Dynamic-`import()`s manifests → feeds registry | modules |
| Client cap-awareness | `capabilities.ts` | Mirrors server `resolve_access`; advisory gating | store, wire |
| Server (Rust) | data + ws | `world_capability_requirements` record + `apply_intent` enforcement + `Welcome` extension | existing cap layer |

## 4. Hook system (`hooks.ts`)

**Open namespaced string keyspace.** Hook names are `namespace:event`
(`core:documentUpdate`, `dnd5e:preRollAttack`). The keyspace is open at runtime
because "Fully Modular" requires modules to define hooks the core cannot know at
compile time. A **typed overlay** — a `CoreHooks` interface mapping known hook
names to payload types, declaration-merge-able by first-party code — gives
compile-time safety for core/typed hooks; module-defined names carry
Zod-validated payloads at the boundary.

> Rationale: chosen on technical merits (type-safety + open extensibility), not
> on resemblance to any existing VTT. Familiarity with another tool is never a
> design justification in this project.

**Three dispatch kinds**, each a distinct contract:

- **informational** — `emitInfo(name, p) → Promise<void>`. Fire-and-forget;
  awaits all handlers; return values ignored.
- **mutating** — `emitMutate(name, p) → Promise<P>`. Each handler receives the
  current payload and returns the next; chained in order.
- **cancellable** — `emitCancel(name, p) → Promise<{ cancelled: boolean; by?: string }>`.
  Halts when a handler returns `false` / a `Stop` sentinel.

**API:**

- `defineHook(name, { version, kind })` — declares the hook's semver and kind.
  Re-defining with an incompatible version is a structured error.
- `on(name, handler, { module, priority?, requires? })` → unsubscribe. Records
  the owning module id (for unload cleanup) and checks `requires` (a semver
  range) against the hook definition's version; incompatible registration is
  refused. Ordering is by `priority` (default 0), then registration order.
- `emitInfo` / `emitMutate` / `emitCancel` as above. Typed wrappers `on<K>` /
  `emit*<K>` over `CoreHooks` for statically-known names.

**Error isolation.** A throwing handler is caught and logged via the core
logger; informational and cancellable chains continue past it; a mutating
handler that throws is skipped and the prior payload is carried forward. One
faulty module cannot corrupt a pipeline or abort dispatch.

**Versioning.** Each hook definition carries a declared semver. As core hooks
evolve, modules that registered with an incompatible `requires` range are
refused at registration (warned in dev), protecting module compatibility.

## 5. Service registry (`services.ts`)

Named singletons a module provides for others to consume:
`provide(name, impl, { module, version })`, `get(name)`, `has(name)`. A duplicate
`provide` for the same name is an **error** (no silent override). Versions are
recorded for consumers. All of a module's services are removed on unload.

## 6. Middleware (`middleware.ts`)

A small, explicit set of ordered `next()`-style pipelines. v1 ships two:

- **intent-submit** — modules may observe, transform, or cancel an outgoing
  optimistic intent before it reaches `OptimisticClient`.
- **inbound-event** — modules may observe a confirmed event as it is applied.

`use(pipeline, mw, { module })`; each `mw` is `(ctx, next) => next()` and may
short-circuit. Middleware differs from hooks: it is ordered and can short-circuit
the whole call via `next`-control; hooks broadcast at a point. Both are PLAN
deliverables and coexist. Removed on module unload.

## 7. Module registry & manifest (`modules.ts`)

**`ModuleManifest`** (Zod-validated):

```ts
{
  id: string,                                  // unique module id
  version: string,                             // semver
  name?: string,
  dependencies: Record<string, string>,        // id -> semver range
  capabilities?: CapabilityDecl[],             // declared namespaced caps + requirements
  hooks?: HookDecl[],                          // hooks this module defines
}
```

**`Module`** = `{ manifest, register(ctx), unregister? }`.

**Lifecycle:**

- `register(module)` — validates the manifest (Zod), checks each dependency is
  present and semver-satisfied. On failure, the module is rejected with a
  structured error; other modules are unaffected.
- `activate()` — **topological sort** by `dependencies`; a cycle aborts
  activation with the cycle path in the error. Calls each `register(ctx)` in
  dependency order.
- **`ModuleContext` (the security chokepoint)** — every module receives only
  capability-scoped handles: a scoped HookBus (auto-tags registrations with the
  module id), the ServiceRegistry, middleware `use`, a **read-only** view of the
  document store, the `OptimisticClient` for writes, and a logger.
- **Hot-unload** — every registration made through the scoped context is tracked
  per module id. `unload(id)` removes that module's hook listeners, services, and
  middleware. Unloading a depended-upon module is **refused** unless
  `{ cascade: true }`, which unloads dependents first (reverse-topo).

**Local module registry** — an in-memory catalog of known modules and their
per-world enabled/disabled state. Populated by the loader adapter.

## 8. Loader adapter (`loader.ts`)

`loadFromManifests(dir, importFn)` resolves manifest files, dynamic-`import()`s
each module's entry to obtain its `Module` object, and feeds the
`ModuleRegistry`. `importFn` is injectable so Node tests drive it without a real
filesystem/bundler. The adapter is deliberately thin — all ordering and
lifecycle live in the registry, so browser, native, and a future Phase-3
sandboxed delivery are alternate adapters behind the **same registry interface**
(ARCHITECTURE §4 "seam now, build behind it later").

## 9. Capability Phase 2 — declarative requirements

### 9.1 Server-side (the only Rust change)

- New per-world data record **`world_capability_requirements`**: a JSON list of
  `{ path_prefix: string, required_caps: string[] }`, stored exactly like the
  existing `world_capability_defaults` and **loaded once per connection**
  (matching commit `c15a1b4`). Written via a config operation gated by
  `core:edit_permissions` / GM (HTTP + WS). The **client** derives the values
  from activated modules' manifest declarations; the **server only stores and
  enforces pure data** — manifests never reach the server.
- **`apply_intent` enforcement.** Phase 1's fixed §3.1 path map
  (`/system`→`core:write_fields`, `/embedded`→`core:manage_embedded`,
  `/permissions`→`core:edit_permissions`) becomes the structural **base**. For
  each `FieldChange.path`, the **most-specific matching** requirement prefix
  contributes caps that are **additive over** the base: writing `/system/vision`
  requires `core:write_fields` **and** `dnd5e:gm_vision`. A multi-change `Update`
  is all-or-nothing (the actor must hold every required cap for every change) in
  the existing single transaction. The server still treats every capability as
  an opaque token; `core:*` remain the only server-understood ones.

### 9.2 `Welcome` extension

`Welcome` gains three fields so the client can replicate resolution:

- `world_default_grants` — the world's `CapabilityGrants`.
- `actor_role` — the connecting actor's `WorldRole`.
- `capability_requirements` — the declarative path→caps map.

ts-rs regenerates `ServerMsg.ts`; the M6a `wire.ts` `Welcome` Zod schema and
`ws-client` accept the new fields. Server and client ship as one binary, so wire
versions always match.

### 9.3 Client capability-awareness (`capabilities.ts`)

Mirrors the server's `resolve_access`: from a document's `PermissionSet`, the
world-default grants, the actor role, and the declarative requirements map, it
answers "may this actor write path P on document D?" to gate module UI and
actions. **Advisory only** — the server remains authoritative; a crafted client
that bypasses the gate is rejected at `apply_intent`.

## 10. Data flow

1. **Activate** — loader → `register(manifest)` → topo-sort → `register(ctx)`;
   `ctx.hooks.on(...)` is tracked under the module id; capability declarations
   are collected.
2. **Publish requirements** — a GM enables modules in a world → the client unions
   their capability-requirement declarations → writes
   `world_capability_requirements` via the gated config op → the server stores it
   → the next `Welcome` broadcasts it to all clients.
3. **Gated write** — a player edits `/system/vision`; cap-awareness sees the
   `dnd5e:gm_vision` requirement and that the actor lacks it → UI disabled, no
   intent sent. A crafted client that sends it anyway → `apply_intent` →
   `Reject{Forbidden}` → M6a rollback. Server-authoritative holds.
4. **Hook fire** — a confirmed event applied to the store → core emits
   `core:documentUpdate` (informational) → module listeners react.

## 11. Error handling

| Condition | Behavior |
|---|---|
| Manifest fails Zod validation | Module rejected with structured error; others unaffected |
| Missing / incompatible dependency | Module + its dependents skipped; logged |
| Dependency cycle | Activation aborts; error names the cycle path |
| Hook handler throws | Caught + logged; info/cancel continue; mutate skips thrower, carries prior payload |
| Hook version incompatible | Registration refused (warned in dev) |
| Duplicate service name | `provide` errors |
| Unload of depended-upon module | Refused unless `{ cascade: true }` |
| Unauthorized server write | `Reject{Forbidden}`; M6a rollback |
| Invalid `Welcome` frame | Existing M6a drop-and-log |

## 12. Testing

- **Unit (vitest, TS):** HookBus (3 kinds, priority/order, version refusal, error
  isolation, unload cleanup); ServiceRegistry (provide/get, conflict, unload);
  middleware (order, short-circuit, transform); ModuleRegistry (manifest
  validation, topo-sort, cycle detection, dep semver, hot-unload incl.
  dependents); loader adapter (injected `importFn`); cap-awareness (resolution
  mirrors the server cases). A tiny **obviously-synthetic example module**
  (hooks + service + capability declaration) serves as a shared fixture.
- **Rust integration:** `apply_intent` declarative enforcement (accept-with-cap,
  reject-without, additive-over-base, all-or-nothing multi-change);
  `world_capability_requirements` CRUD gated by `core:edit_permissions`/GM;
  `Welcome` payload includes the new fields.
- **Node↔Rust e2e (pays down the M6a-deferred TODO):** build the Rust
  `test_server` and drive the real `@shadowcat/core` client over a real WS:
  connect → `Welcome` carries grants/role/requirements → a capability-gated
  intent is accepted/rejected, asserted against the authoritative `world_events`
  log. Delivered with a **new combined CI job** carrying both toolchains.

## 13. Execution slices (for the implementation plan)

1. HookBus + ServiceRegistry + middleware (unit-tested).
2. ModuleManifest + ModuleRegistry + ModuleContext + loader adapter
   (unit-tested).
3. Server: `world_capability_requirements` record + CRUD + `apply_intent`
   declarative enforcement + `Welcome` extension + ts-rs regen (Rust
   integration-tested).
4. Client capability-awareness + `wire.ts` `Welcome` schema (unit-tested).
5. Node↔Rust e2e harness + combined CI job.
6. Docs sync (PLAN/ARCHITECTURE mirror as needed, TODO paydown).

## 14. Decisions settled in brainstorming

1. **Scope** — one M6b spec covering hooks+loader+capabilities (they couple
   through the manifest); sliced execution (§13).
2. **Hook model** — typed overlay over an open runtime keyspace; three explicit
   dispatch kinds; per-hook semver versioning. Middleware is a separate
   primitive. Chosen on merits, not on resemblance to existing VTTs.
3. **Module loading** — hybrid: an import-agnostic `ModuleRegistry` (the durable
   security chokepoint) plus a thin `import()` loader adapter; alternate delivery
   mechanisms become alternate adapters.
4. **Declarative caps** — a per-world data record written by a GM-gated config
   op; the server stays 100% structural; requirements are **additive** over the
   Phase-1 base.
5. **Testing** — build the real Node↔Rust e2e harness now (plus Rust integration
   and TS unit coverage).

## 15. Open decisions (for review)

1. **Middleware pipeline set.** v1 ships `intent-submit` + `inbound-event`. Is
   that the right initial set, or is one sufficient until a consumer appears?
2. **`requires` default.** When a listener omits `requires`, accept any hook
   version (proposed) vs. require an explicit range.
3. **Per-world vs per-world×doc_type requirements.** §9.1 proposes a flat
   per-world map; Phase 1's world defaults left `doc_type` keying open (capability
   spec §7.2). Mirror that here or keep flat for M6b?
