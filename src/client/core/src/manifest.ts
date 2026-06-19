// The module manifest: identity, semver, dependencies, declared capabilities,
// declarative path->capability requirements, and declared hooks. Validated with
// Zod before a module is admitted to the registry. The `requirements` are the
// data the GM publishes to the server's per-world capability_requirements record.
import { z } from "zod";
import type { HookKind } from "./hooks";

export interface CapRequirement {
  path_prefix: string;
  caps: string[];
}
export interface HookDecl {
  name: string;
  version: string;
  kind: HookKind;
}
export interface ModuleManifest {
  id: string;
  version: string;
  name?: string;
  dependencies: Record<string, string>;
  capabilities?: string[];
  requirements?: CapRequirement[];
  hooks?: HookDecl[];
}

const HookKindSchema = z.enum(["info", "mutate", "cancel"]);

const CapRequirementSchema = z.object({
  path_prefix: z.string().startsWith("/"),
  caps: z.array(z.string()).min(1),
});

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
});

export function parseManifest(value: unknown): ModuleManifest {
  return ManifestSchema.parse(value);
}
