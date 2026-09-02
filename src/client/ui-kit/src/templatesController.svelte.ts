// Template merge orchestration. Thin glue: the controller sends the three merge intents
// (`MergePull`/`MergePush`/`MergeRevert`) and lets the server compute the 3-way merge — the
// conflict modal renders whatever conflict set the server reports, and a resolution re-sends the
// intent with the user's per-path "theirs" choices. Holds a reactive `pending` conflict session
// the `TemplateModalHost` renders. Constructed by the shell alongside `SheetsController`; imports
// no module.
import {
  findInstances, syncState, stampInstance, effectiveOwner, MergeIntentError,
  type WireDocument, type StampOpts, type SyncState, type Logger,
  type DocumentStore, type ReadableDocuments, type NotificationLevel,
  type ClientMsg, type WireMergeOutcome, type WireMergeConflict, type WirePushInstanceOutcome,
  type WireMergeErrorKind, type WsTimeoutOptions,
} from "@shadowcat/core";
import type { ConflictGroup } from "./mergeConflict";

/** Wait for a merge reply before the request is abandoned: the whole budget of a pull/revert
 * (one instance) and the per-request share of a push. */
export const MERGE_TIMEOUT_BASE_MS = 10_000;
/** Additional wait per instance a push may commit. The server commits push instances one by
 * one under the room's publish guard and replies only after the last, so a push over many
 * instances legitimately outlasts a single-instance merge. */
export const MERGE_TIMEOUT_PER_INSTANCE_MS = 1_000;

/** The fresh outcome a resolutions rejection carries (`stale_resolutions`/`unknown_resolution`/
 * `unresolvable` — the merge as recomputed from live documents at rejection time), or `null` for
 * a plain refusal with no payload to recover. Not exported (folded into the controller's
 * intent methods).
 * @param reason - The rejection reason from a `MergeIntentError`.
 * @returns The carried outcome, or `null`.
 * @example
 * ```
 * // internal helper; not part of the public API
 * declare const reason: WireMergeErrorKind;
 * freshOutcome(reason);
 * ```
 */
function freshOutcome(reason: WireMergeErrorKind): WireMergeOutcome | null {
  if (typeof reason === "string") return null;
  if ("stale_resolutions" in reason) return reason.stale_resolutions;
  if ("unknown_resolution" in reason) return reason.unknown_resolution;
  return reason.unresolvable;
}

/** Whether one push instance outcome carries a conflict set (as opposed to `applied`/`excluded`).
 * Not exported (folded into the controller's public surface).
 * @param inst - The per-instance outcome to classify.
 * @returns `true` iff the instance is conflicted.
 * @example
 * ```
 * // internal helper; not part of the public API
 * declare const inst: WirePushInstanceOutcome;
 * hasConflicts(inst);
 * ```
 */
function hasConflicts(inst: WirePushInstanceOutcome): boolean {
  return inst.status !== "applied" && inst.status !== "excluded";
}

/** A merge intent frame, already carrying its own `request_id`. */
type MergeIntentMsg = Extract<
  ClientMsg,
  {
    /** Merge frame discriminant literal (`merge_pull`/`merge_push`/`merge_revert`). */
    type: "merge_pull" | "merge_push" | "merge_revert";
  }
>;

/** A pending session's per-group correlation data: which merge intent to re-send (with
 * resolutions) when the modal reports its choices. */
interface ConflictEntry {
  /** The instance id this group's conflicts belong to (`child_id` for pull, the push
   * instance's own id for push). */
  instanceId: string;
}

/** The controller's collaborators, supplied once at construction. */
export interface TemplatesControllerDeps {
  /** Authoritative document mirror `findInstances` snapshots from (display only). */
  store: DocumentStore;
  /** Optimistic document view `#get`/`#templateOf`/`canPull`/`canPush` resolve ids against. */
  documents: ReadableDocuments;
  /** Sends a merge intent and resolves with the server's computed outcome (or rejects with a
   * `MergeIntentError`/`Error`). `opts.timeoutMs` is sized per request by the controller
   * (`MERGE_TIMEOUT_BASE_MS` + `MERGE_TIMEOUT_PER_INSTANCE_MS` per instance for a push). */
  sendMergeIntent: (msg: MergeIntentMsg, opts: WsTimeoutOptions) => Promise<WireMergeOutcome>;
  /** The current user's world-scoped role; `"gm"` short-circuits `#isOwnerOrGm`. */
  role: "gm" | "player" | "spectator";
  /** The current user's id, compared against `effectiveOwner` in `#isOwnerOrGm`. */
  selfId: string;
  /** Advisory write gate (mirrors the server). */
  canEdit: (doc: WireDocument, path: string) => boolean;
  /** Sink for the warnings logged on an unresolvable child/template or a rejected intent. */
  logger: Logger;
  /** UI-visible notification seam (`AppContext.notify`), called alongside `logger.warn` on a
   * rejected or partially-excluded intent with a player-presentable message. */
  notify: (message: string, level?: NotificationLevel) => void;
}

/** An open conflict-resolution session: the grouped conflicts + a resolver the modal calls. */
export interface PendingSession {
  /** The conflict groups to present, one per instance. */
  groups: ConflictGroup[];
  /** Applies the modal's per-group "theirs" choices and re-sends the merge intent with
   * `resolutions`. */
  resolve: (theirsByGroup: Map<string, Set<string>>) => void;
}

/**
 * Template pull/push/revert/stamp orchestration, backing `AppContext.templates`. The server
 * computes the 3-way merge; this controller sends the intent, opens the conflict modal on a
 * conflicted reply, and re-sends with resolutions. Holds a reactive `pending` conflict session
 * that `TemplateModalHost` renders. Constructed by the shell alongside `SheetsController`;
 * imports no module.
 */
export class TemplatesController {
  /** The controller's collaborators, fixed at construction. */
  #deps: TemplatesControllerDeps;
  /** The open conflict session, or `null` when no modal is pending. Reassigned (not
   * mutated in place) on open/resolve/cancel — a `$state` reassignment, so readers must
   * re-read `pending` itself rather than caching the object. */
  pending = $state<PendingSession | null>(null);
  /** Document ids (the child for a pull, the template for a push) with a merge intent still
   * awaiting its reply. A second `pull`/`push` on the same id in that window is dropped: it
   * would race the first call's commit and be refused by the server's recompute as stale
   * against documents the first call itself moved. */
  #inFlight = new Set<string>();

  /** Build a controller wired to its collaborators.
   * @param deps - The controller's collaborators (store/documents/sendMergeIntent/role/canEdit/
   * logger/notify).
   * @example new TemplatesController({ store, documents, sendMergeIntent, role, selfId, canEdit, logger, notify });
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
   * `findInstances` doc comment for the exact scoping rule). Display only — `push` finds its
   * own authoritative instance set server-side.
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
    // Advisory client-side mirror of the server cap union: WRITE_FIELDS (system) ∪
    // MANAGE_EMBEDDED — `/base` dropped: the server writes `/base` unconditionally under
    // `WriteOrigin::TemplateMerge` (no client capability maps to it at all), so gating this
    // advisory check on `canEdit(child, "/base")` would hide pull/revert from exactly the users
    // the server now authorizes. A merge plan is not computed here (expensive/premature — it
    // isn't computed until the user clicks pull), so a user missing MANAGE_EMBEDDED is withheld
    // even for a merge that happens to touch no embedded content (false negative, safe direction
    // to err in).
    return (
      this.#isOwnerOrGm(child) &&
      this.#deps.canEdit(child, "/system") &&
      this.#deps.canEdit(child, "/embedded")
    );
  }

  /** Whether the current user may push `templateId`: owner-or-GM plus `MANAGE_EMBEDDED`
   * (`/embedded`) on the TEMPLATE doc — ONE leg of `canPull`'s union, not the same check
   * (`canPull` also requires `/system` on the instance). `false` when the template
   * has no in-store instances to push to.
   *
   * This gate covers the TEMPLATE only. Per-instance write authorization is derived
   * server-side, per instance, against the actual computed Update — see `push`'s doc comment.
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

  /** Warn (logger + player-presentable notify) once with `message`. Not exported (folded into
   * the intent methods' public surface).
   * @param message - The player-presentable text to notify with; also logged verbatim.
   * @example this.#warn("templates.pull: rejected");
   */
  #warn(message: string): void {
    this.#deps.logger.warn(message);
    this.#deps.notify(message, "warning");
  }

  /** Send `MergePull` for `childId` and handle the outcome: conflict-free applies via the
   * ordinary broadcast Event echo (nothing further to do here); conflicted opens a single-group
   * session. A no-op (with a logged warning) if `childId` is unresolvable. A rejection
   * (`not_found`/`not_an_instance`/`forbidden`/`corrupt_base`/`internal`) warns with the
   * server's player-presentable reason.
   * @param childId - The instance document's id to pull into.
   * @example templates.pull(childId);
   */
  pull(childId: string): void {
    const child = this.#get(childId);
    if (!child) {
      this.#deps.logger.warn(`templates.pull: child ${childId} not in store; pull unavailable`);
      return;
    }
    void this.#sendPull(childId);
  }

  /** Send (or re-send with `resolutions`) `MergePull` for `childId` and route the outcome.
   * Not exported (folded into `pull`'s public surface).
   *
   * A resolutions rejection carries the merge as recomputed from live documents: a
   * conflicted fresh outcome re-opens the modal; a fresh `applied` outcome means the
   * documents moved such that nothing conflicts any more — but the rejected call wrote
   * NOTHING, so the pull is re-sent once as a compute-only call, which applies it. The
   * retry is bounded to one (`retried`): a second consecutive rejection is reported.
   * @param childId - The instance document's id.
   * @param resolutions - Second-call resolutions (conflict paths to take the template side of).
   * @param retried - Whether this call is the one bounded compute-only retry.
   * @example this.#sendPull(childId);
   */
  async #sendPull(childId: string, resolutions?: string[], retried = false): Promise<void> {
    if (this.#inFlight.has(childId)) return;
    this.#inFlight.add(childId);
    let retry = false;
    try {
      const outcome = await this.#deps.sendMergeIntent(
        { type: "merge_pull", request_id: crypto.randomUUID(), child_id: childId, resolutions },
        { timeoutMs: MERGE_TIMEOUT_BASE_MS },
      );
      if (outcome.kind !== "pull") return;
      if (outcome.status === "applied") {
        this.pending = null;
        return;
      }
      this.#openPullSession(childId, outcome.status.conflicts);
    } catch (err) {
      const fresh = err instanceof MergeIntentError ? freshOutcome(err.reason) : null;
      if (fresh?.kind === "pull" && fresh.status !== "applied") {
        this.#openPullSession(childId, fresh.status.conflicts);
      } else if (fresh?.kind === "pull" && !retried) {
        retry = true;
      } else {
        this.#warn(err instanceof Error ? err.message : "templates.pull: rejected");
      }
    } finally {
      this.#inFlight.delete(childId);
    }
    if (retry) await this.#sendPull(childId, undefined, true);
  }

  /** Open a single-group pull conflict session.
   * @param childId - The instance document's id the conflicts belong to.
   * @param conflicts - The server-reported conflict set.
   * @example this.#openPullSession(childId, conflicts);
   */
  #openPullSession(childId: string, conflicts: WireMergeConflict[]): void {
    this.#openSession(
      [{ key: childId, label: null, conflicts }],
      new Map([[childId, { instanceId: childId }]]),
      (byKey, theirsByGroup) => {
        const entry = byKey.get(childId);
        if (!entry) return;
        const resolutions = [...(theirsByGroup.get(childId) ?? new Set<string>())];
        void this.#sendPull(childId, resolutions);
      },
    );
  }

  /** Reset `childId`'s mergeable bands to the template (keeping placement) via `MergeRevert`.
   * Revert never conflicts — there is nothing to reconcile — so it always applies or is
   * rejected outright. A no-op (with a logged warning) if `childId` is unresolvable.
   * @param childId - The instance document's id to revert.
   * @example templates.revert(childId);
   */
  revert(childId: string): void {
    const child = this.#get(childId);
    if (!child) {
      this.#deps.logger.warn(`templates.revert: child ${childId} not in store; revert unavailable`);
      return;
    }
    void this.#deps
      .sendMergeIntent(
        { type: "merge_revert", request_id: crypto.randomUUID(), child_id: childId },
        { timeoutMs: MERGE_TIMEOUT_BASE_MS },
      )
      .catch((err: unknown) => {
        this.#warn(err instanceof Error ? err.message : "templates.revert: rejected");
      });
  }

  /** Push `templateId` to every same-world instance the server reports: applied instances need
   * nothing further (the ordinary broadcast Event echo confirms them); conflicted instances open
   * one group each in the same modal session; excluded instances (visible but not writable by
   * the pusher) are warned once, listing every excluded instance's id. An instance invisible to
   * the pusher is omitted from the server's outcome entirely (existence-hiding) and never
   * appears here at all. A no-op (with a logged warning) if `templateId` is unresolvable. A
   * whole-intent rejection warns with the server's player-presentable reason.
   * @param templateId - The template document's id to push.
   * @example templates.push(templateId);
   */
  push(templateId: string): void {
    const tmpl = this.#get(templateId);
    if (!tmpl) {
      this.#deps.logger.warn(`templates.push: template ${templateId} not in store; push unavailable`);
      return;
    }
    void this.#sendPush(templateId);
  }

  /** Send (or re-send with `resolutions`) `MergePush` for `templateId` and route the outcome.
   * Not exported (folded into `push`'s public surface).
   *
   * The reply timeout scales with the instances the pusher can see (the server commits them
   * one by one before replying). A resolutions rejection carries the freshly recomputed
   * outcome: one with conflicted instances re-opens the modal; one with none means nothing
   * conflicts any more — but the rejected call wrote NOTHING, so the push is re-sent once as
   * a compute-only call, which applies it. The retry is bounded to one (`retried`).
   * @param templateId - The template document's id.
   * @param resolutions - Second-call resolutions, per instance.
   * @param retried - Whether this call is the one bounded compute-only retry.
   * @example this.#sendPush(templateId);
   */
  async #sendPush(
    templateId: string,
    resolutions?: Record<string, string[]>,
    retried = false,
  ): Promise<void> {
    if (this.#inFlight.has(templateId)) return;
    this.#inFlight.add(templateId);
    let retry = false;
    try {
      const instances = this.findInstances(templateId).length;
      const outcome = await this.#deps.sendMergeIntent(
        { type: "merge_push", request_id: crypto.randomUUID(), template_id: templateId, resolutions },
        { timeoutMs: MERGE_TIMEOUT_BASE_MS + MERGE_TIMEOUT_PER_INSTANCE_MS * instances },
      );
      if (outcome.kind !== "push") return;
      this.#routePushOutcome(templateId, outcome.instances);
    } catch (err) {
      const fresh = err instanceof MergeIntentError ? freshOutcome(err.reason) : null;
      if (fresh?.kind === "push" && fresh.instances.some((i) => hasConflicts(i))) {
        this.#routePushOutcome(templateId, fresh.instances);
      } else if (fresh?.kind === "push" && !retried) {
        retry = true;
      } else {
        this.#warn(err instanceof Error ? err.message : "templates.push: rejected");
      }
    } finally {
      this.#inFlight.delete(templateId);
    }
    if (retry) await this.#sendPush(templateId, undefined, true);
  }

  /** Route one push outcome: open a conflict session for any conflicted instances, warn once
   * about excluded ones, and leave applied instances to the broadcast Event echo. Not exported
   * (folded into `push`'s public surface).
   * @param templateId - The template pushed.
   * @param instances - The per-instance outcomes to route.
   * @example this.#routePushOutcome(templateId, outcome.instances);
   */
  #routePushOutcome(
    templateId: string,
    instances: WirePushInstanceOutcome[],
  ): void {
    const groups: ConflictGroup[] = [];
    const byKey = new Map<string, ConflictEntry>();
    const excluded: string[] = [];
    for (const inst of instances) {
      if (inst.status === "applied") continue;
      if (inst.status === "excluded") {
        excluded.push(inst.instance_id);
        continue;
      }
      groups.push({ key: inst.instance_id, label: inst.name ?? inst.instance_id, conflicts: inst.status.conflicts });
      byKey.set(inst.instance_id, { instanceId: inst.instance_id });
    }
    if (excluded.length > 0) {
      this.#warn(`Push skipped ${excluded.length} instance(s) you don't have permission to edit.`);
    }
    if (groups.length === 0) {
      this.pending = null;
      return;
    }
    this.#openSession(groups, byKey, (byKeyNow, theirsByGroup) => {
      const resolutions: Record<string, string[]> = {};
      for (const [key] of byKeyNow) {
        const paths = [...(theirsByGroup.get(key) ?? new Set<string>())];
        if (paths.length > 0) resolutions[key] = paths;
      }
      void this.#sendPush(templateId, resolutions);
    });
  }

  /** Dismiss the open conflict session without applying anything.
   * @example templates.cancel();
   */
  cancel(): void {
    this.pending = null;
  }

  /** Open a conflict-resolution session: publish `pending` for `TemplateModalHost` to render,
   * wiring its `resolve` to `onResolve`. Not exported (folded into `pull`/`push`'s public
   * surface).
   * @param groups - The conflict groups to present, one per instance.
   * @param byKey - Each group's correlation data, keyed by the same key used in `groups`.
   * @param onResolve - Called with `byKey` and the modal's per-group "theirs" choices; the
   * caller re-sends the underlying merge intent with `resolutions`.
   * @example this.#openSession(groups, byKey, onResolve);
   */
  #openSession(
    groups: ConflictGroup[],
    byKey: Map<string, ConflictEntry>,
    onResolve: (byKey: Map<string, ConflictEntry>, theirsByGroup: Map<string, Set<string>>) => void,
  ): void {
    this.pending = {
      groups,
      resolve: (theirsByGroup) => {
        // Close eagerly on submit — the round trip re-opens a fresh session (with the
        // server-recomputed conflict set) if the resolution turns out stale/incomplete, or a
        // still-conflicted push instance remains conflicted.
        this.pending = null;
        onResolve(byKey, theirsByGroup);
      },
    };
  }
}
