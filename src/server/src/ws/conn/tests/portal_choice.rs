//! The portal offer's addressing and refusal shape at the connection layer: who `egress_loop`
//! delivers a `PortalOffer` to (`portal_offer_admitted`), and the role-conditioned wording of a
//! refusal frame (`portal_error_frame`).

use super::*;
use crate::data::document::WorldRole;
use crate::ws::room::PortalChoiceError;

/// A public offer reaches its owner and every GM; nobody else. A GM-only offer reaches GMs alone,
/// even the owner is refused. Mutation witness: dropping the `!gm_only` clause lets the owner
/// receive a secret portal's offer, failing the fourth assertion.
#[test]
fn portal_offer_admitted_addresses_the_owner_and_gms_and_nobody_else() {
    let owner = Uuid::from_u128(1);
    let other = Uuid::from_u128(2);
    assert!(portal_offer_admitted(
        false,
        Some(owner),
        owner,
        WorldRole::Player
    ));
    assert!(!portal_offer_admitted(
        false,
        Some(owner),
        other,
        WorldRole::Player
    ));
    assert!(portal_offer_admitted(
        false,
        Some(owner),
        other,
        WorldRole::Gm
    ));
    assert!(!portal_offer_admitted(
        true,
        Some(owner),
        owner,
        WorldRole::Player
    ));
    assert!(portal_offer_admitted(
        true,
        Some(owner),
        other,
        WorldRole::Gm
    ));
    assert!(
        !portal_offer_admitted(false, None, other, WorldRole::Player),
        "a token with no owner offers to GMs only"
    );
}

/// A refusal is one uniform text for a player, whichever variant caused it, and a specific text
/// for a GM; success replies nothing.
#[test]
fn portal_error_frame_is_uniform_for_a_player_and_specific_for_a_gm() {
    let request_id = Uuid::from_u128(9);
    let player = PermissionContext {
        user_id: Uuid::from_u128(1),
        world_role: WorldRole::Player,
    };
    let gm = PermissionContext {
        user_id: Uuid::from_u128(2),
        world_role: WorldRole::Gm,
    };
    assert!(portal_error_frame(request_id, Ok(()), &player).is_none());

    let text_of = |err: PortalChoiceError, ctx: &PermissionContext| match portal_error_frame(
        request_id,
        Err(err),
        ctx,
    ) {
        Some(ServerMsg::PortalChoiceError {
            request_id: id,
            message,
            reason: None,
        }) => {
            assert_eq!(id, request_id, "the refusal is correlated");
            message
        }
        other => panic!("expected a PortalChoiceError, got {other:?}"),
    };
    let player_texts = [
        text_of(PortalChoiceError::UnknownOffer, &player),
        text_of(PortalChoiceError::NotAuthorized, &player),
        text_of(PortalChoiceError::TokenMoved, &player),
        text_of(PortalChoiceError::BadChoice, &player),
        text_of(PortalChoiceError::NoPortal, &player),
        text_of(PortalChoiceError::RateLimited, &player),
    ];
    assert!(
        player_texts.iter().all(|t| t == &player_texts[0]),
        "one wording for every refusal, got {player_texts:?}"
    );
    assert_ne!(
        text_of(PortalChoiceError::NotAuthorized, &gm),
        text_of(PortalChoiceError::UnknownOffer, &gm),
        "a GM learns which refusal it was"
    );
}
