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
//! that
//! no longer match (`MergeErrorKind::StaleResolutions`/`UnknownResolution`, both
//! carrying the fresh outcome so the client re-opens its modal without a round
//! trip). Authorization lives HERE, not in the write path: `TemplateMerge` waives
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
    MergeError, MergePlan,
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
    /// `load_effective_owner`), never a literal `owner` read.
    async fn access(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        doc: &Document,
    ) -> Result<Access, DataError> {
        let owner = repo.effective_owner_of(doc).await?;
        Ok(resolve_access_world(
            ctx.user_id,
            ctx.world_role,
            doc,
            &self.defaults.grants_for(&doc.doc_type),
            owner,
        ))
    }
}

/// Whether `access` may commit `update`'s change paths: THE SAME predicate
/// `apply_intent`'s Update arm applies — each path's structural capability
/// (`required_cap_for_path`) plus the additive declared requirements
/// (`declared_caps_for_path`) — re-derived here because the commit runs under
/// `WriteOrigin::TemplateMerge`, which waives that arm's per-op gates. The
/// whole-band `/base` refresh is exempted exactly as `apply_intent`'s merge-base
/// carve-out exempts it (the server-owned write this origin exists to commit);
/// `/base/...` sub-paths and any other unmappable path refuse, fail-closed, for
/// every origin there and so for the merge here.
fn update_authorized(update: &Operation, access: &Access, inputs: &AuthInputs) -> bool {
    let Operation::Update { changes, .. } = update else {
        return false;
    };
    changes.iter().all(|ch| {
        if ch.path == "/base" {
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
/// posture).
async fn commit_error<F, Fut>(request_id: Uuid, e: DataError, fresh: F) -> ServerMsg
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
/// The template gate replicates the client flow's reach — the client only ever
/// merged against templates in the requester's own store — and it is what keeps a
/// `Conflicts` reply (whose entries carry template-side values) from disclosing a
/// template the requester could not already read. A template in ANOTHER world is
/// reported `NotFound`, the same existence-hiding the child's own world check
/// applies.
struct PullDocs {
    /// The instance being merged into / reset.
    child: Document,
    /// The instance's template.
    template: Document,
    /// The requester's resolved access on the child (the per-path derivation's input).
    child_access: Access,
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
    let child_access = inputs
        .access(repo, ctx, &child)
        .await
        .map_err(|_| MergeErrorKind::Internal)?;
    // Owner-or-GM is `Access::is_owner` (the effective-owner rule) — the uncapped
    // GM short-circuit sets it; a `gm_role`-capped GM is deliberately an ordinary
    // actor here. READ rides along so an owner who cannot even receive the
    // document cannot merge it (mirroring combat's `owns_combatant` conjunction).
    if !(child_access.is_owner && child_access.has(cap::READ)) {
        return Err(MergeErrorKind::Forbidden);
    }
    let template_access = inputs
        .access(repo, ctx, &template)
        .await
        .map_err(|_| MergeErrorKind::Internal)?;
    if !template_access.has(cap::READ) {
        return Err(MergeErrorKind::Forbidden);
    }
    Ok(PullDocs {
        child,
        template,
        child_access,
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
            Ok(apply_resolutions(
                &plan.merged_bands,
                &plan.conflicts,
                &theirs,
            ))
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
    let plan = match compute_pull(&docs.child, &docs.template) {
        Ok(p) => p,
        Err(MergeError::CorruptBase) => {
            return merge_error(request_id, MergeErrorKind::CorruptBase)
        }
    };
    let bands = match resolved_bands(&plan, resolutions) {
        Ok(b) => b,
        Err(ResolutionsRejection::Unknown) => {
            return merge_error(
                request_id,
                MergeErrorKind::UnknownResolution(pull_outcome(child_id, &plan)),
            );
        }
        Err(ResolutionsRejection::Stale) => {
            return merge_error(
                request_id,
                MergeErrorKind::StaleResolutions(pull_outcome(child_id, &plan)),
            );
        }
    };
    let update = plan_to_update(&docs.child, &docs.template, &bands);
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
    match room
        .publish(repo, ctx, vec![update], now, WriteOrigin::TemplateMerge)
        .await
    {
        Ok(_) => ServerMsg::MergeResult {
            request_id,
            outcome: MergeOutcome::Pull {
                child_id,
                status: MergePullStatus::Applied,
            },
        },
        Err(e) => {
            commit_error(request_id, e, || async {
                let inputs = AuthInputs::load(repo, room.world_id)
                    .await
                    .map_err(|_| MergeErrorKind::Internal)?;
                let docs = load_pull_docs(room, repo, ctx, &inputs, child_id).await?;
                let plan = compute_pull(&docs.child, &docs.template)
                    .map_err(|_| MergeErrorKind::CorruptBase)?;
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
    let update = compute_revert(&docs.child, &docs.template);
    if !update_authorized(&update, &docs.child_access, &inputs) {
        return merge_error(request_id, MergeErrorKind::Forbidden);
    }
    match room
        .publish(repo, ctx, vec![update], now, WriteOrigin::TemplateMerge)
        .await
    {
        Ok(_) => ServerMsg::MergeResult {
            request_id,
            outcome: MergeOutcome::Revert {
                child_id,
                status: MergeRevertStatus::Applied,
            },
        },
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

/// One instance's computed push state.
enum PushSlot {
    /// Not visible to the pusher (existence-hiding: no name disclosed).
    Excluded,
    /// Visible; the raw (child-wins) plan and the pusher's access, awaiting the
    /// per-call update computation and per-path derivation.
    Planned(Box<PlannedInstance>),
}

/// A visible instance's phase-1 state.
struct PlannedInstance {
    /// The instance document as loaded.
    doc: Document,
    /// The pusher-visible display name (the redacted view's `name`; `None` when a
    /// redaction failure withholds it, the fail-closed direction).
    name: Option<String>,
    /// The pusher's resolved access on the instance.
    access: Access,
    /// The raw computed plan: child-wins bands plus the CURRENT conflict set.
    plan: MergePlan,
}

/// Phase 1 of a push: load every same-world instance and compute its raw plan.
/// `MergeErrorKind::CorruptBase` aborts the WHOLE intent here, before anything is
/// written — a corrupt snapshot means the instance needs human attention, and a
/// partial push across its siblings would make that state harder to reason about.
async fn plan_push(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    inputs: &AuthInputs,
    template: &Document,
) -> Result<Vec<(Uuid, PushSlot)>, MergeErrorKind> {
    let instances = repo
        .instances_of(room.world_id, template.id)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "merge: instance query failed");
            MergeErrorKind::Internal
        })?;
    let mut slots = Vec::with_capacity(instances.len());
    for doc in instances {
        let access = inputs
            .access(repo, ctx, &doc)
            .await
            .map_err(|_| MergeErrorKind::Internal)?;
        if !access.has(cap::READ) {
            slots.push((doc.id, PushSlot::Excluded));
            continue;
        }
        let name = match filter_properties(&doc, &access) {
            Ok(view) => view.name,
            Err(e) => {
                tracing::warn!(doc_id = %doc.id, error = %e, "merge: name redaction failed; withholding");
                None
            }
        };
        let plan = compute_pull(&doc, template).map_err(|_| MergeErrorKind::CorruptBase)?;
        slots.push((
            doc.id,
            PushSlot::Planned(Box::new(PlannedInstance {
                doc,
                name,
                access,
                plan,
            })),
        ));
    }
    Ok(slots)
}

/// The push outcome a phase-1 slot set describes WITHOUT writing anything:
/// `Conflicts` for a conflicted instance, `Applied` for a clean one ("currently
/// conflict-free" — `MergePullStatus`'s contract covers the uncommitted reading),
/// `Excluded` for an invisible one. A VISIBLE instance whose child-wins update
/// fails the per-path derivation is `Excluded` here too, mirroring the client
/// flow, which kept such instances out of the conflict modal entirely.
fn push_outcome(
    template_id: Uuid,
    slots: &[(Uuid, PushSlot)],
    inputs: &AuthInputs,
    template: &Document,
) -> MergeOutcome {
    let instances = slots
        .iter()
        .map(|(id, slot)| {
            let (name, status) = match slot {
                PushSlot::Excluded => (None, PushInstanceStatus::Excluded),
                PushSlot::Planned(p) => {
                    let update = plan_to_update(&p.doc, template, &p.plan.merged_bands);
                    if !update_authorized(&update, &p.access, inputs) {
                        (p.name.clone(), PushInstanceStatus::Excluded)
                    } else if p.plan.conflicts.is_empty() {
                        (p.name.clone(), PushInstanceStatus::Applied)
                    } else {
                        (
                            p.name.clone(),
                            PushInstanceStatus::Conflicts(p.plan.conflicts.clone()),
                        )
                    }
                }
            };
            PushInstanceOutcome {
                instance_id: *id,
                name,
                status,
            }
        })
        .collect();
    MergeOutcome::Push {
        template_id,
        instances,
    }
}

/// Validate a resolutions MAP against the phase-1 slots: every key must name a
/// visible push target, and every path a current conflict of that instance.
fn check_push_resolutions(
    resolutions: &BTreeMap<Uuid, Vec<String>>,
    slots: &[(Uuid, PushSlot)],
) -> Result<(), ResolutionsRejection> {
    for (id, paths) in resolutions {
        let Some((_, PushSlot::Planned(p))) = slots.iter().find(|(slot_id, _)| slot_id == id)
        else {
            return Err(ResolutionsRejection::Unknown);
        };
        check_resolutions(&paths.iter().cloned().collect(), &p.plan.conflicts)?;
    }
    Ok(())
}

/// `MergePush`: push the template into every same-world instance of it — the
/// pusher must be the template's effective owner (or GM) holding READ plus
/// `/embedded` write on it; each instance must be visible to the pusher and pass
/// the per-path derivation against the actual computed Update, else it reports
/// `Excluded` (which never discloses not-visible vs not-writable).
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
    let template_access = match inputs.access(repo, ctx, &template).await {
        Ok(a) => a,
        Err(_) => return merge_error(request_id, MergeErrorKind::Internal),
    };
    // Owner-or-GM of the TEMPLATE plus `/embedded` writability on it — the one
    // capability every push can require (merged embedded collections add/remove
    // children on instances), read off the shared `required_cap_for_path`
    // predicate rather than restated.
    let embedded_cap = required_cap_for_path("/embedded").expect("/embedded maps to a capability");
    if !(template_access.is_owner
        && template_access.has(cap::READ)
        && template_access.has(embedded_cap))
    {
        return merge_error(request_id, MergeErrorKind::Forbidden);
    }
    let slots = match plan_push(room, repo, ctx, &inputs, &template).await {
        Ok(s) => s,
        Err(reason) => return merge_error(request_id, reason),
    };
    if let Some(map) = &resolutions {
        if let Err(rejection) = check_push_resolutions(map, &slots) {
            let fresh = push_outcome(template_id, &slots, &inputs, &template);
            let reason = match rejection {
                ResolutionsRejection::Unknown => MergeErrorKind::UnknownResolution(fresh),
                ResolutionsRejection::Stale => MergeErrorKind::StaleResolutions(fresh),
            };
            return merge_error(request_id, reason);
        }
    }
    // Phase 2: per instance, compute THIS call's update (child-wins on a first
    // call, resolutions folded in on a second), derive authorization against it,
    // then commit — or, on a first call with conflicts, report without writing.
    let mut outcomes: Vec<PushInstanceOutcome> = Vec::with_capacity(slots.len());
    for (id, slot) in slots {
        let PushSlot::Planned(p) = slot else {
            outcomes.push(PushInstanceOutcome {
                instance_id: id,
                name: None,
                status: PushInstanceStatus::Excluded,
            });
            continue;
        };
        let theirs = resolutions
            .as_ref()
            .and_then(|m| m.get(&id))
            .cloned()
            .unwrap_or_default();
        let bands = if first_call {
            p.plan.merged_bands.clone()
        } else {
            apply_resolutions(
                &p.plan.merged_bands,
                &p.plan.conflicts,
                &theirs.into_iter().collect(),
            )
        };
        let update = plan_to_update(&p.doc, &template, &bands);
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
                    let slots = plan_push(room, repo, ctx, &inputs, &template).await?;
                    Ok(push_outcome(template_id, &slots, &inputs, &template))
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
