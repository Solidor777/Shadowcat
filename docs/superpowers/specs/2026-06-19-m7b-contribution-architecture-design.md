# M7b — UI Contribution Architecture: Design Spec

> Status: **DRAFT for review.** The foundation sub-milestone of M7 (parent spec:
> [`2026-06-19-m7-layout-theming-design.md`](2026-06-19-m7-layout-theming-design.md),
> §5). Builds the generic, framework-neutral mechanism by which every UI element
> is a module contributing components into **surfaces** declared by other modules,
> with contract-based (virtual) dependencies — on the existing M6b module system.
> Decomposed into **M7b-1** (contract schema + server mirror), **M7b-2** (client
> registry + resolution), **M7b-3** (Svelte `<Surface>` adapter + harness).
>
> **Scope note (server-mirrored path, chosen at brainstorm):** the contract
> declarations are mirrored server-side (stored, validated, broadcast), against
> the recorded tradeoff that the server is otherwise module-free and has no
> server-side module registry yet (parent §"open decisions"). M7b builds the
> declaration + validation + distribution layer; it does NOT build module
> distribution/loading or hard server enforcement of the client's loaded set —
> those are decomposed to later milestones (see §7).

## 1. Goal

A generic contribution architecture, assumption-free about any specific UI
element, such that:

1. A module **declares a surface** (a named mount point, by string contract) and
   any module **contributes** components into surfaces by contract.
2. Inter-module dependencies resolve **by contract** ("requires *a* sidebar"),
   generalizing M6b's id+semver dependencies.
3. The contract vocabulary is a **language-neutral schema** (shared Rust↔TS via
   ts-rs), mirrored server-side for validation + cross-client consistency.
4. The default renderer is Svelte (`<Surface>`), but the registry is
   **framework-neutral** — a Vue (or other) host implements its own renderer
   against the same `@shadowcat/core` registry.

This is the foundation M7c (shell + entry flow as modules) and every later UI
milestone (chat M11, combat tracker, browsers M12) build on.

## 2. Non-goals (explicitly later)

- **Module distribution/installation** and **server-authoritative module
  *loading*** — M7b clients still load their own modules; the server validates +
  broadcasts declarations but does not push/enforce a module set. (Phase 4
  module-management territory.)
- **Hard client-side enforcement** of the `Welcome` topology — M7b reconciles
  client-loaded modules against the broadcast and **warns** on mismatch; it does
  not refuse to render. Full enforcement lands with module management.
- **Multi-provider `singleton` conflict *policy*** + capability **version
  negotiation** — deferred (logged in `TODO.md`); M7b uses deterministic
  loud-fail on a duplicate `singleton`. (Parent §2.)
- The real shell, `core-ui` module, entry-flow views, and the `embed.rs` flip —
  all M7c. M7b ships only the mechanism + a minimal test harness.
- Theming, i18n, session wiring — M7d.

## 3. The contract model (shared schema)

- **Contract** — a string id naming an extension point, e.g.
  `shadowcat.surface:sidebar`. Core attaches no meaning to specific ids.
- **Surface** — a UI contract that is a mount point (rendered by `<Surface>`).
- **Contribution** — `{ id, contract, order?, props?, component }` placed into a
  surface; `component` is an opaque handle (`unknown` at the neutral layer).
- **Cardinality** — `"singleton"` (one provider) | `"multi"` (many).
- **Declaration** (the portable, server-mirrored unit):
  ```
  ContractDeclaration {
    module_id: string,
    version: string,
    provides: { contract: string, cardinality: "singleton" | "multi" }[],
    requires: string[],   // contract ids needing an active provider
  }
  ```
  Emitted as ts-rs types to `src/types/generated/` — the single Rust↔TS source of
  truth for the wire shape.

## 4. Server mirror (Rust) — mirrors the capability-requirements pattern

### 4.1 Storage
Per-world module contract declarations, GM-published. Stored by the **same
mechanism the existing `capability_requirements` use** (the M7b-1 plan inspects
`set_world_cap_requirements`/`world_cap_requirements` in `sqlite.rs` and mirrors
it — same table-vs-column choice, same serde encoding), accessed by new parallel
`SqliteRepository` methods `world_contract_declarations(world)` /
`set_world_contract_declarations(world, decls)`. Migration-added if a new table.

### 4.2 Endpoints (GM-only)
- `GET /api/worlds/{id}/contracts` → `Vec<ContractDeclaration>`.
- `PUT /api/worlds/{id}/contracts` → 204, replacing the set.

Both gated by the existing `require_gm`. Mirrors
`get/set_world_capability_requirements` (routes.rs) exactly.

### 4.3 Validation on PUT (server is the consistency authority, fail-closed 422)
Bounded count (`MAX_CONTRACT_DECLARATIONS`, e.g. 256). For the declared set:
- Every `requires` contract has ≥1 `provides` of that contract somewhere in the
  set (no dangling requirement).
- No two declarations `provides` the **same `singleton`** contract (the loud-fail
  rule, server-enforced).
- Every contract string is well-formed (non-empty `<namespace>:<name>` shape,
  reusing the `validate_capability`-style structural check).
- `module_id` / `version` non-empty.
A violation returns `AppError::Unprocessable` with a specific message — same rigor
and shape as `set_world_capability_requirements`.

### 4.4 Broadcast
Add `contract_declarations: Vec<ContractDeclaration>` to `ServerMsg::Welcome`
(protocol.rs), populated in the egress task (conn.rs) alongside
`capability_requirements`, loaded once per connection. Read failure → empty +
`tracing::warn` (advisory, like the requirements fallback).

### 4.5 The module-free invariant holds
The server stores/validates/distributes **declaration strings only**. It never
holds components, never renders, never executes module code.

## 5. Client registry (`@shadowcat/core`, framework-neutral)

### 5.1 `ContributionRegistry` (Svelte-free, neutral reactive)
New `src/client/core/src/contributions.ts`:
```ts
export interface Contribution {
  id: string;
  contract: string;
  order?: number;            // ascending; default 0
  props?: Record<string, unknown>;
  component: unknown;        // opaque handle; the host knows how to render it
}
export class ContributionRegistry {
  contribute(c: Contribution): () => void;             // returns dispose
  contributionsFor(contract: string): readonly Contribution[]; // sorted by order, then insert order
  subscribe(listener: () => void): () => void;         // neutral reactivity (as DocumentStore)
  removeModule(moduleId: string): void;                // per-module teardown
}
```
Same `subscribe`/snapshot shape as `DocumentStore` — no Svelte runes; any
framework adapts it.

### 5.2 Exposure on `ModuleContext`
Add `contributions` to `ModuleContext` (modules.ts), constructed once in the core
bootstrap and passed via `contextFor`. A contribution registered through
`ctx.contributions.contribute(...)` is tagged with `ctx.moduleId`; the registry's
`removeModule` is called from `ModuleRegistry.unload` alongside the existing
hooks/services/middleware teardown.

### 5.3 Manifest: `provides` / `requires`
Extend `ModuleManifest` (manifest.ts) — distinct from the security `capabilities`
field:
```ts
provides?: { contract: string; cardinality: "singleton" | "multi" }[];
requires?: string[];
```
Zod-validated (`ManifestSchema`) and ts-rs-aligned (the shared `ContractDeclaration`
shape; the manifest `provides`/`requires` + `id`/`version` project to a
`ContractDeclaration`).

### 5.4 Resolution (modules.ts — generalize existing logic)
- `depsSatisfied`: in addition to id+semver `dependencies`, every `requires`
  contract must have ≥1 **active** module whose `provides` includes it.
- `topoSort`: add edges requirer → each provider of a required contract, so
  providers activate first; cycles still throw.
- **Singleton loud-fail:** activating a module that `provides` a `singleton`
  contract already provided by another active module throws (mirrors
  `ServiceRegistry.provide`'s duplicate-name hard error). A `requires` with no
  provider → the module is not activated (existing "dependency unmet" path), at
  `logger.warn`.

### 5.5 `Welcome` topology reconciliation (advisory in M7b)
On `Welcome`, the client compares its locally-loaded modules' declarations against
the broadcast `contract_declarations`; a mismatch (locally-loaded module absent
from the world set, or vice-versa) is surfaced via `logger.warn`. M7b does not
refuse to render on mismatch (§2 non-goal). Client resolution (§5.4) drives
rendering; the server copy is the consistency/validation authority.

## 6. Svelte `<Surface>` adapter (`ui` package — the default host)

### 6.1 `<Surface>` component
`src/client/ui/src/lib/Surface.svelte`:
- Props: `contract: string`.
- Wraps `registry.subscribe` with `createSubscriber` (`svelte/reactivity`); reads
  `registry.contributionsFor(contract)`; renders each contribution's opaque
  `component` as a Svelte 5 dynamic component, keyed by `contribution.id`, passing
  `contribution.props`.
- Re-renders reactively when contributions change; the dynamic component's own
  lifecycle handles mount/teardown.

### 6.2 Contribution input contract (ambient state)
Ambient app state — `{ core, store, world, t (i18n), role }` — is provided once at
the shell root via Svelte context (`setContext`/`getContext` with a typed key in
`src/client/ui/src/lib/appContext.ts`). `<Surface>` is rendered within that
context, so every contributed component reads it via `getContext`. Per-contribution
`props` (plain data from registration) are passed explicitly. (Adjustable; this is
the proposed default.)

### 6.3 Framework neutrality
`<Surface>` is the Svelte default host only. The registry (§5) is the neutral
contract; an alternate-framework host renders the same opaque handles its own way.
Only `component` values are framework-specific.

## 7. Decomposition (one spec, three plan→execute→review cycles)

- **M7b-1 — Contract schema + server mirror (Rust + shared types).** ts-rs
  `ContractDeclaration`; migration + repo accessors; `GET/PUT
  /api/worlds/{id}/contracts` (GM-only, validated); `Welcome` extension. Rust
  tests mirroring capability-requirements. **Buddy-check candidate** (new GM-only
  write surface + Welcome change + validation correctness).
- **M7b-2 — Client registry + resolution (`@shadowcat/core`).**
  `ContributionRegistry`; `ModuleContext.contributions` + unload teardown;
  manifest `provides`/`requires` (Zod + ts-rs alignment); generalized
  `depsSatisfied`/`topoSort` + singleton loud-fail; `Welcome` reconciliation
  warn. Vitest.
- **M7b-3 — Svelte `<Surface>` adapter + harness (`ui` package).** `<Surface>`
  component; `appContext`; a minimal test harness (a fake module contributing a
  component into a surface renders, reorders by `order`, and tears down on
  dispose). Vitest + @testing-library/svelte (new to the ui package).

## 8. Testing

- **Rust (M7b-1):** PUT validation — requires-satisfied accept, dangling-requires
  reject (422), duplicate-singleton reject (422), malformed-contract reject (422),
  over-count reject; GM-only gating; GET round-trip; `Welcome` carries the
  declarations; read-failure → empty + warn.
- **Core (M7b-2):** registry contribute/dispose/`contributionsFor` ordering/
  `subscribe` notification/`removeModule`; resolution — provider-before-requirer
  ordering, missing-provider non-activation, duplicate-singleton throw; manifest
  Zod accept/reject; `Welcome` reconciliation warn on mismatch.
- **UI (M7b-3):** `<Surface>` renders contributions sorted by `order`, updates
  reactively on contribute/dispose, injects ambient context, passes per-contribution
  props; an empty surface renders nothing.

## 9. Decisions (resolved at brainstorm)

1. **Surface rendering → declarative `<Surface contract>` Svelte component**
   (Svelte 5 dynamic components), not imperative DOM-target mounting.
2. **Registry → framework-neutral in `@shadowcat/core`** with `subscribe`/snapshot
   reactivity and opaque (`unknown`) component handles; Svelte `<Surface>` is a
   thin adapter. A Vue host uses the same registry.
3. **Server mirrors contract declarations now** — stored, validated, broadcast in
   `Welcome`, mirroring capability-requirements. Tradeoff (server otherwise
   module-free; no server module registry yet) recorded and accepted; module
   distribution/loading + hard enforcement deferred (§2, §7).
4. **Contribution input → ambient Svelte context** (`core`, `store`, `world`, `t`,
   `role`) + explicit per-contribution `props`.
5. **Singleton conflict → deterministic loud-fail** now; the resolution *policy*
   is deferred (`TODO.md`).
