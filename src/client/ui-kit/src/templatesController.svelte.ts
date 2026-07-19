// Template merge orchestration (M13e). Thin glue: pure core functions → the conflict modal →
// `dispatchIntent`. Holds a reactive `pending` conflict session the `TemplateModalHost` renders.
// Constructed by the shell alongside `SheetsController`; imports no module.
import {
  computePull, computeRevert, planToUpdate, applyResolutions, findInstances, syncState, stampInstance,
  type WireDocument, type WireOperation, type StampOpts, type SyncState, type Logger,
  type DocumentStore, type ReadableDocuments, type MergePlan,
} from "@shadowcat/core";
import type { ConflictGroup } from "./MergeConflictModal.svelte";

export interface TemplatesControllerDeps {
  store: DocumentStore;
  documents: ReadableDocuments;
  dispatchIntent: (ops: WireOperation[]) => void;
  role: "gm" | "player" | "spectator";
  selfId: string;
  /** Advisory write gate (mirrors the server). */
  canEdit: (doc: WireDocument, path: string) => boolean;
  logger: Logger;
}

/** An open conflict-resolution session: the grouped conflicts + a resolver the modal calls. */
export interface PendingSession {
  groups: ConflictGroup[];
  resolve: (theirsByGroup: Map<string, Set<string>>) => void;
}

export class TemplatesController {
  #deps: TemplatesControllerDeps;
  pending = $state<PendingSession | null>(null);

  constructor(deps: TemplatesControllerDeps) {
    this.#deps = deps;
  }

  #get(id: string): WireDocument | undefined {
    return this.#deps.documents.get(id);
  }

  #templateOf(child: WireDocument): WireDocument | undefined {
    return child.source ? this.#get(child.source.id) : undefined;
  }

  #isOwnerOrGm(doc: WireDocument): boolean {
    return this.#deps.role === "gm" || doc.owner === this.#deps.selfId;
  }

  stampInstance(source: WireDocument, opts: StampOpts): WireDocument {
    return stampInstance(source, opts);
  }

  findInstances(templateId: string): WireDocument[] {
    return findInstances(templateId, this.#deps.store.snapshot());
  }

  syncState(childId: string): SyncState {
    const child = this.#get(childId);
    if (!child) return "none";
    return syncState(child, this.#templateOf(child));
  }

  canPull(childId: string): boolean {
    const child = this.#get(childId);
    if (!child || !this.#templateOf(child)) return false;
    return this.#isOwnerOrGm(child) && this.#deps.canEdit(child, "/base") && this.#deps.canEdit(child, "/system");
  }

  canPush(templateId: string): boolean {
    const tmpl = this.#get(templateId);
    if (!tmpl) return false;
    return this.#isOwnerOrGm(tmpl) && this.findInstances(templateId).length > 0;
  }

  pull(childId: string): void {
    const child = this.#get(childId);
    if (!child) return;
    const template = this.#templateOf(child);
    if (!template) {
      this.#deps.logger.warn(`templates.pull: template ${child.source?.id ?? "?"} not in store; pull unavailable`);
      return;
    }
    const plan = computePull(child, template);
    if (plan.conflicts.length === 0) {
      this.#deps.dispatchIntent([planToUpdate(child, template, plan.mergedBands)]);
      return;
    }
    this.#openSession([{ key: childId, label: null, conflicts: plan.conflicts }], new Map([[childId, { child, template, plan }]]));
  }

  revert(childId: string): void {
    const child = this.#get(childId);
    if (!child) return;
    const template = this.#templateOf(child);
    if (!template) {
      this.#deps.logger.warn(`templates.revert: template ${child.source?.id ?? "?"} not in store; revert unavailable`);
      return;
    }
    this.#deps.dispatchIntent([computeRevert(child, template)]);
  }

  push(templateId: string): void {
    const template = this.#get(templateId);
    if (!template) return;
    const instances = this.findInstances(templateId);
    const groups: ConflictGroup[] = [];
    const conflicted = new Map<string, { child: WireDocument; template: WireDocument; plan: MergePlan }>();
    for (const inst of instances) {
      const plan = computePull(inst, template);
      if (plan.conflicts.length === 0) {
        this.#deps.dispatchIntent([planToUpdate(inst, template, plan.mergedBands)]);
      } else {
        groups.push({ key: inst.id, label: inst.name ?? inst.id, conflicts: plan.conflicts });
        conflicted.set(inst.id, { child: inst, template, plan });
      }
    }
    if (groups.length > 0) this.#openSession(groups, conflicted);
  }

  cancel(): void {
    this.pending = null;
  }

  #openSession(
    groups: ConflictGroup[],
    byKey: Map<string, { child: WireDocument; template: WireDocument; plan: MergePlan }>,
  ): void {
    this.pending = {
      groups,
      resolve: (theirsByGroup) => {
        for (const [key, entry] of byKey) {
          const theirs = theirsByGroup.get(key) ?? new Set<string>();
          const resolved = applyResolutions(entry.plan.mergedBands, entry.plan.conflicts, theirs);
          this.#deps.dispatchIntent([planToUpdate(entry.child, entry.template, resolved)]);
        }
        this.pending = null;
      },
    };
  }
}
