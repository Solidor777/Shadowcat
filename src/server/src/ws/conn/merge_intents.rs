//! The wire-dispatch layer for the three template-merge intents
//! (`ClientMsg::MergePull`/`MergePush`/`MergeRevert`): loads the live documents,
//! computes the `MergePlan` through the server merge engine (`crate::merge`),
//! derives authorization against the ACTUAL computed `Operation::Update`, commits
//! through `Room::publish` under `WriteOrigin::TemplateMerge` — the one write
//! path, so broadcast, redaction and event-log behaviour match a
//! client-dispatched merge — and replies to the originator with
//! `ServerMsg::MergeResult`/`MergeError`.
//!
//! The flow is STATELESS: there is no server-side merge session. A resolutions
//! call recomputes the merge from live documents and rejects resolution paths
//! that no longer match, or that name a conflict whose template side cannot be
//! applied to the current merged shape (`MergeErrorKind::StaleResolutions`/
//! `UnknownResolution`/`Unresolvable`, each carrying the fresh outcome so the
//! client re-opens its modal without a round trip). Authorization lives HERE, not in the write path: `TemplateMerge` waives
//! `apply_intent`'s per-op capability gates, so `update_authorized` — which reads
//! the same `required_cap_for_path`/`declared_caps_for_path` predicates that arm
//! applies — is the only capability check these writes get.

use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use crate::data::command::{Operation, WriteOrigin};
use crate::data::document::{world_of, CapabilityRequirement, Document, WorldCapDefaults};
use crate::data::membership::PermissionContext;
use crate::data::permission::{
    cap, declared_caps_for_path, filter_properties, required_cap_for_path, resolve_access_world,
    Access,
};
use crate::data::repository::Repository;
use crate::data::DataError;
use crate::merge::{
    apply_resolutions, compute_pull, compute_revert, plan_to_update, MergeBands, MergeConflict,
    MergeError, MergePlan, RequesterView,
};
use crate::ws::protocol::{
    ClientMsg, MergeErrorKind, MergeOutcome, MergePullStatus, MergeRevertStatus,
    PushInstanceOutcome, PushInstanceStatus, ServerMsg,
};
use crate::ws::room::Room;

/// The per-world authority inputs the handlers' access resolutions and per-path
/// capability derivations read, loaded ONCE per intent — mirroring combat's
/// `run_intent`, which reads `world_cap_defaults` once for the same reason: an
/// unresolvable authority input fails closed (`MergeErrorKind::Internal`), never
/// guessed at.
struct AuthInputs {
    /// The world's default per-document capability grants.
    defaults: WorldCapDefaults,
    /// The world's declarative capability requirements (additive over the
    /// structural base capability of each change path).
    reqs: Vec<CapabilityRequirement>,
}

impl AuthInputs {
    /// Load both authority inputs for `world_id`.
    async fn load(repo: &dyn Repository, world_id: Uuid) -> Result<AuthInputs, DataError> {
        Ok(AuthInputs {
            defaults: repo.world_cap_defaults(world_id).await?,
            reqs: repo.world_cap_requirements(world_id).await?,
        })
    }

    /// `resolve_access_world` for `doc` under this world's default grants, with the
    /// effective owner joined LIVE (`Repository::effective_owner_of` — the same
    /// linked-actor join `apply_intent`'s Update arm performs through
    /// `load_effective_owner`), never a literal `owner` read — returned beside
    /// the access, since the instance/template owner relation
    /// (`merge::bands::relate_tier`'s `same_owner`) compares the same value.
    async fn access_and_owner(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        doc: &Document,
    ) -> Result<(Access, Option<Uuid>), DataError> {
        let owner = repo.effective_owner_of(doc).await?;
        Ok((
            resolve_access_world(
                ctx.user_id,
                ctx.world_role,
                doc,
                &self.defaults.grants_for(&doc.doc_type),
                owner,
            ),
            owner,
        ))
    }
}

/// Whether `access` may commit `update`'s change paths: THE SAME predicate
/// `apply_intent`'s Update arm applies — each path's structural capability
/// (`required_cap_for_path`) plus the additive declared requirements
/// (`declared_caps_for_path`) — re-derived here because the commit runs under
/// `WriteOrigin::TemplateMerge`, which waives that arm's per-op gates. Two
/// server-authored writes are exempted: the whole-band `/base` refresh, exactly
/// as `apply_intent`'s merge-base carve-out exempts it (the server-owned write
/// this origin exists to commit), and the `/permissions/property_overrides`
/// propagation of the template's policy (`merge::bands::propagate_overrides`,
/// additive by construction — it can only narrow an audience, never widen one,
/// so the requester's own `cap::EDIT_PERMISSIONS` standing is immaterial).
/// `/base/...` sub-paths and any other unmappable path refuse, fail-closed, for
/// every origin there and so for the merge here.
fn update_authorized(update: &Operation, access: &Access, inputs: &AuthInputs) -> bool {
    let Operation::Update { changes, .. } = update else {
        return false;
    };
    changes.iter().all(|ch| {
        if ch.path == "/base" || ch.path == "/permissions/property_overrides" {
            return true;
        }
        let Some(need) = required_cap_for_path(&ch.path) else {
            return false;
        };
        access.has(need)
            && declared_caps_for_path(&ch.path, &inputs.reqs)
                .iter()
                .all(|c| access.has(c))
    })
}

/// Whether `update` carries no changes at all — an instance already in sync
/// with its template (`plan_to_update` emits only what differs, the `/base`
/// refresh included). Such an update is reported applied without a publish:
/// there is nothing to write, and a no-op `Event` per clean instance per
/// resolution round is pure broadcast noise.
fn writes_nothing(update: &Operation) -> bool {
    matches!(update, Operation::Update { changes, .. } if changes.is_empty())
}

/// Whether `path` lies on the mergeable surface — the only paths a merge conflict
/// can ever name (`merge::plan::merge3` diffs the `name`/`engine`/`system`
/// synthetic tree and the `embedded` collections, nothing else). A resolutions
/// path OFF the surface was never a conflict (`MergeErrorKind::UnknownResolution`);
/// one ON the surface but absent from the current conflict set was plausibly
/// resolved away by an interleaving edit (`MergeErrorKind::StaleResolutions`).
fn on_merge_surface(path: &str) -> bool {
    ["/name", "/engine", "/system", "/embedded"]
        .iter()
        .any(|band| path == *band || path.starts_with(&format!("{band}/")))
}

/// Map a merge-engine refusal to the wire vocabulary: a corrupt snapshot is
/// its own kind; an unanswerable visibility question and the engine's own
/// (by-construction unreachable) pointer refusal both fail closed to
/// `Internal` (nothing about the documents' conflicts may be disclosed).
fn engine_error(child_id: Uuid, e: MergeError) -> MergeErrorKind {
    match e {
        MergeError::CorruptBase => MergeErrorKind::CorruptBase,
        MergeError::VisibilityUnknown | MergeError::Pointer(_) => {
            tracing::warn!(doc_id = %child_id, error = %e, "merge: engine refused; failing closed");
            MergeErrorKind::Internal
        }
    }
}

/// The template AS THE REQUESTER SEES IT — `filter_properties` under the
/// requester's access, the same view egress delivers — which is the parent
/// side of every merge this requester runs (pull, revert, push). The `/base`
/// refresh snapshots the FULL template instead (`plan_to_update`): the stored
/// snapshot is one canonical value, and each recipient's cut of it is made at
/// egress by the policy it records. Fails closed (`Internal`) when the view
/// cannot be computed. Together with the `RequesterView` oracle (which reduces
/// the stored base by the same template-side hidden set and EXCLUDES the
/// template-hidden paths from the parent diff, so a redaction-induced delete or
/// null never reads as a template change) this is what makes a merge never
/// move data the requester cannot see: not into the instance's content, not
/// onto the wire — and the propagated policy (`propagate_overrides`) keeps
/// what a seeing requester moves hidden from the instance's own readers.
fn visible_template(template: &Document, access: &Access) -> Result<Document, MergeErrorKind> {
    filter_properties(template, access).map_err(|e| {
        tracing::warn!(doc_id = %template.id, error = %e, "merge: template view unresolvable; failing closed");
        MergeErrorKind::Internal
    })
}

/// `compute_pull` under THIS requester's view: the merged bands and the
/// conflict set as the requester may observe them, `template` being the
/// requester-visible template (`visible_template`). The `RequesterView` oracle
/// hands the engine the same per-document hidden set egress strips by, and the
/// engine applies it by document identity at every embedded depth — a
/// template-hidden path is excluded from the parent diff, a conflict on a
/// child-hidden path is withheld from the set and left at its child-wins
/// default. The one construction both `pull` (and its fresh-outcome recompute)
/// and `push`'s per-instance planning use, so the report path and the
/// resolutions path can never disagree on what the current conflict set is. A
/// GM sees everything, so a GM's merge is unchanged.
fn visible_pull_plan(
    child: &Document,
    child_access: &Access,
    template: &Document,
    template_access: &Access,
) -> Result<MergePlan, MergeErrorKind> {
    let vis = RequesterView {
        template: template_access,
        child: child_access,
    };
    compute_pull(child, template, &vis).map_err(|e| engine_error(child.id, e))
}

/// The two ways a submitted resolutions set can fail against a recomputed plan;
/// both reply with the fresh outcome attached, so this marker carries only the
/// distinction (`MergeErrorKind`'s diagnostic half).
enum ResolutionsRejection {
    /// A submitted path can never name a merge conflict (off the merge surface, or
    /// — push — an instance id that is not a visible push target).
    Unknown,
    /// A submitted path is on-surface but not a CURRENT conflict — the documents
    /// moved between the conflict report and this call.
    Stale,
    /// Every submitted path is a current conflict, but taking the template's
    /// side at one of them cannot be applied to the current merged shape
    /// (`merge::apply_resolutions`'s refusal).
    Unresolvable,
}

impl ResolutionsRejection {
    /// The wire kind for this rejection, carrying `fresh` — the outcome as
    /// recomputed from live documents, so the client re-opens its modal.
    fn into_kind(self, fresh: MergeOutcome) -> MergeErrorKind {
        match self {
            ResolutionsRejection::Unknown => MergeErrorKind::UnknownResolution(fresh),
            ResolutionsRejection::Stale => MergeErrorKind::StaleResolutions(fresh),
            ResolutionsRejection::Unresolvable => MergeErrorKind::Unresolvable(fresh),
        }
    }
}

/// Check `theirs` against the CURRENT conflict set: every submitted path must be
/// a current conflict path.
fn check_resolutions(
    theirs: &BTreeSet<String>,
    conflicts: &[MergeConflict],
) -> Result<(), ResolutionsRejection> {
    for path in theirs {
        if !on_merge_surface(path) {
            return Err(ResolutionsRejection::Unknown);
        }
        if !conflicts.iter().any(|c| &c.path == path) {
            return Err(ResolutionsRejection::Stale);
        }
    }
    Ok(())
}

/// Render a rejection reply frame.
fn merge_error(request_id: Uuid, reason: MergeErrorKind) -> ServerMsg {
    ServerMsg::MergeError { request_id, reason }
}

/// Map a write-path failure to the error vocabulary: an OCC pre-image mismatch
/// means the documents moved between plan computation and commit, so the caller
/// gets the merge recomputed from live documents via `fresh` under
/// `StaleResolutions` — the same rejection an interleaving edit between the two
/// calls produces. `Forbidden` passes through; anything else is logged and
/// collapsed to `Internal` (details never echoed, the `WsErrorCode::Internal`
/// posture). `pub(crate)` so the mapping is unit-testable without a live room.
pub(crate) async fn commit_error<F, Fut>(request_id: Uuid, e: DataError, fresh: F) -> ServerMsg
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<MergeOutcome, MergeErrorKind>>,
{
    match e {
        DataError::Conflict(_) => match fresh().await {
            Ok(outcome) => merge_error(request_id, MergeErrorKind::StaleResolutions(outcome)),
            Err(reason) => merge_error(request_id, reason),
        },
        DataError::Forbidden => merge_error(request_id, MergeErrorKind::Forbidden),
        other => {
            tracing::warn!(error = %other, "merge commit failed");
            merge_error(request_id, MergeErrorKind::Internal)
        }
    }
}

/// Dispatch one merge intent frame to its handler. `None` is unreachable in
/// production — the dispatch match routes only the three merge variants here —
/// mirroring `combat::handle_combat_intent`'s wildcard arm.
pub async fn handle_merge_intent(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    msg: ClientMsg,
    now: i64,
) -> Option<ServerMsg> {
    match msg {
        ClientMsg::MergePull {
            request_id,
            child_id,
            resolutions,
        } => Some(pull(room, repo, ctx, request_id, child_id, resolutions, now).await),
        ClientMsg::MergePush {
            request_id,
            template_id,
            resolutions,
        } => Some(push(room, repo, ctx, request_id, template_id, resolutions, now).await),
        ClientMsg::MergeRevert {
            request_id,
            child_id,
        } => Some(revert(room, repo, ctx, request_id, child_id, now).await),
        _ => None,
    }
}

/// The documents a pull/revert operates on, loaded and gated: the child must be a
/// stamped instance OF THIS WORLD and the requester its effective owner (or GM)
/// holding whole-document READ; the template must be readable by the requester.
/// The template gate bounds the merge to templates the requester could already
/// receive, and it is what keeps a `Conflicts` reply (whose entries carry
/// template-side values) from disclosing a template the requester cannot read. A
/// template the requester cannot READ, like one in ANOTHER world or one that does
/// not exist, is reported `NotFound` — existence-hiding, the same posture `push`
/// takes for a missing template: a distinguishable `Forbidden` would confirm that
/// the `source` id names a real document.
struct PullDocs {
    /// The instance being merged into / reset (unredacted: the requester is
    /// its owner or a GM, and the instance's own hidden fields must survive the
    /// whole-band write).
    child: Document,
    /// The instance's template AS THE REQUESTER SEES IT (`visible_template`) —
    /// the parent side of the merge.
    template: Document,
    /// The instance's template in FULL — the `/base` snapshot source and the
    /// policy `plan_to_update` propagates onto the instance.
    template_full: Document,
    /// The requester's resolved access on the child (the per-path derivation's input).
    child_access: Access,
    /// The requester's resolved access on the template — the template side of
    /// the `RequesterView` oracle, which excludes the template-hidden paths from
    /// the parent diff at every embedded depth.
    template_access: Access,
    /// Whether the instance and the template share an effective owner — the
    /// relation the propagated tiers and the stored snapshot's recorded policy
    /// are expressed under (`merge::bands::relate_tier`).
    same_owner: bool,
}

/// Load and gate the pull/revert document pair.
async fn load_pull_docs(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    inputs: &AuthInputs,
    child_id: Uuid,
) -> Result<PullDocs, MergeErrorKind> {
    let child = match repo.get_document(child_id).await {
        Ok(Some(doc)) => doc,
        Ok(None) => return Err(MergeErrorKind::NotFound),
        Err(e) => {
            tracing::warn!(error = %e, "merge: child load failed");
            return Err(MergeErrorKind::Internal);
        }
    };
    if world_of(&child) != Some(room.world_id) {
        return Err(MergeErrorKind::NotFound);
    }
    let Some(source) = &child.source else {
        return Err(MergeErrorKind::NotAnInstance);
    };
    let template = match repo.get_document(source.id).await {
        Ok(Some(doc)) => doc,
        Ok(None) => return Err(MergeErrorKind::NotFound),
        Err(e) => {
            tracing::warn!(error = %e, "merge: template load failed");
            return Err(MergeErrorKind::Internal);
        }
    };
    if world_of(&template).is_some_and(|w| w != room.world_id) {
        return Err(MergeErrorKind::NotFound);
    }
    let (child_access, child_owner) = inputs
        .access_and_owner(repo, ctx, &child)
        .await
        .map_err(|_| MergeErrorKind::Internal)?;
    // Owner-or-GM is `Access::is_owner` (the effective-owner rule) — the uncapped
    // GM short-circuit sets it; a `gm_role`-capped GM is deliberately an ordinary
    // actor here. READ rides along so an owner who cannot even receive the
    // document cannot merge it (mirroring combat's `owns_combatant` conjunction).
    if !(child_access.is_owner && child_access.has(cap::READ)) {
        return Err(MergeErrorKind::Forbidden);
    }
    let (template_access, template_owner) = inputs
        .access_and_owner(repo, ctx, &template)
        .await
        .map_err(|_| MergeErrorKind::Internal)?;
    if !template_access.has(cap::READ) {
        return Err(MergeErrorKind::NotFound);
    }
    let template_view = visible_template(&template, &template_access)?;
    Ok(PullDocs {
        child,
        template: template_view,
        template_full: template,
        child_access,
        template_access,
        same_owner: child_owner == template_owner,
    })
}

/// The pull outcome a plan describes WITHOUT writing anything: `Conflicts` when
/// conflicts remain, else `Applied` — which, on a frame that committed nothing
/// (a first call's report or an error's fresh outcome), reads as "the merge is
/// currently conflict-free" per `MergePullStatus`'s contract.
fn pull_outcome(child_id: Uuid, plan: &MergePlan) -> MergeOutcome {
    MergeOutcome::Pull {
        child_id,
        status: if plan.conflicts.is_empty() {
            MergePullStatus::Applied
        } else {
            MergePullStatus::Conflicts(plan.conflicts.clone())
        },
    }
}

/// The resolutions half of the pull/reject flow shared by `pull` and — keyed per
/// instance — `push`: validate the submitted paths, then fold them into the
/// merged bands. `None` resolutions (a first call) is the child-wins plan
/// unchanged.
fn resolved_bands(
    plan: &MergePlan,
    resolutions: Option<Vec<String>>,
) -> Result<MergeBands, ResolutionsRejection> {
    match resolutions {
        None => Ok(plan.merged_bands.clone()),
        Some(paths) => {
            let theirs: BTreeSet<String> = paths.into_iter().collect();
            check_resolutions(&theirs, &plan.conflicts)?;
            apply_resolutions(&plan.merged_bands, &plan.conflicts, &theirs)
                .map_err(|_| ResolutionsRejection::Unresolvable)
        }
    }
}

/// `MergePull`: compute the 3-way merge of the child's template into it; commit
/// when conflict-free (or fully resolved), report the conflict set otherwise.
async fn pull(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    request_id: Uuid,
    child_id: Uuid,
    resolutions: Option<Vec<String>>,
    now: i64,
) -> ServerMsg {
    let first_call = resolutions.is_none();
    let inputs = match AuthInputs::load(repo, room.world_id).await {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!(error = %e, "merge: authority inputs unloadable");
            return merge_error(request_id, MergeErrorKind::Internal);
        }
    };
    let docs = match load_pull_docs(room, repo, ctx, &inputs, child_id).await {
        Ok(d) => d,
        Err(reason) => return merge_error(request_id, reason),
    };
    let plan = match visible_pull_plan(
        &docs.child,
        &docs.child_access,
        &docs.template,
        &docs.template_access,
    ) {
        Ok(p) => p,
        Err(reason) => return merge_error(request_id, reason),
    };
    let bands = match resolved_bands(&plan, resolutions) {
        Ok(b) => b,
        Err(rejection) => {
            return merge_error(
                request_id,
                rejection.into_kind(pull_outcome(child_id, &plan)),
            );
        }
    };
    let update = plan_to_update(&docs.child, &docs.template_full, &bands, docs.same_owner);
    if !update_authorized(&update, &docs.child_access, &inputs) {
        return merge_error(request_id, MergeErrorKind::Forbidden);
    }
    // A conflicted first call writes nothing: the conflict set is the reply.
    if first_call && !plan.conflicts.is_empty() {
        return ServerMsg::MergeResult {
            request_id,
            outcome: pull_outcome(child_id, &plan),
        };
    }
    let applied = ServerMsg::MergeResult {
        request_id,
        outcome: MergeOutcome::Pull {
            child_id,
            status: MergePullStatus::Applied,
        },
    };
    if writes_nothing(&update) {
        return applied;
    }
    match room
        .publish(repo, ctx, vec![update], now, WriteOrigin::TemplateMerge)
        .await
    {
        Ok(_) => applied,
        Err(e) => {
            commit_error(request_id, e, || async {
                let inputs = AuthInputs::load(repo, room.world_id)
                    .await
                    .map_err(|_| MergeErrorKind::Internal)?;
                let docs = load_pull_docs(room, repo, ctx, &inputs, child_id).await?;
                let plan = visible_pull_plan(
                    &docs.child,
                    &docs.child_access,
                    &docs.template,
                    &docs.template_access,
                )?;
                Ok(pull_outcome(child_id, &plan))
            })
            .await
        }
    }
}

/// `MergeRevert`: reset the child's mergeable bands to the template (placement
/// kept). Never conflicts, so it always commits and answers `Revert`.
async fn revert(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    request_id: Uuid,
    child_id: Uuid,
    now: i64,
) -> ServerMsg {
    let inputs = match AuthInputs::load(repo, room.world_id).await {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!(error = %e, "merge: authority inputs unloadable");
            return merge_error(request_id, MergeErrorKind::Internal);
        }
    };
    let docs = match load_pull_docs(room, repo, ctx, &inputs, child_id).await {
        Ok(d) => d,
        Err(reason) => return merge_error(request_id, reason),
    };
    let vis = RequesterView {
        template: &docs.template_access,
        child: &docs.child_access,
    };
    let update = match compute_revert(&docs.child, &docs.template, &vis) {
        Ok(bands) => plan_to_update(&docs.child, &docs.template_full, &bands, docs.same_owner),
        Err(e) => return merge_error(request_id, engine_error(child_id, e)),
    };
    if !update_authorized(&update, &docs.child_access, &inputs) {
        return merge_error(request_id, MergeErrorKind::Forbidden);
    }
    let applied = ServerMsg::MergeResult {
        request_id,
        outcome: MergeOutcome::Revert {
            child_id,
            status: MergeRevertStatus::Applied,
        },
    };
    if writes_nothing(&update) {
        return applied;
    }
    match room
        .publish(repo, ctx, vec![update], now, WriteOrigin::TemplateMerge)
        .await
    {
        Ok(_) => applied,
        Err(e) => {
            commit_error(request_id, e, || async {
                Ok(MergeOutcome::Revert {
                    child_id,
                    status: MergeRevertStatus::Applied,
                })
            })
            .await
        }
    }
}

/// A visible instance as PLANNED — loaded, access-resolved and merged in
/// memory, nothing committed yet.
struct PlannedInstance {
    /// The instance document as loaded.
    doc: Document,
    /// The pusher-visible display name (the redacted view's `name`; `None` when a
    /// redaction failure withholds it, the fail-closed direction).
    name: Option<String>,
    /// The pusher's resolved access on the instance.
    access: Access,
    /// The computed plan: child-wins bands plus the CURRENT conflict set,
    /// already filtered to the conflicts this pusher may see
    /// (`visible_pull_plan` against BOTH the instance and the template).
    plan: MergePlan,
    /// Whether this instance and the template share an effective owner
    /// (`merge::bands::relate_tier`'s relation).
    same_owner: bool,
}

/// Plan every same-world instance of a push before any commit: load each,
/// resolve the pusher's access, compute its plan. Planning the whole set
/// first is what lets a resolutions rejection precede every write.
/// An instance the pusher cannot READ is OMITTED from the outcome entirely —
/// true existence-hiding parity with redaction (the pusher's store never
/// contained it), so the reply carries no entry, name, or count for it.
/// `MergeErrorKind::CorruptBase` aborts the WHOLE intent here, before anything is
/// written — a corrupt snapshot means the instance needs human attention, and a
/// partial push across its siblings would make that state harder to reason about.
async fn plan_push(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    inputs: &AuthInputs,
    template: &Document,
    template_access: &Access,
    template_owner: Option<Uuid>,
) -> Result<Vec<(Uuid, PlannedInstance)>, MergeErrorKind> {
    let instances = repo
        .instances_of(room.world_id, template.id)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "merge: instance query failed");
            MergeErrorKind::Internal
        })?;
    let mut planned = Vec::with_capacity(instances.len());
    for doc in instances {
        let (access, owner) = inputs
            .access_and_owner(repo, ctx, &doc)
            .await
            .map_err(|_| MergeErrorKind::Internal)?;
        if !access.has(cap::READ) {
            continue;
        }
        let name = match filter_properties(&doc, &access) {
            Ok(view) => view.name,
            Err(e) => {
                tracing::warn!(doc_id = %doc.id, error = %e, "merge: name redaction failed; withholding");
                None
            }
        };
        let plan = visible_pull_plan(&doc, &access, template, template_access)?;
        planned.push((
            doc.id,
            PlannedInstance {
                doc,
                name,
                access,
                plan,
                same_owner: owner == template_owner,
            },
        ));
    }
    Ok(planned)
}

/// The push outcome a planned instance set describes WITHOUT writing anything:
/// `Conflicts` for a conflicted instance, `Applied` for a clean one ("currently
/// conflict-free" — `MergePullStatus`'s contract covers the uncommitted reading).
/// A VISIBLE instance whose child-wins update fails the per-path derivation is
/// `Excluded`, mirroring the client flow, which kept such instances out of the
/// conflict modal entirely. `template` is the FULL template (the `/base`
/// source), not the pusher's view.
fn push_outcome(
    template_id: Uuid,
    instances: &[(Uuid, PlannedInstance)],
    inputs: &AuthInputs,
    template: &Document,
) -> MergeOutcome {
    let instances = instances
        .iter()
        .map(|(id, p)| {
            let update = plan_to_update(&p.doc, template, &p.plan.merged_bands, p.same_owner);
            let status = if !update_authorized(&update, &p.access, inputs) {
                PushInstanceStatus::Excluded
            } else if p.plan.conflicts.is_empty() {
                PushInstanceStatus::Applied
            } else {
                PushInstanceStatus::Conflicts(p.plan.conflicts.clone())
            };
            PushInstanceOutcome {
                instance_id: *id,
                name: p.name.clone(),
                status,
            }
        })
        .collect();
    MergeOutcome::Push {
        template_id,
        instances,
    }
}

/// Validate a resolutions MAP against the planned set: every key must name a
/// visible push target, and every path a current conflict of that instance.
fn check_push_resolutions(
    resolutions: &BTreeMap<Uuid, Vec<String>>,
    instances: &[(Uuid, PlannedInstance)],
) -> Result<(), ResolutionsRejection> {
    for (id, paths) in resolutions {
        let Some((_, p)) = instances.iter().find(|(slot_id, _)| slot_id == id) else {
            return Err(ResolutionsRejection::Unknown);
        };
        check_resolutions(&paths.iter().cloned().collect(), &p.plan.conflicts)?;
    }
    Ok(())
}

/// `MergePush`: push the template into every same-world instance of it — the
/// pusher must be the template's effective owner (or GM) holding READ plus
/// `/embedded` write on it; each instance must be VISIBLE to the pusher (an
/// invisible one is omitted from the outcome entirely — existence-hiding
/// parity with redaction, not an `Excluded` entry) and pass the per-path
/// derivation against the actual computed Update, else it reports `Excluded`
/// (visible but not writable — the one thing `Excluded` means).
///
/// Instances commit ONE BY ONE through `Room::publish`, not atomically: every
/// resolution is validated and folded (`resolved`) before the first commit, so
/// a resolutions rejection precedes any write, while a commit failure mid-loop
/// (`commit_error`) leaves the instances before it committed and broadcast.
/// The reply for that case carries no ledger of its own — the fresh outcome is
/// recomputed from live documents, in which an already-committed instance
/// reads as `Applied` — and a re-sent intent commits the remainder
/// (`MergeErrorKind`'s push commit contract).
async fn push(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    request_id: Uuid,
    template_id: Uuid,
    resolutions: Option<BTreeMap<Uuid, Vec<String>>>,
    now: i64,
) -> ServerMsg {
    let first_call = resolutions.is_none();
    let inputs = match AuthInputs::load(repo, room.world_id).await {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!(error = %e, "merge: authority inputs unloadable");
            return merge_error(request_id, MergeErrorKind::Internal);
        }
    };
    let template = match repo.get_document(template_id).await {
        Ok(Some(doc)) => doc,
        Ok(None) => return merge_error(request_id, MergeErrorKind::NotFound),
        Err(e) => {
            tracing::warn!(error = %e, "merge: template load failed");
            return merge_error(request_id, MergeErrorKind::Internal);
        }
    };
    // Push is same-world by design (compendium/cross-world push is out of scope);
    // a template from anywhere else is reported exactly like a missing one.
    if world_of(&template) != Some(room.world_id) {
        return merge_error(request_id, MergeErrorKind::NotFound);
    }
    let (template_access, template_owner) =
        match inputs.access_and_owner(repo, ctx, &template).await {
            Ok(a) => a,
            Err(_) => return merge_error(request_id, MergeErrorKind::Internal),
        };
    // Owner-or-GM of the TEMPLATE plus `/embedded` writability on it — the one
    // capability every push can require (merged embedded collections add/remove
    // children on instances), read off the shared `required_cap_for_path`
    // predicate rather than restated. The mapping is a constant structural
    // invariant, but a request path fails closed (`Internal`), never panics.
    let Some(embedded_cap) = required_cap_for_path("/embedded") else {
        tracing::warn!("merge: `/embedded` maps to no capability");
        return merge_error(request_id, MergeErrorKind::Internal);
    };
    if !(template_access.is_owner
        && template_access.has(cap::READ)
        && template_access.has(embedded_cap))
    {
        return merge_error(request_id, MergeErrorKind::Forbidden);
    }
    // The PUSHER's view of the template is the parent side of every instance's
    // merge; the full template is what every instance's `/base` refresh
    // snapshots and whose policy `plan_to_update` propagates.
    let template_view = match visible_template(&template, &template_access) {
        Ok(t) => t,
        Err(reason) => return merge_error(request_id, reason),
    };
    let instances = match plan_push(
        room,
        repo,
        ctx,
        &inputs,
        &template_view,
        &template_access,
        template_owner,
    )
    .await
    {
        Ok(s) => s,
        Err(reason) => return merge_error(request_id, reason),
    };
    if let Some(map) = &resolutions {
        if let Err(rejection) = check_push_resolutions(map, &instances) {
            let fresh = push_outcome(template_id, &instances, &inputs, &template);
            return merge_error(request_id, rejection.into_kind(fresh));
        }
    }
    // Fold THIS call's resolutions into every instance's bands (child-wins on a
    // first call) BEFORE any commit, so a resolution the current merged shape
    // cannot take rejects the whole call with nothing written — the same
    // all-or-nothing posture as the resolutions check above.
    let mut resolved: Vec<(Uuid, &PlannedInstance, MergeBands)> =
        Vec::with_capacity(instances.len());
    for (id, p) in &instances {
        let theirs = resolutions
            .as_ref()
            .and_then(|m| m.get(id))
            .cloned()
            .unwrap_or_default();
        let bands = if first_call {
            p.plan.merged_bands.clone()
        } else {
            match apply_resolutions(
                &p.plan.merged_bands,
                &p.plan.conflicts,
                &theirs.into_iter().collect(),
            ) {
                Ok(b) => b,
                Err(_) => {
                    let fresh = push_outcome(template_id, &instances, &inputs, &template);
                    return merge_error(
                        request_id,
                        ResolutionsRejection::Unresolvable.into_kind(fresh),
                    );
                }
            }
        };
        resolved.push((*id, p, bands));
    }
    // Then, per instance, derive authorization against the actual update and
    // commit — or, on a first call with conflicts, report without writing.
    let mut outcomes: Vec<PushInstanceOutcome> = Vec::with_capacity(resolved.len());
    for (id, p, bands) in resolved {
        let update = plan_to_update(&p.doc, &template, &bands, p.same_owner);
        if !update_authorized(&update, &p.access, &inputs) {
            outcomes.push(PushInstanceOutcome {
                instance_id: id,
                name: p.name.clone(),
                status: PushInstanceStatus::Excluded,
            });
            continue;
        }
        if first_call && !p.plan.conflicts.is_empty() {
            outcomes.push(PushInstanceOutcome {
                instance_id: id,
                name: p.name.clone(),
                status: PushInstanceStatus::Conflicts(p.plan.conflicts.clone()),
            });
            continue;
        }
        if writes_nothing(&update) {
            outcomes.push(PushInstanceOutcome {
                instance_id: id,
                name: p.name.clone(),
                status: PushInstanceStatus::Applied,
            });
            continue;
        }
        match room
            .publish(repo, ctx, vec![update], now, WriteOrigin::TemplateMerge)
            .await
        {
            Ok(_) => outcomes.push(PushInstanceOutcome {
                instance_id: id,
                name: p.name.clone(),
                status: PushInstanceStatus::Applied,
            }),
            Err(e) => {
                return commit_error(request_id, e, || async {
                    let inputs = AuthInputs::load(repo, room.world_id)
                        .await
                        .map_err(|_| MergeErrorKind::Internal)?;
                    let instances = plan_push(
                        room,
                        repo,
                        ctx,
                        &inputs,
                        &template_view,
                        &template_access,
                        template_owner,
                    )
                    .await?;
                    Ok(push_outcome(template_id, &instances, &inputs, &template))
                })
                .await;
            }
        }
    }
    ServerMsg::MergeResult {
        request_id,
        outcome: MergeOutcome::Push {
            template_id,
            instances: outcomes,
        },
    }
}
