// Advisory reconciliation of the client's loaded module topology against the
// server-broadcast world topology (Welcome.contract_declarations). Warn-only —
// the client renders from its own resolution; the server copy is the
// consistency authority. Hard enforcement lands with module management.
import type { Logger } from "./logger";
import type { ContractDeclaration, ContractProvide } from "./manifest";
import type { WireContractDeclaration } from "./wire";

/**
 * Reconciliation of the client's loaded module topology against the
 * server-broadcast world topology, keyed by `module_id`. Warn-only — the
 * client renders from its own resolution; the server copy is the consistency
 * authority. Presence mismatches are reported for modules on exactly one
 * side; for a `module_id` present on both sides, `version`, `provides`
 * (contract id set AND per-contract `cardinality`), and `requires` (contract
 * id set) are each compared and any mismatch is warned individually.
 * @param local This client's own loaded module contract declarations.
 * @param remote The server-broadcast `Welcome.contract_declarations`.
 * @param logger Diagnostic sink for each mismatch.
 * @example
 * ```ts
 * import { reconcileTopology, silentLogger } from "@shadowcat/core";
 *
 * reconcileTopology([], [], silentLogger);
 * ```
 */
export function reconcileTopology(
  local: ContractDeclaration[],
  remote: WireContractDeclaration[],
  logger: Logger,
): void {
  const localById = new Map(local.map((d) => [d.module_id, d]));
  const remoteById = new Map(remote.map((d) => [d.module_id, d]));
  for (const id of localById.keys()) {
    if (!remoteById.has(id)) {
      logger.warn(`module ${id} is loaded but absent from the world contract topology`);
    }
  }
  for (const id of remoteById.keys()) {
    if (!localById.has(id)) {
      logger.warn(`world contract topology declares module ${id} which is not loaded`);
    }
  }
  for (const [id, l] of localById) {
    const r = remoteById.get(id);
    if (!r) {
      continue;
    }
    if (l.version !== r.version) {
      logger.warn(
        `module ${id} version mismatch: locally loaded ${l.version}, world contract declares ${r.version}`,
      );
    }
    for (const msg of providesMismatches(id, l.provides, r.provides)) {
      logger.warn(msg);
    }
    for (const msg of requiresMismatches(id, l.requires, r.requires)) {
      logger.warn(msg);
    }
  }
}

/**
 * Compares two `provides` lists as a set keyed by `contract`, checking both
 * set membership and, for a contract id present on both sides, `cardinality`
 * equality.
 * @param moduleId The declaring module's id, used to prefix each warning.
 * @param local The local side's `provides` list.
 * @param remote The remote side's `provides` list.
 * @returns One warning message per discrepancy (added/removed contract id,
 * or a cardinality mismatch on a shared one).
 * @example
 * ```ts
 * providesMismatches("mod", [{ contract: "shadowcat.panel", cardinality: "singleton" }], []);
 * ```
 */
function providesMismatches(
  moduleId: string,
  local: ContractProvide[],
  remote: ContractProvide[],
): string[] {
  const localByContract = new Map(local.map((p) => [p.contract, p.cardinality]));
  const remoteByContract = new Map(remote.map((p) => [p.contract, p.cardinality]));
  const messages: string[] = [];
  for (const [contract, cardinality] of localByContract) {
    if (!remoteByContract.has(contract)) {
      messages.push(
        `module ${moduleId} provides ${contract} (${cardinality}) locally, which the world contract topology does not declare`,
      );
    }
  }
  for (const [contract, cardinality] of remoteByContract) {
    if (!localByContract.has(contract)) {
      messages.push(
        `module ${moduleId} world contract topology declares provides ${contract} (${cardinality}), which is not provided locally`,
      );
    }
  }
  for (const [contract, localCardinality] of localByContract) {
    const remoteCardinality = remoteByContract.get(contract);
    if (remoteCardinality !== undefined && remoteCardinality !== localCardinality) {
      messages.push(
        `module ${moduleId} provides ${contract} with cardinality mismatch: locally ${localCardinality}, world contract declares ${remoteCardinality}`,
      );
    }
  }
  return messages;
}

/**
 * Compares two `requires` lists as an unordered set of contract ids.
 * @param moduleId The declaring module's id, used to prefix each warning.
 * @param local The local side's `requires` list.
 * @param remote The remote side's `requires` list.
 * @returns One warning message per discrepancy (added/removed contract id).
 * @example
 * ```ts
 * requiresMismatches("mod", ["shadowcat.sheet:actor"], []);
 * ```
 */
function requiresMismatches(moduleId: string, local: string[], remote: string[]): string[] {
  const localSet = new Set(local);
  const remoteSet = new Set(remote);
  const messages: string[] = [];
  for (const contract of localSet) {
    if (!remoteSet.has(contract)) {
      messages.push(
        `module ${moduleId} requires ${contract} locally, which the world contract topology does not declare`,
      );
    }
  }
  for (const contract of remoteSet) {
    if (!localSet.has(contract)) {
      messages.push(
        `module ${moduleId} world contract topology declares requires ${contract}, which is not required locally`,
      );
    }
  }
  return messages;
}
