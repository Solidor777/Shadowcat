// Template merge orchestration. Thin glue: pure core functions → the conflict modal →
// `dispatchIntent`. Holds a reactive `pending` conflict session the `TemplateModalHost` renders.
// Constructed by the shell alongside `SheetsController`; imports no module.
import {
  computePull, computeRevert, planToUpdate, applyResolutions, findInstances, syncState, stampInstance,
  effectiveOwner,
  type WireDocument, type WireOperation, type StampOpts, type SyncState, type Logger,
  type DocumentStore, type ReadableDocuments, type MergePlan,
} from "@shadowcat/core";
import type { ConflictGroup } from "./mergeConflict";

/** The child/template/plan triple a conflict group's key resolves back to, so
 * `#openSession`'s `resolve` callback can find what to merge and dispatch once the
 * modal reports its per-group "theirs" choices. */
interface ConflictEntry {
  /** The instance document a conflict group's resolution applies to. */
  child: WireDocument;
  /** The child's template document, already resolved via `#templateOf`. */
  template: WireDocument;
  /** The precomputed merge plan (`mergedBands` + `conflicts`) from `computePull`. */
  plan: MergePlan;
}

/** The controller's collaborators, supplied once at construction. */
export interface TemplatesControllerDeps {
  /** Authoritative document mirror `findInstances` snapshots from. */
  store: DocumentStore;
  /** Optimistic document view `#get`/`#templateOf` resolve ids against. */
  documents: ReadableDocuments;
  /** Transmits the merge/stamp/revert operations the controller computes. */
  dispatchIntent: (ops: WireOperation[]) => void;
  /** The current user's world-scoped role; `"gm"` short-circuits `#isOwnerOrGm`. */
  role: "gm" | "player" | "spectator";
  /** The current user's id, compared against `effectiveOwner` in `#isOwnerOrGm`. */
  selfId: string;
  /** Advisory write gate (mirrors the server). */
  canEdit: (doc: WireDocument, path: string) => boolean;
  /** Sink for the warnings logged on an unresolvable child/template. */
  logger: Logger;
}

/** An open conflict-resolution session: the grouped conflicts + a resolver the modal calls. */
export interface PendingSession {
  /** The conflict groups to present, one per instance. */
  groups: ConflictGroup[];
  /** Applies the modal's per-group "theirs" choices and dispatches the resulting Update(s). */
  resolve: (theirsByGroup: Map<string, Set<string>>) => void;
}

/**
 * Template pull/push/revert/stamp orchestration, backing `AppContext.templates`. Thin
 * glue: pure core merge functions → the conflict modal → `dispatchIntent`. Holds a reactive
 * `pending` conflict session that `TemplateModalHost` renders. Constructed by the shell
 * alongside `SheetsController`; imports no module.
 */
export class TemplatesController {
  /** The controller's collaborators, fixed at construction. */
  #deps: TemplatesControllerDeps;
  /** The open conflict session, or `null` when no modal is pending. Reassigned (not
   * mutated in place) on open/resolve/cancel — a `$state` reassignment, so readers must
   * re-read `pending` itself rather than caching the object. */
  pending = $state<PendingSession | null>(null);

  /** Build a controller wired to its collaborators.
   * @param deps - The controller's collaborators (store/documents/dispatch/role/canEdit/logger).
   * @example new TemplatesController({ store, documents, dispatchIntent, role, selfId, canEdit, logger });
   */
  constructor(deps: TemplatesControllerDeps) {
    this.#deps = deps;
  }

  /** Look up a document by id in the optimistic view.
   * @param id - The document id to resolve.
   * @returns The document, or `undefined` if not in the store.
   * @example this.#get(childId);
   */
  #get(id: string): WireDocument | undefined {
    return this.#deps.documents.get(id);
  }

  /** Resolve `child`'s template document via its `source` reference.
   * @param child - The instance document.
   * @returns The template document, or `undefined` if `child` has no `source` or the
   * template is not currently in the store.
   * @example this.#templateOf(child);
   */
  #templateOf(child: WireDocument): WireDocument | undefined {
    return child.source ? this.#get(child.source.id) : undefined;
  }

  /** Whether the current user is a GM or the EFFECTIVE owner of `doc` (core `effectiveOwner`:
   * per-doc override, else the linked actor's owner) — the same rule the server enforces at
   * egress; a literal `doc.owner` read here forks it.
   * @param doc - The document to check ownership of.
   * @returns Whether the current user is a GM or `doc`'s effective owner.
   * @example this.#isOwnerOrGm(doc);
   */
  #isOwnerOrGm(doc: WireDocument): boolean {
    return this.#deps.role === "gm" || effectiveOwner(doc, this.#deps.documents) === this.#deps.selfId;
  }

  /** Deep-clone `source` into a new stamped instance (pure core function; the caller
   * dispatches the resulting Create).
   * @param source - The template document to stamp from.
   * @param opts - Where the new instance lands (world/owner/parent/permissions).
   * @returns The stamped instance document, not yet dispatched.
   * @example templates.stampInstance(templateDoc, { worldId, ownerId: null, parentId: null });
   */
  stampInstance(source: WireDocument, opts: StampOpts): WireDocument {
    return stampInstance(source, opts);
  }

  /** In-store instances stamped from `templateId` (same-world only; see the core
   * `findInstances` doc comment for the exact scoping rule).
   * @param templateId - The template document's id.
   * @returns Every in-store instance whose `source.id` is `templateId`.
   * @example templates.findInstances(templateId);
   */
  findInstances(templateId: string): WireDocument[] {
    return findInstances(templateId, this.#deps.store.snapshot());
  }

  /** Provenance/sync state for the sheet badge: how `childId` compares to its template.
   * @param childId - The instance document's id.
   * @returns `"none"` if `childId` is not in the store; otherwise the core `syncState`
   * result comparing the child to its resolved template (or lack thereof).
   * @example templates.syncState(childId);
   */
  syncState(childId: string): SyncState {
    const child = this.#get(childId);
    if (!child) return "none";
    return syncState(child, this.#templateOf(child));
  }

  /** Whether the current user may pull/revert `childId` (owner-or-GM + write caps).
   * @param childId - The instance document's id.
   * @returns Whether pull/revert is currently permitted.
   * @example templates.canPull(childId);
   */
  canPull(childId: string): boolean {
    const child = this.#get(childId);
    if (!child || !this.#templateOf(child)) return false;
    // Advisory client-side mirror of the server cap union: WRITE_FIELDS
    // (base/system) ∪ MANAGE_EMBEDDED. A merge plan is not computed here (expensive/premature —
    // it isn't computed until the user clicks pull), so a user missing MANAGE_EMBEDDED is
    // withheld even for a merge that happens to touch no embedded content (false negative, safe
    // direction to err in).
    return (
      this.#isOwnerOrGm(child) &&
      this.#deps.canEdit(child, "/base") &&
      this.#deps.canEdit(child, "/system") &&
      this.#deps.canEdit(child, "/embedded")
    );
  }

  /** Whether the current user may push `templateId`: owner-or-GM plus `MANAGE_EMBEDDED`
   * (`/embedded`) on the TEMPLATE doc — ONE leg of `canPull`'s union, not the same check
   * (`canPull` also requires `/base` + `/system` on the instance). `false` when the template
   * has no in-store instances to push to.
   *
   * This gate covers the TEMPLATE only. Per-instance write authorization is derived
   * separately inside `push`, via `#canApplyUpdate` against the Update each instance's merge
   * actually produces — see that method's doc comment.
   * @param templateId - The template document's id.
   * @returns Whether push is currently permitted.
   * @example templates.canPush(templateId);
   */
  canPush(templateId: string): boolean {
    const tmpl = this.#get(templateId);
    if (!tmpl) return false;
    return (
      this.#isOwnerOrGm(tmpl) &&
      this.#deps.canEdit(tmpl, "/embedded") &&
      this.findInstances(templateId).length > 0
    );
  }

  /** Merge the template into `childId`. Dispatches directly when the merge is
   * conflict-free; otherwise opens a single-group conflict session for the modal.
   * A no-op (with a logged warning) if `childId` is unresolvable or has no template.
   * @param childId - The instance document's id to pull into.
   * @example templates.pull(childId);
   */
  pull(childId: string): void {
    const child = this.#get(childId);
    if (!child) {
      this.#deps.logger.warn(`templates.pull: child ${childId} not in store; pull unavailable`);
      return;
    }
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

  /** Reset `childId`'s mergeable bands to the template (keeping placement) and dispatch
   * immediately — reverting never opens the conflict modal (the child's own changes are
   * discarded outright, so there is nothing to reconcile). A no-op (with a logged warning)
   * if `childId` is unresolvable or has no template.
   * @param childId - The instance document's id to revert.
   * @example templates.revert(childId);
   */
  revert(childId: string): void {
    const child = this.#get(childId);
    if (!child) {
      this.#deps.logger.warn(`templates.revert: child ${childId} not in store; revert unavailable`);
      return;
    }
    const template = this.#templateOf(child);
    if (!template) {
      this.#deps.logger.warn(`templates.revert: template ${child.source?.id ?? "?"} not in store; revert unavailable`);
      return;
    }
    this.#deps.dispatchIntent([computeRevert(child, template)]);
  }

  /** Whether every field path `op` writes is one `inst`'s current writer may write, derived
   * from the Update actually produced rather than a guessed list of bands — the paths
   * `planToUpdate` emits vary per instance (a whole-band write only appears when that band
   * changed), so a fixed list of capabilities to check drifts the moment `planToUpdate` starts
   * emitting a path nobody enumerated. Both `push` and `#openSession`'s `resolve` compute their
   * Update first, then call this on the result, so agreement is structural rather than a
   * pairing someone has to keep in sync by hand.
   * @param inst - The instance the Update targets.
   * @param op - The computed Update, e.g. from `planToUpdate`.
   * @returns Whether every changed path in `op` passes `canEdit`; `false` for any operation
   * kind other than `"update"`.
   * @example this.#canApplyUpdate(inst, planToUpdate(inst, template, plan.mergedBands));
   */
  #canApplyUpdate(inst: WireDocument, op: WireOperation): boolean {
    if (op.op !== "update") return false;
    return op.changes.every((change) => this.#deps.canEdit(inst, change.path));
  }

  /** Push `templateId` to every in-store instance the pusher can see + write. `findInstances`
   * is same-world only (see the core `findInstances` doc comment) and says nothing about
   * per-instance write authorization, which can differ from the template's own ownership (an
   * instance may belong to a different player) — write authorization is derived per instance via
   * `#canApplyUpdate`, computed against the actual Update that instance's merge produces, never
   * against a guessed set of bands. An instance whose provisional Update (from its
   * `computePull`-produced `mergedBands`, before any conflict resolution) already touches a path
   * the pusher cannot write is excluded before it can even enter the conflict modal; an instance
   * that clears that gate but whose FINAL resolved Update touches a path the pusher cannot write
   * (`#openSession`'s `resolve` checks this) is excluded there instead. Either way the caller
   * is warned once, listing every excluded instance, so the push's partial reach is visible
   * rather than silently stale.
   * A no-op (with a logged warning) if `templateId` is unresolvable.
   * @param templateId - The template document's id to push.
   * @example templates.push(templateId);
   */
  push(templateId: string): void {
    const template = this.#get(templateId);
    if (!template) {
      this.#deps.logger.warn(`templates.push: template ${templateId} not in store; push unavailable`);
      return;
    }
    const groups: ConflictGroup[] = [];
    const conflicted = new Map<string, ConflictEntry>();
    const excluded: string[] = [];
    for (const inst of this.findInstances(templateId)) {
      const plan = computePull(inst, template);
      const op = planToUpdate(inst, template, plan.mergedBands);
      if (!this.#canApplyUpdate(inst, op)) {
        excluded.push(inst.id);
        continue;
      }
      if (plan.conflicts.length === 0) {
        this.#deps.dispatchIntent([op]);
      } else {
        groups.push({ key: inst.id, label: inst.name ?? inst.id, conflicts: plan.conflicts });
        conflicted.set(inst.id, { child: inst, template, plan });
      }
    }
    if (excluded.length > 0) {
      this.#deps.logger.warn(
        `templates.push: excluded instance(s) not writable by the pusher: ${excluded.join(", ")}`,
      );
    }
    if (groups.length > 0) this.#openSession(groups, conflicted);
  }

  /** Dismiss the open conflict session without applying anything.
   * @example templates.cancel();
   */
  cancel(): void {
    this.pending = null;
  }

  /** Open a conflict-resolution session: publish `pending` for `TemplateModalHost` to
   * render, and wire its `resolve` to apply each group's chosen theirs-paths, derive write
   * authorization from the resulting Update via `#canApplyUpdate`, dispatch it if authorized
   * (warning once, listing every excluded instance, if not), then clear `pending`. A group
   * absent from the resolver's map (nothing chosen "theirs") resolves with an empty
   * theirs-set — everything stays "mine".
   * @param groups - The conflict groups to present, one per instance.
   * @param byKey - Each group's child/template/plan, keyed by the same key used in `groups`.
   * @example this.#openSession(groups, byKey);
   */
  #openSession(
    groups: ConflictGroup[],
    byKey: Map<string, ConflictEntry>,
  ): void {
    this.pending = {
      groups,
      resolve: (theirsByGroup) => {
        const excluded: string[] = [];
        for (const [key, entry] of byKey) {
          const theirs = theirsByGroup.get(key) ?? new Set<string>();
          const resolved = applyResolutions(entry.plan.mergedBands, entry.plan.conflicts, theirs);
          const op = planToUpdate(entry.child, entry.template, resolved);
          if (this.#canApplyUpdate(entry.child, op)) {
            this.#deps.dispatchIntent([op]);
          } else {
            excluded.push(entry.child.id);
          }
        }
        this.pending = null;
        if (excluded.length > 0) {
          this.#deps.logger.warn(
            `templates: excluded instance(s) not writable by the resolving user: ${excluded.join(", ")}`,
          );
        }
      },
    };
  }
}
