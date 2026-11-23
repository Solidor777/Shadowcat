//! Portal offers: the offer/answer exchange behind `TriggerEffect::PortalChoice`.
//!
//! A portal region's trigger fires after the token's move has committed, so the token already
//! stands in the region when the offer is made. The offer is a short-lived in-memory record
//! (`Room::portal_offers`); the answer (`Room::handle_choose_portal`) validates it and applies
//! the chosen destination through the SAME firing funnel a fixed `TriggerEffect::Teleport` uses
//! (`Room::apply_region_trigger_effects` -> `Room::apply_teleport_to`), so a chosen hop and a
//! fixed hop can never produce different ops for the same target.

use uuid::Uuid;

use super::{Room, TriggerEffectPlan};
use crate::data::document::{Document, WorldRole};
use crate::data::engine as eng;
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::ws::protocol::{PortalOfferOption, ServerMsg};

/// How long a portal offer stays answerable, in milliseconds. An offer is a moment, not data:
/// it lapses on its own when the player walks away or the connection drops.
pub(crate) const PORTAL_OFFER_TTL_MS: i64 = 120_000;

/// One pending offer: what `Room::handle_choose_portal` validates an answer against. Held per
/// token in `Room::portal_offers`; a newer offer for the token replaces it.
pub(crate) struct PortalOffer {
    /// The id the answer names (`ClientMsg::ChoosePortal::offer_id`).
    offer_id: Uuid,
    /// The region whose trigger made the offer; the token's stored membership must still name it.
    region_id: Uuid,
    /// The scene the token stood on when the offer was made.
    scene: Uuid,
    /// The offered options with their real targets; the wire carries only labels and indices.
    options: Vec<eng::PortalOption>,
    /// Commit timestamp (ms) from which the offer is dead.
    expires_ts: i64,
}

/// The portal-choice trigger an offer is built from.
pub(super) struct PortalSource<'a> {
    /// The region carrying the trigger.
    pub region_id: Uuid,
    /// Whether the region's `/engine` band is visible to every world member; `false` makes the
    /// offer GM-only.
    pub visible_to_all: bool,
    /// The authored heading.
    pub prompt: Option<&'a String>,
    /// The authored destinations.
    pub options: &'a [eng::PortalOption],
}

/// Why a `ClientMsg::ChoosePortal` or `ClientMsg::OpenPortal` was refused. `Display` yields ONE
/// wording for every variant, the wire message for a non-GM caller: a distinguishable wording
/// would tell a player whether an offer, a token or a portal exists. A GM caller receives the
/// variant-specific text (`portal_choice_wire_message`).
#[derive(Debug, thiserror::Error)]
pub(crate) enum PortalChoiceError {
    /// No live offer by that id (never made, expired, or already answered).
    #[error("portal rejected")]
    UnknownOffer,
    /// The caller is neither the token's effective owner nor a GM.
    #[error("portal rejected")]
    NotAuthorized,
    /// The token is gone, left the offer's scene, or no longer stands in the offering region.
    #[error("portal rejected")]
    TokenMoved,
    /// The chosen index names no option.
    #[error("portal rejected")]
    BadChoice,
    /// The token stands in no portal region the caller may open.
    #[error("portal rejected")]
    NoPortal,
    /// A repository read failed.
    #[error("portal rejected")]
    Data(#[from] crate::data::DataError),
}

/// The error-to-wire conversion `ws::conn` applies: the caller's role decides disclosure, never
/// the variant. A non-GM gets the uniform `Display` wording; a GM gets the specific diagnostic.
pub(crate) fn portal_choice_wire_message(err: &PortalChoiceError, world_role: WorldRole) -> String {
    if world_role != WorldRole::Gm {
        return err.to_string();
    }
    match err {
        PortalChoiceError::UnknownOffer => "no such offer, or it expired or was already answered",
        PortalChoiceError::NotAuthorized => "you do not control that token",
        PortalChoiceError::TokenMoved => "the token is no longer in the offering portal",
        PortalChoiceError::BadChoice => "no such destination",
        PortalChoiceError::NoPortal => "the token is in no portal",
        PortalChoiceError::Data(_) => "internal error",
    }
    .to_string()
}

/// Whether a trigger offers a portal choice. An `Exit` trigger is excluded: the token has left
/// the region by then, so its offer would be dead on arrival.
pub(super) fn offers_portal(trigger: &eng::RegionTrigger) -> bool {
    matches!(trigger.effect, eng::TriggerEffect::PortalChoice { .. })
        && trigger.on != eng::TriggerEvent::Exit
}

impl Room {
    /// Registers an offer for `token_doc`'s token (replacing its previous one) and broadcasts
    /// `ServerMsg::PortalOffer`. `ws::conn` delivers it only to GM connections and, for a public
    /// region, the token's effective owner.
    ///
    /// Each option's destination scene is resolved here: `available` is false for a scene that
    /// no longer exists in this world, and the real name is sent only for an option that sets
    /// `reveal_scene`. A repository failure on a lookup marks that option unavailable.
    ///
    /// PRECONDITION: caller holds `publish_guard`, which serializes every offer-table writer.
    pub(super) async fn offer_portal(
        &self,
        repo: &dyn Repository,
        token_doc: &Document,
        source: PortalSource<'_>,
        ts: i64,
    ) {
        let Some(scene) = token_doc.parent_id else {
            return;
        };
        let owner = { self.scene.read().await.token_effective_owner(token_doc) };
        let mut wire_options = Vec::with_capacity(source.options.len());
        for (index, option) in source.options.iter().enumerate() {
            let dest_doc = match repo
                .get_document(option.target.scene.unwrap_or(scene))
                .await
            {
                Ok(Some(doc))
                    if doc.doc_type == "scene"
                        && crate::data::document::world_of(&doc) == Some(self.world_id) =>
                {
                    Some(doc)
                }
                _ => None,
            };
            wire_options.push(PortalOfferOption {
                index: index as u32,
                label: option.label.clone(),
                destination_name: dest_doc
                    .as_ref()
                    .filter(|_| option.reveal_scene)
                    .and_then(|doc| doc.name.clone()),
                available: dest_doc.is_some(),
            });
        }
        let offer_id = Uuid::new_v4();
        {
            let mut offers = self.portal_offers.lock().unwrap_or_else(|e| e.into_inner());
            offers.retain(|_, o| o.expires_ts > ts);
            offers.insert(
                token_doc.id,
                PortalOffer {
                    offer_id,
                    region_id: source.region_id,
                    scene,
                    options: source.options.to_vec(),
                    expires_ts: ts.saturating_add(PORTAL_OFFER_TTL_MS),
                },
            );
        }
        self.broadcast_aux(ServerMsg::PortalOffer {
            offer_id,
            scene,
            token: token_doc.id,
            prompt: source.prompt.cloned(),
            options: wire_options,
            gm_only: !source.visible_to_all,
            owner,
        });
    }

    /// Answers an offer (`ClientMsg::ChoosePortal`): `choice` is the picked option's index, or
    /// `None` to decline. Every refusal is the same `PortalChoiceError` family, uniform on the
    /// wire for a non-GM (`portal_choice_wire_message`).
    ///
    /// Checks, in order: a live offer by `offer_id` exists; `ctx` is the token's effective owner
    /// or a GM (a refusal here leaves the offer standing, so a stranger cannot burn it); then the
    /// offer is consumed, whether the answer is accepted, declined or refused below; the token
    /// still stands on the offer's scene and its stored membership still names the offering
    /// region (anything that moved the token away kills a stale offer); the index is in range.
    /// An accepted choice applies through `apply_region_trigger_effects` as a fired
    /// `TriggerEffect::Teleport`, so it commits under `WriteOrigin::Trigger`, plays the target's
    /// VFX, posts the GM notice for a refused destination and fires the destination's `Enter`
    /// effects with teleporting disallowed, exactly as a fixed portal does.
    ///
    /// A paused session's refusal of a non-GM accept happens before this call, in
    /// `ws::session_pause::msg_refusal`.
    pub(crate) async fn handle_choose_portal(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        offer_id: Uuid,
        choice: Option<u32>,
        ts: i64,
    ) -> Result<(), PortalChoiceError> {
        let _guard = self.publish_guard.lock().await;
        let token = {
            let mut offers = self.portal_offers.lock().unwrap_or_else(|e| e.into_inner());
            offers.retain(|_, o| o.expires_ts > ts);
            offers
                .iter()
                .find(|(_, o)| o.offer_id == offer_id)
                .map(|(token, _)| *token)
                .ok_or(PortalChoiceError::UnknownOffer)?
        };
        let (token_scene, stands_in_region) = {
            let ecs = self.scene.read().await;
            let located = ecs.token_scene_and_effective_owner(token);
            let allowed = ctx.world_role == WorldRole::Gm
                || located.is_some_and(|(_, owner)| owner == Some(ctx.user_id));
            if !allowed {
                return Err(PortalChoiceError::NotAuthorized);
            }
            let membership = ecs.region_membership_of(token);
            (
                located.map(|(scene, _)| scene),
                membership.cloned().unwrap_or_default(),
            )
        };
        let offer = self
            .portal_offers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&token)
            .ok_or(PortalChoiceError::UnknownOffer)?;
        if token_scene != Some(offer.scene) || !stands_in_region.contains(&offer.region_id) {
            return Err(PortalChoiceError::TokenMoved);
        }
        let Some(index) = choice else {
            return Ok(());
        };
        let option = offer
            .options
            .get(index as usize)
            .ok_or(PortalChoiceError::BadChoice)?;
        let token_doc = repo
            .get_document(token)
            .await?
            .ok_or(PortalChoiceError::TokenMoved)?;
        let hop = eng::RegionTrigger {
            on: eng::TriggerEvent::Enter,
            effect: eng::TriggerEffect::Teleport {
                target: option.target.clone(),
            },
        };
        self.apply_region_trigger_effects(
            repo,
            ctx,
            token_doc,
            TriggerEffectPlan {
                fired: vec![(offer.region_id, hop)],
                secrecy_rows: &[],
                combat_hint: None,
                allow_teleport: true,
            },
            ts,
        )
        .await;
        Ok(())
    }

    /// Re-issues the offer for `token` (`ClientMsg::OpenPortal`): the first region in the token's
    /// stored membership carrying an offering trigger (`offers_portal`) supplies it, recomputed
    /// from the live region rows, so no extra state is stored. The same owner-or-GM rule as
    /// `handle_choose_portal` applies. A non-GM is refused as `NoPortal` for a region that is not
    /// visible to every world member: replying success with no offer would tell a player a secret
    /// portal is there.
    pub(crate) async fn handle_open_portal(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        token: Uuid,
        ts: i64,
    ) -> Result<(), PortalChoiceError> {
        let _guard = self.publish_guard.lock().await;
        let found = {
            let ecs = self.scene.read().await;
            let (_, owner) = ecs
                .token_scene_and_effective_owner(token)
                .ok_or(PortalChoiceError::NoPortal)?;
            if ctx.world_role != WorldRole::Gm && owner != Some(ctx.user_id) {
                return Err(PortalChoiceError::NotAuthorized);
            }
            ecs.region_membership_of(token)
                .into_iter()
                .flatten()
                .filter_map(|id| ecs.trigger_region_row(*id))
                .find_map(|row| {
                    row.triggers
                        .iter()
                        .find(|t| offers_portal(t))
                        .and_then(|t| match &t.effect {
                            eng::TriggerEffect::PortalChoice { prompt, options } => Some((
                                row.region_id,
                                row.visible_to_all,
                                prompt.clone(),
                                options.clone(),
                            )),
                            _ => None,
                        })
                })
        };
        let (region_id, visible_to_all, prompt, options) =
            found.ok_or(PortalChoiceError::NoPortal)?;
        if !visible_to_all && ctx.world_role != WorldRole::Gm {
            return Err(PortalChoiceError::NoPortal);
        }
        let token_doc = repo
            .get_document(token)
            .await?
            .ok_or(PortalChoiceError::NoPortal)?;
        self.offer_portal(
            repo,
            &token_doc,
            PortalSource {
                region_id,
                visible_to_all,
                prompt: prompt.as_ref(),
                options: &options,
            },
            ts,
        )
        .await;
        Ok(())
    }
}
