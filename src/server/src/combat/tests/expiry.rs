use super::*;

/// One chat notice a command created: its audience kind and its text.
struct Notice {
    /// The `Audience` tag (`public`, `gm_only`).
    audience: String,
    /// The notice's single text segment.
    text: String,
}

/// Every `message` a command's ops create, in op order.
fn notices(ops: &[Operation]) -> Vec<Notice> {
    ops.iter()
        .filter_map(|o| match o {
            Operation::Create { doc } if doc.doc_type == "message" => {
                let engine = doc.engine.as_ref().unwrap();
                Some(Notice {
                    audience: engine["audience"]["kind"].as_str().unwrap().to_string(),
                    text: engine["content"][0]["text"].as_str().unwrap().to_string(),
                })
            }
            _ => None,
        })
        .collect()
}

/// An actor host `id` named `name`, readable by default or not, carrying one effect that expires
/// at the host's own turn end and starts at `remaining = 1`.
fn host_named(id: u128, name: &str, host_readable: bool) -> Document {
    let mut host = actor_with_effect(id, None, 1, ExpiryPoint::TurnEnd, DurationUnit::Turns);
    host.name = Some(name.into());
    host.permissions.default = if host_readable {
        DocRole::Observer
    } else {
        DocRole::None
    };
    host
}

/// Names the host's one effect, sets whether its own document is readable by default, and gives
/// it `origin`.
fn style_effect(host: &mut Document, name: &str, readable: bool, origin: Option<EffectOrigin>) {
    let effect = &mut host.embedded.get_mut("effect").unwrap()[0];
    effect.name = Some(name.into());
    effect.permissions.default = if readable {
        DocRole::Observer
    } else {
        DocRole::None
    };
    let mut engine: EffectEngine = serde_json::from_value(effect.engine.clone().unwrap()).unwrap();
    engine.origin = origin;
    effect.engine = Some(serde_json::to_value(&engine).unwrap());
}

/// A label-only origin.
fn labelled(label: &str) -> Option<EffectOrigin> {
    Some(EffectOrigin {
        source: None,
        label: Some(label.into()),
    })
}

/// Advances a one-combatant combat at round 1 whose combatant is `hidden` or not, over `hosts`,
/// with `world_default` as the world layer of the defaults chain.
fn advance_over(
    hosts: Vec<Document>,
    hidden: bool,
    world_default: Option<CombatDefaults>,
) -> (CombatSnapshot, Vec<Operation>) {
    let combat = crate::test_ids::test_uuid(1);
    let a = actor_combatant(10, combat, 0x9A, None, hidden, 0.0);
    let mut snap = snapshot(
        combat_engine(vec![a.doc.id], Some(a.doc.id), 1, true),
        vec![a],
        hosts,
    );
    snap.chain = (None, world_default, None);
    let ops = advance(&snap, WORLD, Uuid::nil(), 0).unwrap();
    (snap, ops)
}

#[test]
fn a_public_effects_expiry_is_named_to_everyone_with_its_origin_label() {
    let mut host = host_named(0x9A, "Seraphine", true);
    style_effect(&mut host, "Bless", true, labelled("Aldric"));
    let (snap, ops) = advance_over(vec![host], false, None);
    let n = notices(&ops);
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].audience, "public");
    assert_eq!(n[0].text, "Expired: Bless on Seraphine (from Aldric)");
    let docs = apply(&snap, &ops);
    assert!(
        !effect_on_9a_active(&docs),
        "the note accompanies a real expiry"
    );
}

/// Whether the effect on host `0x9A` is still active in an applied doc map.
fn effect_on_9a_active(docs: &HashMap<Uuid, Document>) -> bool {
    let e: EffectEngine = serde_json::from_value(
        docs[&crate::test_ids::test_uuid(0x9A)].embedded["effect"][0]
            .engine
            .clone()
            .unwrap(),
    )
    .unwrap();
    e.active
}

#[test]
fn an_effect_with_no_origin_is_named_without_a_from_clause() {
    let mut host = host_named(0x9A, "Seraphine", true);
    style_effect(&mut host, "Bless", true, None);
    let (_, ops) = advance_over(vec![host], false, None);
    let n = notices(&ops);
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].text, "Expired: Bless on Seraphine");
}

#[test]
fn a_source_only_origin_is_named_by_the_source_documents_name_and_a_dangling_one_is_omitted() {
    let mut named = host_named(0x9A, "Seraphine", true);
    let queen = host_named(0x300, "Spider Queen", true);
    style_effect(
        &mut named,
        "Poison",
        true,
        Some(EffectOrigin {
            source: Some(queen.id),
            label: None,
        }),
    );
    let (_, ops) = advance_over(vec![named, queen], false, None);
    assert_eq!(
        notices(&ops)[0].text,
        "Expired: Poison on Seraphine (from Spider Queen)"
    );

    let mut dangling = host_named(0x9A, "Seraphine", true);
    style_effect(
        &mut dangling,
        "Poison",
        true,
        Some(EffectOrigin {
            source: Some(crate::test_ids::test_uuid(0x404)),
            label: None,
        }),
    );
    let (_, ops) = advance_over(vec![dangling], false, None);
    assert_eq!(notices(&ops)[0].text, "Expired: Poison on Seraphine");
}

#[test]
fn the_gm_only_setting_sends_even_a_public_effects_note_to_the_gm_alone() {
    let mut host = host_named(0x9A, "Seraphine", true);
    style_effect(&mut host, "Bless", true, None);
    let setting = CombatDefaults {
        expiry_notice: Some(ExpiryNotice::GmOnly),
        ..Default::default()
    };
    let (_, ops) = advance_over(vec![host], false, Some(setting));
    let n = notices(&ops);
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].audience, "gm_only");
}

#[test]
fn an_effect_everyone_cannot_read_is_named_to_the_gm_alone_under_the_default_setting() {
    // Each case unreadable on exactly one axis: the effect document, its host, the combatant
    // that swept it.
    let mut effect_hidden = host_named(0x9A, "Seraphine", true);
    style_effect(&mut effect_hidden, "Curse", false, None);
    let mut host_hidden = host_named(0x9A, "Seraphine", false);
    style_effect(&mut host_hidden, "Curse", true, None);
    let mut combatant_hidden = host_named(0x9A, "Seraphine", true);
    style_effect(&mut combatant_hidden, "Curse", true, None);
    for (host, hidden, axis) in [
        (effect_hidden, false, "effect"),
        (host_hidden, false, "host"),
        (combatant_hidden, true, "combatant"),
    ] {
        let (_, ops) = advance_over(vec![host], hidden, None);
        let n = notices(&ops);
        assert_eq!(n.len(), 1, "{axis}");
        assert_eq!(n[0].audience, "gm_only", "{axis}");
        assert!(n[0].text.contains("Curse"), "{axis}");
    }
}

#[test]
fn an_effect_on_a_secret_identity_token_is_named_to_the_gm_alone() {
    let mut host = host_named(0x9A, "Wolf Pack Leader", true);
    host.doc_type = "token".into();
    host.engine = Some(json!({ "identity_hidden": true }));
    style_effect(&mut host, "Bless", true, None);
    let (_, ops) = advance_over(vec![host], false, None);
    let n = notices(&ops);
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].audience, "gm_only");
    assert!(n[0].text.contains("Wolf Pack Leader"));
}

#[test]
fn an_effect_on_the_actor_a_secret_identity_token_links_is_named_to_the_gm_alone() {
    let mut actor = host_named(0x9A, "Seraphine", true);
    style_effect(&mut actor, "Bless", true, None);
    let mut token = doc(
        0x500,
        "token",
        None,
        json!({
            "identity_hidden": true,
            "actor_id": crate::test_ids::test_uuid(0x9A).to_string(),
        }),
    );
    token.permissions.default = DocRole::Observer;
    let (_, ops) = advance_over(vec![actor, token], false, None);
    let n = notices(&ops);
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].audience, "gm_only");
}

#[test]
fn an_origin_source_nobody_else_can_read_is_left_out_of_the_public_note() {
    let mut named = host_named(0x9A, "Seraphine", true);
    let queen = host_named(0x300, "Spider Queen", false);
    style_effect(
        &mut named,
        "Poison",
        true,
        Some(EffectOrigin {
            source: Some(queen.id),
            label: None,
        }),
    );
    let (_, ops) = advance_over(vec![named, queen], false, None);
    let n = notices(&ops);
    assert_eq!(n[0].audience, "public");
    assert_eq!(n[0].text, "Expired: Poison on Seraphine");
}

#[test]
fn a_command_mixing_public_and_private_expiries_posts_one_note_per_audience() {
    let mut host = host_named(0x9A, "Seraphine", true);
    style_effect(&mut host, "Bless", true, None);
    let mut secret = host.embedded["effect"][0].clone();
    secret.id = crate::test_ids::test_uuid(0x9B);
    secret.name = Some("Hidden Curse".into());
    secret.permissions.default = DocRole::None;
    host.embedded.get_mut("effect").unwrap().push(secret);
    let (_, ops) = advance_over(vec![host], false, None);
    let n = notices(&ops);
    assert_eq!(n.len(), 2, "one public note and one GM-only note");
    let public = n.iter().find(|x| x.audience == "public").unwrap();
    let private = n.iter().find(|x| x.audience == "gm_only").unwrap();
    assert!(public.text.contains("Bless") && !public.text.contains("Hidden Curse"));
    assert!(private.text.contains("Hidden Curse") && !private.text.contains("Bless"));
}

#[test]
fn several_public_expiries_in_one_command_share_one_note() {
    let mut host = host_named(0x9A, "Seraphine", true);
    style_effect(&mut host, "Bless", true, None);
    let mut second = host.embedded["effect"][0].clone();
    second.id = crate::test_ids::test_uuid(0x9B);
    second.name = Some("Haste".into());
    host.embedded.get_mut("effect").unwrap().push(second);
    let (_, ops) = advance_over(vec![host], false, None);
    let n = notices(&ops);
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].text, "Expired: Bless on Seraphine; Haste on Seraphine");
}

#[test]
fn a_command_that_expires_nothing_posts_no_expiry_note() {
    let mut host = host_named(0x9A, "Seraphine", true);
    style_effect(&mut host, "Bless", true, None);
    // Three turns remain, so the turn-end tick only decrements.
    let mut engine: EffectEngine =
        serde_json::from_value(host.embedded["effect"][0].engine.clone().unwrap()).unwrap();
    engine.duration.as_mut().unwrap().remaining = Some(3);
    host.embedded.get_mut("effect").unwrap()[0].engine =
        Some(serde_json::to_value(&engine).unwrap());
    let (_, ops) = advance_over(vec![host], false, None);
    assert!(notices(&ops).is_empty());
}

#[test]
fn ending_a_combat_names_the_effects_its_cleanup_expires() {
    let combat = crate::test_ids::test_uuid(1);
    let a = actor_combatant(10, combat, 0x9A, None, false, 0.0);
    let mut host = host_named(0x9A, "Seraphine", true);
    style_effect(&mut host, "Bless", true, labelled("Aldric"));
    let snap = snapshot(
        combat_engine(vec![a.doc.id], Some(a.doc.id), 1, true),
        vec![a],
        vec![host],
    );
    let ops = end(&snap, WORLD, Uuid::nil(), 0).unwrap();
    let n = notices(&ops);
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].audience, "public");
    assert_eq!(n[0].text, "Expired: Bless on Seraphine (from Aldric)");
}
