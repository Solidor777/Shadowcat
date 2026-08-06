// The module manifest: identity, semver, dependencies, declared capabilities,
// declarative path->capability requirements, and declared hooks. Validated with
// Zod before a module is admitted to the registry. The `requirements` are the
// data the GM publishes to the server's per-world capability_requirements record.
import { z } from "zod";
import type { HookKind } from "./hooks";
import type { Cardinality } from "./contributions";

/** One declarative path→capability requirement a module publishes; the GM's aggregate of these
 * (unioned with the world's own policy) reaches the client as advisory metadata only — see
 * `welcome_capability_requirements` (`src/server/src/ws/conn.rs:1112`), which is NOT the
 * server-side write-enforcement input (`apply_intent` in `data/sqlite.rs` consults only the
 * GM-authored `world_cap_requirements` record). The client stores the already-unioned result
 * verbatim (`worldSession.svelte.ts:672`); it performs no union logic of its own. */
export interface CapRequirement {
  /** Document-pointer path prefix this requirement applies to, e.g. `"/system"`. Validated by
   * `CapRequirementSchema` to start with `/`. */
  path_prefix: string;
  /** Capability names required on `path_prefix`; non-empty (`CapRequirementSchema` enforces
   * `min(1)`). */
  caps: string[];
}
/** A hook a module declares in its manifest (identity + kind), distinct from the runtime
 * `HookDefinition` passed to `ModuleContext.hooks.defineHook`. */
export interface HookDecl {
  /** The hook's name, as registered with `HookBus.defineHook`. */
  name: string;
  /** The hook's declared semver version. */
  version: string;
  /** The hook's dispatch kind (`"info" | "mutate" | "cancel"` — see `HookKind` in `hooks.ts`). */
  kind: HookKind;
}

/** A UI surface contract a module provides, with its cardinality. */
export interface ContractProvide {
  /** The contract id, e.g. `"shadowcat.panel"` or `"shadowcat.sheet:actor"`. */
  contract: string;
  /** `"singleton"` (one active provider, collision aborts activation of the second) or
   * `"multi"` (`ModuleRegistry.activate`'s singleton-collision check in `modules.ts`). */
  cardinality: Cardinality;
}

/** A module's UI contract declaration (structurally matches the ts-rs type). */
export interface ContractDeclaration {
  /** The declaring module's id. */
  module_id: string;
  /** The declaring module's semver version. */
  version: string;
  /** Contracts this module provides; compared locally by `reconcileTopology` against the
   * server-broadcast `Welcome.contract_declarations`. */
  provides: ContractProvide[];
  /** Contract ids this module requires at least one active provider of. */
  requires: string[];
}

/** Minimal engine-compat gate (T6, M13-1). Optional on the shared manifest
 * shape (first-party modules never set it — they ship version-locked inside
 * the binary); the modules-folder install/enable/load pipeline treats a
 * missing or unsatisfied range as a hard reject for community modules
 * specifically (see `loader.ts`'s `checkEngineCompat` and the server's
 * `engine_compat_ok`). */
export interface ModuleEngines {
  /** Semver range this module requires of the host engine, e.g. `"^1.2.0"` — matched by
   * `semver.ts`'s `satisfies` (caret-0.x leftmost-non-zero fix), mirrored server-side by
   * `semver_satisfies` in `src/server/src/modules.rs`. */
  shadowcat: string;
}

/**
 * The module manifest: identity, semver, dependencies, declared capabilities, declarative
 * path→capability requirements, and declared hooks. This interface — not `ManifestSchema` — is
 * the statement of record for a manifest's shape; `ManifestSchema` is a runtime validator
 * annotated (`z.ZodType<ModuleManifest>`) to conform to it. That annotation's compile-time
 * guarantee is PARTIAL, verified by direct experiment (a scratch schema/interface pair compiled
 * with `tsc`, then deleted): adding a new REQUIRED field to this interface without adding a
 * matching key to `ManifestSchema`'s `z.object({...})` IS a compile error at the `ManifestSchema`
 * declaration (`TS2322`, the object schema's inferred output is no longer assignable to
 * `ModuleManifest`). Adding a new OPTIONAL field (`foo?: string`) is NOT a compile error — an
 * object type missing an optional property is still structurally assignable to a type that
 * declares it — and the gap is not merely a missing type check: `z.object()` strips unrecognized
 * input keys by default (confirmed at runtime: `z.object({ id: z.string() }).parse({ id: "x",
 * foo: "y" })` returns `{ id: "x" }`), so a manifest author who sets `foo` in `module.json` has
 * that field silently dropped by `parseManifest` with no validation error and no field in the
 * returned value, despite the interface saying it may be present.
 */
export interface ModuleManifest {
  /** Unique module id; validated non-empty by `ManifestSchema` (`min(1)`). */
  id: string;
  /** Semver version string; validated non-empty by `ManifestSchema` (`min(1)`). */
  version: string;
  /** Display name; absent means the module has no human-readable name distinct from `id`. */
  name?: string;
  /** Module-id → semver-range map; `ModuleRegistry.depsSatisfied` requires each entry's module
   * present, active, and satisfying the range before this module can activate. */
  dependencies: Record<string, string>;
  /** Capability names this module claims to use; declarative metadata only — the registry does
   * not gate activation on it (distinct from `requirements`, which the GM publishes to the
   * server). */
  capabilities?: string[];
  /** Path→capability requirements this module publishes toward the server's per-world
   * `capability_requirements` record; absent means none. See `CapRequirement`. */
  requirements?: CapRequirement[];
  /** Hooks this module declares; absent means none. */
  hooks?: HookDecl[];
  /** UI surface contracts this module provides; absent means none (`declarationOf` normalizes
   * to `[]`). */
  provides?: ContractProvide[];
  /** UI surface contract ids this module requires at least one active provider of; absent means
   * none (`declarationOf` normalizes to `[]`). */
  requires?: string[];
  /** Engine-compat range; optional because first-party modules never set it (see `ModuleEngines`
   * doc) while community modules must. */
  engines?: ModuleEngines;
}

const HookKindSchema = z.enum(["info", "mutate", "cancel"]);

const CapRequirementSchema = z.object({
  path_prefix: z.string().startsWith("/"),
  caps: z.array(z.string()).min(1),
});

const ModuleEnginesSchema = z.object({ shadowcat: z.string().min(1) });

export const ManifestSchema: z.ZodType<ModuleManifest> = z.object({
  id: z.string().min(1),
  version: z.string().min(1),
  name: z.string().optional(),
  dependencies: z.record(z.string()),
  capabilities: z.array(z.string()).optional(),
  requirements: z.array(CapRequirementSchema).optional(),
  hooks: z
    .array(z.object({ name: z.string(), version: z.string(), kind: HookKindSchema }))
    .optional(),
  provides: z
    .array(z.object({ contract: z.string(), cardinality: z.enum(["singleton", "multi"]) }))
    .optional(),
  requires: z.array(z.string()).optional(),
  engines: ModuleEnginesSchema.optional(),
});

/** Validates and parses an unknown value as a `ModuleManifest`; throws a Zod
 * error on shape mismatch.
 * @param value The candidate manifest, typically `module.json` parsed as JSON.
 * @returns The validated `ModuleManifest`.
 * @example
 * ```ts
 * import { parseManifest } from "@shadowcat/core";
 *
 * const manifest = parseManifest({ id: "example", version: "1.0.0", dependencies: {} });
 * ```
 */
export function parseManifest(value: unknown): ModuleManifest {
  return ManifestSchema.parse(value);
}

/** Project a manifest to its UI contract declaration (empty arrays when unset).
 * @param m The module manifest.
 * @returns The `ContractDeclaration`, compared locally by `reconcileTopology` against the server-broadcast `Welcome.contract_declarations`.
 * @example
 * ```ts
 * import { declarationOf, parseManifest } from "@shadowcat/core";
 *
 * const manifest = parseManifest({ id: "example", version: "1.0.0", dependencies: {} });
 * declarationOf(manifest);
 * ```
 */
export function declarationOf(m: ModuleManifest): ContractDeclaration {
  return {
    module_id: m.id,
    version: m.version,
    provides: m.provides ?? [],
    requires: m.requires ?? [],
  };
}
