// Advisory reconciliation of the client's loaded module topology against the
// server-broadcast world topology (Welcome.contract_declarations). Warn-only —
// the client renders from its own resolution; the server copy is the
// consistency authority. Hard enforcement lands with module management.
import type { Logger } from "./logger";
import type { ContractDeclaration } from "./manifest";

interface WireLike {
  module_id: string;
}

/**
 * Presence-only reconciliation: warn for each module present on exactly one
 * side, keyed by `module_id`. Version and the `provides`/`requires` payload are
 * NOT compared — a same-id/different-contract-set drift reconciles silently.
 * Richer mismatch detection is deferred to module management (see TODO.md).
 */
export function reconcileTopology(
  local: ContractDeclaration[],
  remote: WireLike[],
  logger: Logger,
): void {
  const localIds = new Set(local.map((d) => d.module_id));
  const remoteIds = new Set(remote.map((d) => d.module_id));
  for (const id of localIds) {
    if (!remoteIds.has(id)) {
      logger.warn(`module ${id} is loaded but absent from the world contract topology`);
    }
  }
  for (const id of remoteIds) {
    if (!localIds.has(id)) {
      logger.warn(`world contract topology declares module ${id} which is not loaded`);
    }
  }
}
