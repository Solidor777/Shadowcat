use super::*;
use crate::dice::notation::ModeKind;
use crate::dice::spec::Direction;

fn total_ctx() -> ParseContext {
    ParseContext {
        mode: ModeKind::Total,
        direction: Direction::HighWins,
    }
}

fn success_ctx() -> ParseContext {
    ParseContext {
        mode: ModeKind::SuccessCount,
        direction: Direction::HighWins,
    }
}

// --- Caps ---

#[test]
fn dice_count_at_limit_accepts() {
    assert!(validate_formula("100d6", total_ctx()).is_ok());
}

#[test]
fn dice_count_past_limit_rejects() {
    match validate_formula("101d6", total_ctx()) {
        Err(RollError::TooManyDice(101)) => {}
        other => panic!("expected TooManyDice(101), got {other:?}"),
    }
}

#[test]
fn expertise_at_limit_accepts() {
    assert!(validate_formula("1d20t10e100", success_ctx()).is_ok());
}

#[test]
fn expertise_past_limit_rejects() {
    match validate_formula("1d20t10e101", success_ctx()) {
        Err(RollError::ExpertiseTooLarge(101)) => {}
        other => panic!("expected ExpertiseTooLarge(101), got {other:?}"),
    }
}

#[test]
fn sides_at_limit_accepts() {
    assert!(validate_formula("1d10000", total_ctx()).is_ok());
}

#[test]
fn sides_past_limit_rejects() {
    match validate_formula("1d10001", total_ctx()) {
        Err(RollError::SidesTooLarge(10_001)) => {}
        other => panic!("expected SidesTooLarge(10001), got {other:?}"),
    }
}

#[test]
fn records_cap_rejects_post_roll() {
    // The base dice-count cap (100) is enforced before any roll happens, so
    // exceeding `MAX_ROLL_RECORDS` (1000) via base count alone is
    // unreachable through `execute_roll`/`execute_roll_with_seed` -- the
    // records cap exists to bound an explosion chain's fan-out instead.
    // `100d2!>=1` sets an explicit explode target of 1: every `d2` face
    // (1 or 2) satisfies `>=1`, so every base die's chain deterministically
    // runs to `CHAIN_CAP = 100` (`eval::groups`'s per-die chain cap)
    // regardless of seed -- 100 base dice x (1 + 100 chained extras) =
    // 10_100 records, well past `MAX_ROLL_RECORDS`. No seed search needed.
    match execute_roll_with_seed("100d2!>=1", total_ctx(), None, 42) {
        Err(RollError::TooManyRecords(n)) => assert!(n > MAX_ROLL_RECORDS),
        other => panic!("expected TooManyRecords, got {other:?}"),
    }
}

// --- Scanner grammar matrix ---

#[test]
fn scan_plain_text_is_one_text_chunk() {
    let chunks = scan_body("hello world").unwrap();
    assert_eq!(chunks, vec![BodyChunk::Text("hello world")]);
}

#[test]
fn scan_single_inline_roll() {
    let chunks = scan_body("rolling [[2d6]] now").unwrap();
    assert_eq!(
        chunks,
        vec![
            BodyChunk::Text("rolling "),
            BodyChunk::Inline("2d6"),
            BodyChunk::Text(" now"),
        ]
    );
}

#[test]
fn scan_multiple_inline_rolls() {
    let chunks = scan_body("[[1d4]] and [[1d6]]").unwrap();
    assert_eq!(
        chunks,
        vec![
            BodyChunk::Inline("1d4"),
            BodyChunk::Text(" and "),
            BodyChunk::Inline("1d6"),
        ]
    );
}

#[test]
fn scan_button_without_label() {
    let chunks = scan_body("[[roll:2d6+3]]").unwrap();
    assert_eq!(
        chunks,
        vec![BodyChunk::Button {
            formula: "2d6+3",
            label: None,
        }]
    );
}

#[test]
fn scan_button_with_label() {
    let chunks = scan_body("[[roll:2d6+3|Attack]]").unwrap();
    assert_eq!(
        chunks,
        vec![BodyChunk::Button {
            formula: "2d6+3",
            label: Some("Attack"),
        }]
    );
}

#[test]
fn scan_button_with_whitespace_only_label_is_none() {
    let chunks = scan_body("[[roll:2d6|   ]]").unwrap();
    assert_eq!(
        chunks,
        vec![BodyChunk::Button {
            formula: "2d6",
            label: None,
        }]
    );
}

#[test]
fn scan_button_empty_formula_parses_then_fails_downstream() {
    let chunks = scan_body("[[roll:]]").unwrap();
    assert_eq!(
        chunks,
        vec![BodyChunk::Button {
            formula: "",
            label: None,
        }]
    );
    match validate_formula("", total_ctx()) {
        Err(RollError::Parse(ParseError::Empty)) => {}
        other => panic!("expected Parse(Empty), got {other:?}"),
    }
}

#[test]
fn scan_nested_label_survives_balanced_brackets() {
    let chunks = scan_body("[[4d6[atk]]]").unwrap();
    assert_eq!(chunks, vec![BodyChunk::Inline("4d6[atk]")]);
}

#[test]
fn scan_adjacent_spans_with_no_text_between() {
    let chunks = scan_body("[[1d4]][[1d6]]").unwrap();
    assert_eq!(
        chunks,
        vec![BodyChunk::Inline("1d4"), BodyChunk::Inline("1d6")]
    );
}

#[test]
fn scan_unterminated_span_errors() {
    assert_eq!(
        scan_body("text [[1d6 no close"),
        Err(RollError::Unterminated)
    );
}

#[test]
fn scan_doc_link() {
    let chunks = scan_body("[[doc:00000000-0000-0000-0000-000000000001|My Document]]").unwrap();
    assert_eq!(
        chunks,
        vec![BodyChunk::DocLink {
            target: DocLinkTarget::Doc {
                doc_id: Uuid::from_u128(1),
                embedded_path: None,
            },
            label: "My Document",
        }]
    );
}

#[test]
fn scan_doc_link_with_embedded_path() {
    let chunks =
        scan_body("[[doc:00000000-0000-0000-0000-000000000001/embedded/actor/0|My Item]]").unwrap();
    assert_eq!(
        chunks,
        vec![BodyChunk::DocLink {
            target: DocLinkTarget::Doc {
                doc_id: Uuid::from_u128(1),
                embedded_path: Some("/embedded/actor/0".into()),
            },
            label: "My Item",
        }]
    );
}

#[test]
fn scan_token_link() {
    let chunks = scan_body("[[token:00000000-0000-0000-0000-000000000002|Goblin]]").unwrap();
    assert_eq!(
        chunks,
        vec![BodyChunk::DocLink {
            target: DocLinkTarget::Token {
                token_id: Uuid::from_u128(2),
            },
            label: "Goblin",
        }]
    );
}

#[test]
fn scan_doc_link_with_surrounding_text() {
    let chunks = scan_body("see [[doc:00000000-0000-0000-0000-000000000001|Doc]] please").unwrap();
    assert_eq!(
        chunks,
        vec![
            BodyChunk::Text("see "),
            BodyChunk::DocLink {
                target: DocLinkTarget::Doc {
                    doc_id: Uuid::from_u128(1),
                    embedded_path: None,
                },
                label: "Doc",
            },
            BodyChunk::Text(" please"),
        ]
    );
}

#[test]
fn scan_doc_link_missing_label_is_malformed() {
    assert_eq!(
        scan_body("[[doc:00000000-0000-0000-0000-000000000001]]"),
        Err(RollError::MalformedDocLink)
    );
}

#[test]
fn scan_doc_link_empty_label_is_malformed() {
    assert_eq!(
        scan_body("[[doc:00000000-0000-0000-0000-000000000001|   ]]"),
        Err(RollError::MalformedDocLink)
    );
}

#[test]
fn scan_doc_link_bad_uuid_is_malformed() {
    assert_eq!(
        scan_body("[[doc:not-a-uuid|Label]]"),
        Err(RollError::MalformedDocLink)
    );
}

#[test]
fn scan_token_link_missing_label_is_malformed() {
    assert_eq!(
        scan_body("[[token:00000000-0000-0000-0000-000000000002]]"),
        Err(RollError::MalformedDocLink)
    );
}

#[test]
fn scan_token_link_empty_label_is_malformed() {
    assert_eq!(
        scan_body("[[token:00000000-0000-0000-0000-000000000002|   ]]"),
        Err(RollError::MalformedDocLink)
    );
}

#[test]
fn scan_token_link_bad_uuid_is_malformed() {
    assert_eq!(
        scan_body("[[token:not-a-uuid|Label]]"),
        Err(RollError::MalformedDocLink)
    );
}

#[test]
fn scan_doc_link_label_may_contain_slash_and_pipe() {
    // The id/path split only ever runs on the portion BEFORE the first `|`
    // (`rest.split_once('|')`), so a `/` or `|` inside the label — which is
    // everything after that first `|` — can never re-enter either split.
    let chunks = scan_body("[[doc:00000000-0000-0000-0000-000000000001|A/B|C]]").unwrap();
    assert_eq!(
        chunks,
        vec![BodyChunk::DocLink {
            target: DocLinkTarget::Doc {
                doc_id: Uuid::from_u128(1),
                embedded_path: None,
            },
            label: "A/B|C",
        }]
    );
}

#[test]
fn scan_doc_link_counts_toward_max_inline_rolls() {
    let body = "[[doc:00000000-0000-0000-0000-000000000001|D]] ".repeat(MAX_INLINE_ROLLS);
    assert!(scan_body(&body).is_ok());
    let over = "[[doc:00000000-0000-0000-0000-000000000001|D]] ".repeat(MAX_INLINE_ROLLS + 1);
    assert!(matches!(scan_body(&over), Err(RollError::TooManyInline(_))));
}

#[test]
fn scan_unterminated_nested_span_errors() {
    assert_eq!(scan_body("[[4d6[atk]"), Err(RollError::Unterminated));
}

#[test]
fn scan_max_inline_rolls_at_limit_accepts() {
    let body = "[[1d6]]".repeat(MAX_INLINE_ROLLS);
    let chunks = scan_body(&body).unwrap();
    assert_eq!(chunks.len(), MAX_INLINE_ROLLS);
}

#[test]
fn scan_max_inline_rolls_past_limit_rejects() {
    let body = "[[1d6]]".repeat(MAX_INLINE_ROLLS + 1);
    match scan_body(&body) {
        Err(RollError::TooManyInline(n)) => assert_eq!(n, MAX_INLINE_ROLLS + 1),
        other => panic!("expected TooManyInline, got {other:?}"),
    }
}

// --- Determinism / entropy ---

#[test]
fn execute_roll_with_seed_is_deterministic() {
    let a = execute_roll_with_seed("4d6+2", total_ctx(), None, 12345).unwrap();
    let b = execute_roll_with_seed("4d6+2", total_ctx(), None, 12345).unwrap();
    assert_eq!(a, b);
}

#[test]
fn execute_roll_returns_the_formula_verbatim() {
    let (formula, _, _, _) = execute_roll_with_seed("2d6+1", total_ctx(), None, 1).unwrap();
    assert_eq!(formula, "2d6+1");
}

#[test]
fn execute_roll_with_seed_returns_spec_and_raw_matching_the_outcome() {
    let (_, outcome, spec, raw) = execute_roll_with_seed("2d6+1", total_ctx(), None, 5).unwrap();
    // spec/raw are exactly what `evaluate` was run against -- re-evaluating
    // them independently must reproduce the same outcome.
    assert_eq!(crate::dice::evaluate(&spec, &raw), outcome);
    assert_eq!(raw.dice.len(), 2);
}

#[test]
fn entropy_seed_two_calls_differ() {
    // Not a formal randomness proof -- a sanity check that two draws are
    // not trivially identical (would indicate a broken/constant seed source).
    let a = entropy_seed();
    let b = entropy_seed();
    assert_ne!(a, b);
}

#[test]
fn execute_roll_rejects_over_cap_formula() {
    match execute_roll("101d6", total_ctx(), None) {
        Err(RollError::TooManyDice(101)) => {}
        other => panic!("expected TooManyDice(101), got {other:?}"),
    }
}

#[test]
fn validate_formula_rejects_the_same_way_execute_roll_would() {
    // validate_pre_roll runs before any RNG use, so a cap rejection is
    // identical whether reached via validate_formula or execute_roll.
    match validate_formula("101d6", total_ctx()) {
        Err(RollError::TooManyDice(101)) => {}
        other => panic!("expected TooManyDice(101), got {other:?}"),
    }
}

#[test]
fn dice_nested_inside_a_call_argument_still_counts_toward_max_roll_dice() {
    // `walk_groups` must recurse into `Expr::Call`'s args -- otherwise a dice group
    // wrapped in floor/ceil/round/abs/min/max would bypass MAX_ROLL_DICE entirely.
    match validate_formula("floor(101d6/2)", total_ctx()) {
        Err(RollError::TooManyDice(101)) => {}
        other => panic!("expected TooManyDice(101), got {other:?}"),
    }
}

#[test]
fn xs_modifier_from_notation_fires_crit_success_end_to_end() {
    let spec = notation::parse("6d6cs>=4xs6", total_ctx()).unwrap();
    // Seed chosen so at least one d6 rolls a 6 (verified: exactly one) -- required
    // for this test to be non-vacuous; see the assert! below.
    let raws = roll(&spec, &mut NoiseRng::from_seed(4));
    let out = eval::evaluate(&spec, &raws);
    let expected_crits = raws
        .records
        .iter()
        .filter(|r| r.kept && r.value >= 6)
        .count() as i32;
    assert!(
        expected_crits > 0,
        "test is vacuous for this seed -- no d6 rolled a 6, so it can't distinguish \
         xs6 firing from xs6 being silently dropped"
    );
    assert_eq!(out.crit_successes, expected_crits);
}

#[test]
fn pure_const_multiplication_chain_saturates_without_panic() {
    // Zero dice groups: `walk_groups` counts none, so `MAX_ROLL_DICE`/
    // `MAX_ROLL_RECORDS` never see this formula. Run under a debug build
    // (overflow-checks on) -- if the fold used raw `*` this would panic;
    // reaching a saturated result proves it does not.
    let (_, out, _, _) =
        execute_roll_with_seed("2000000000*2000000000*3", total_ctx(), None, 1).unwrap();
    assert_eq!(out.total, i64::MAX);
}

#[test]
fn multi_group_multiplication_saturates_without_panic() {
    // Two `d10000` groups multiplied together: within `MAX_ROLL_DICE`/
    // `MAX_ROLL_SIDES`, but `1d10000 * 1d10000` can still reach values
    // near `i64::MAX` depending on draws; assert only that evaluation
    // completes (no panic) and the total is a finite, non-negative i64
    // (both dice draws are positive, so the true product is always >= 0,
    // never spuriously saturating to `i64::MIN`).
    let (_, out, _, _) = execute_roll_with_seed("1d10000*1d10000", total_ctx(), None, 7).unwrap();
    assert!(out.total >= 0);
}

#[test]
fn roll_error_display_has_no_debug_artifacts() {
    let variants = vec![
        RollError::Parse(ParseError::Empty),
        RollError::TooManyDice(200),
        RollError::TooManyRecords(2000),
        RollError::ExpertiseTooLarge(200),
        RollError::SidesTooLarge(20_000),
        RollError::TooManyInline(9),
        RollError::Unterminated,
        RollError::DuplicateTierOffset(5),
        RollError::MalformedDocLink,
        RollError::Reference(crate::formula::FormulaError::new(
            crate::formula::FormulaErrorKind::UnknownRef,
            "unknown reference 'stats.str'",
        )),
    ];
    assert_eq!(
        variants.len(),
        10,
        "update this test if a RollError variant is added or removed"
    );
    for v in variants {
        let rendered = v.to_string();
        assert!(!rendered.contains("Some("), "{rendered}");
        assert!(!rendered.is_empty());
    }
}

#[test]
fn duplicate_tier_offsets_are_rejected_pre_roll() {
    use crate::dice::spec::{ConstTerm, Direction, Expr, Mode, RollSpec, Tier, TotalConfig};
    let spec = RollSpec {
        expr: Expr::Const(ConstTerm {
            value: 1,
            label: None,
        }),
        direction: Direction::HighWins,
        mode: Mode::Total(TotalConfig {
            difficulty: Some(0),
            tiers: vec![
                Tier {
                    margin_offset: 5,
                    label: Some("a".into()),
                    tier_value: Some(1),
                },
                Tier {
                    margin_offset: 5,
                    label: Some("b".into()),
                    tier_value: Some(2),
                },
            ],
        }),
    };
    assert!(matches!(
        validate_pre_roll(&spec),
        Err(RollError::DuplicateTierOffset(5))
    ));
    // Unique offsets pass.
    let mut ok = spec.clone();
    if let Mode::Total(cfg) = &mut ok.mode {
        cfg.tiers[1].margin_offset = 6;
    }
    assert!(validate_pre_roll(&ok).is_ok());
}

#[test]
fn duplicate_tr_offsets_from_notation_are_rejected_at_the_wire_boundary() {
    match validate_formula("4d6cs>4tr3:1[Good]tr3:2[Also]", total_ctx()) {
        Err(RollError::DuplicateTierOffset(3)) => {}
        other => panic!("expected DuplicateTierOffset(3), got {other:?}"),
    }
}

#[test]
fn tr_and_rs_together_produce_a_working_successcount_tier_classification_end_to_end() {
    // `tr<offset>` alone builds a `SuccessConfig.tiers` ladder, but
    // `evaluate_success` only classifies over it when `required_successes`
    // is set (`rs<N>`) -- without `rs`, `tr`'s ladder is inert (pass/margin/
    // tier stay None on every roll). Proves the pairing works end-to-end,
    // not just that each parses.
    let spec = notation::parse("6d6cs>=4rs2tr0[Fail]tr1[Pass]", success_ctx()).unwrap();
    let raws = roll(&spec, &mut NoiseRng::from_seed(1));
    let out = eval::evaluate(&spec, &raws);
    let net = out
        .successes
        .expect("SuccessCount mode always reports successes");
    assert_eq!(
        out.margin,
        Some((net - 2) as i64),
        "margin must be net successes minus the rs<N> required-successes target"
    );
    // A non-empty ladder always reports via `tier_label`/`tier_value`, never `pass`
    // (`classify::classify` sets `pass` only for the default empty-ladder case).
    assert!(
        out.tier_label.is_some(),
        "a tier ladder built by tr<offset> must actually classify once rs<N> supplies \
         the required-successes reference -- got tier_label={:?}",
        out.tier_label
    );
}

// --- Reference resolution ---

/// A host document carrying the given `system` band — the shape
/// `SystemLeafResolver` reads references from.
fn host_doc(system: serde_json::Value) -> Document {
    Document {
        id: Uuid::new_v4(),
        scope: crate::data::document::Scope::World {
            world_id: Uuid::new_v4(),
        },
        doc_type: "actor".into(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: crate::data::document::PermissionSet::default(),
        embedded: Default::default(),
        parent_id: None,
        engine: None,
        system,
        created_at: 0,
        updated_at: 0,
    }
}

#[test]
fn a_reference_resolves_against_the_host_system_band() {
    let host = host_doc(serde_json::json!({ "stats": { "str": 3 } }));
    let (formula, outcome, _, _) =
        execute_roll_with_seed("1d1+stats.str", total_ctx(), Some(&host), 1).unwrap();
    assert_eq!(
        formula, "1d1+stats.str",
        "the stored formula keeps the author's template text"
    );
    assert_eq!(outcome.total, 1 + 3);
    assert_eq!(
        outcome.labeled_consts,
        vec![crate::dice::spec::ConstTerm {
            value: 3,
            label: Some("stats.str".into())
        }],
        "the substituted reference surfaces as a labeled chip"
    );
}

#[test]
fn a_negative_reference_substitutes_as_a_signed_labeled_const() {
    let host = host_doc(serde_json::json!({ "stats": { "str": -2 } }));
    let (_, outcome, _, _) =
        execute_roll_with_seed("1d1+stats.str", total_ctx(), Some(&host), 1).unwrap();
    assert_eq!(outcome.total, 1 - 2);
    assert_eq!(
        outcome.labeled_consts,
        vec![crate::dice::spec::ConstTerm {
            value: -2,
            label: Some("stats.str".into())
        }]
    );
}

#[test]
fn a_reference_without_a_host_fails_unknown_ref() {
    match execute_roll("1d20+str", total_ctx(), None) {
        Err(RollError::Reference(e)) => assert_eq!(e.detail, "unknown reference 'str'"),
        other => panic!("expected Reference(unknown-ref), got {other:?}"),
    }
}

#[test]
fn a_reference_to_a_non_number_leaf_fails_with_a_type_error() {
    let host = host_doc(serde_json::json!({ "name": "Goblin" }));
    match execute_roll("1d20+name", total_ctx(), Some(&host)) {
        Err(RollError::Reference(e)) => {
            assert_eq!(e.error, crate::formula::FormulaErrorKind::Type)
        }
        other => panic!("expected Reference(type), got {other:?}"),
    }
}

#[test]
fn a_referencing_button_template_validates_without_a_host() {
    // Buttons validate structurally: references stand in as placeholder
    // zeros, so an unbound author can store a statted button.
    assert!(validate_formula("1d20+stats.str", total_ctx()).is_ok());
}

#[test]
fn a_button_template_with_an_unterminated_label_is_rejected() {
    match validate_formula("1d20[attack", total_ctx()) {
        Err(RollError::Reference(e)) => {
            assert_eq!(e.detail, "unterminated '[' label at position 4")
        }
        other => panic!("expected Reference(parse), got {other:?}"),
    }
}

#[test]
fn a_reference_free_template_is_byte_identical_after_resolution() {
    // Backwards compatibility: pre-substituted notation (what older clients
    // sent) and plain literal notation both roll unchanged.
    for src in ["2d6kh1+3", "1d20+3[str]"] {
        let (formula, _, _, _) = execute_roll(src, total_ctx(), None).unwrap();
        assert_eq!(formula, src);
    }
}
