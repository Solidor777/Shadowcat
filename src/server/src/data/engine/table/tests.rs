use super::*;
use crate::chat::rolls::MAX_DIE_SIDES;
use crate::data::document::Document;

fn weighted_row(weight: u32, label: &str) -> TableRow {
    TableRow {
        weight,
        range: None,
        label: label.to_string(),
        results: vec![],
    }
}

fn ranged_row(lo: i32, hi: i32, label: &str) -> TableRow {
    TableRow {
        weight: 1,
        range: Some(RowRange { lo, hi }),
        label: label.to_string(),
        results: vec![],
    }
}

#[test]
fn a_valid_weighted_table_is_accepted() {
    let table = TableEngine {
        draw: DrawRule::Weighted,
        rows: vec![weighted_row(1, "a"), weighted_row(2, "b")],
        description: String::new(),
    };
    assert!(table.validate().is_ok());
}

#[test]
fn a_weighted_row_with_weight_zero_is_rejected() {
    let table = TableEngine {
        draw: DrawRule::Weighted,
        rows: vec![weighted_row(0, "a")],
        description: String::new(),
    };
    assert!(table.validate().is_err());
}

#[test]
fn a_range_under_weighted_is_rejected() {
    let table = TableEngine {
        draw: DrawRule::Weighted,
        rows: vec![ranged_row(1, 2, "a")],
        description: String::new(),
    };
    assert!(table.validate().is_err());
}

#[test]
fn weighted_sum_bound_is_the_chat_die_cap() {
    // Positive control: a sum exactly at the cap is accepted.
    let at_cap = TableEngine {
        draw: DrawRule::Weighted,
        rows: vec![weighted_row(MAX_DIE_SIDES as u32, "a")],
        description: String::new(),
    };
    assert!(at_cap.validate().is_ok());

    // A sum one past the cap is rejected.
    let over_cap = TableEngine {
        draw: DrawRule::Weighted,
        rows: vec![weighted_row(MAX_DIE_SIDES as u32 + 1, "a")],
        description: String::new(),
    };
    assert!(over_cap.validate().is_err());
}

#[test]
fn a_missing_range_under_formula_is_rejected() {
    let table = TableEngine {
        draw: DrawRule::Formula {
            notation: "2d6".to_string(),
        },
        rows: vec![weighted_row(1, "a")],
        description: String::new(),
    };
    assert!(table.validate().is_err());
}

#[test]
fn a_row_range_with_lo_greater_than_hi_is_rejected() {
    let table = TableEngine {
        draw: DrawRule::Formula {
            notation: "2d6".to_string(),
        },
        rows: vec![ranged_row(6, 2, "a")],
        description: String::new(),
    };
    assert!(table.validate().is_err());
}

#[test]
fn overlapping_ranges_are_rejected() {
    let table = TableEngine {
        draw: DrawRule::Formula {
            notation: "2d6".to_string(),
        },
        rows: vec![ranged_row(2, 6, "a"), ranged_row(5, 8, "b")],
        description: String::new(),
    };
    assert!(table.validate().is_err());
}

#[test]
fn non_overlapping_ranges_are_accepted() {
    let table = TableEngine {
        draw: DrawRule::Formula {
            notation: "2d6".to_string(),
        },
        rows: vec![ranged_row(2, 6, "a"), ranged_row(7, 12, "b")],
        description: String::new(),
    };
    assert!(table.validate().is_ok());
}

#[test]
fn a_referencing_formula_is_rejected() {
    let table = TableEngine {
        draw: DrawRule::Formula {
            notation: "1d20+str".to_string(),
        },
        rows: vec![ranged_row(1, 30, "a")],
        description: String::new(),
    };
    assert!(table.validate().is_err());
}

#[test]
fn a_success_count_formula_is_rejected() {
    let table = TableEngine {
        draw: DrawRule::Formula {
            notation: "5d10cs>=7".to_string(),
        },
        rows: vec![ranged_row(0, 5, "a")],
        description: String::new(),
    };
    assert!(table.validate().is_err());
}

#[test]
fn more_than_max_table_rows_is_rejected() {
    let rows: Vec<TableRow> = (0..(MAX_TABLE_ROWS + 1))
        .map(|i| weighted_row(1, &format!("row{i}")))
        .collect();
    let table = TableEngine {
        draw: DrawRule::Weighted,
        rows,
        description: String::new(),
    };
    assert!(table.validate().is_err());
}

#[test]
fn an_empty_label_is_rejected() {
    let table = TableEngine {
        draw: DrawRule::Weighted,
        rows: vec![weighted_row(1, "   ")],
        description: String::new(),
    };
    assert!(table.validate().is_err());
}

#[test]
fn over_cap_text_is_rejected() {
    let mut row = weighted_row(1, "a");
    row.results = vec![TableEntry::Text {
        text: "x".repeat(MAX_ROW_TEXT_CHARS + 1),
    }];
    let table = TableEngine {
        draw: DrawRule::Weighted,
        rows: vec![row],
        description: String::new(),
    };
    assert!(table.validate().is_err());
}

#[test]
fn over_cap_alt_is_rejected() {
    let mut row = weighted_row(1, "a");
    row.results = vec![TableEntry::Image {
        asset_id: Uuid::nil(),
        alt: "x".repeat(MAX_IMAGE_ALT_CHARS + 1),
    }];
    let table = TableEngine {
        draw: DrawRule::Weighted,
        rows: vec![row],
        description: String::new(),
    };
    assert!(table.validate().is_err());
}

#[test]
fn over_cap_description_is_rejected() {
    let table = TableEngine {
        draw: DrawRule::Weighted,
        rows: vec![weighted_row(1, "a")],
        description: "x".repeat(MAX_TABLE_DESCRIPTION_CHARS + 1),
    };
    assert!(table.validate().is_err());
}

#[test]
fn a_nested_draw_count_of_zero_is_rejected() {
    let mut row = weighted_row(1, "a");
    row.results = vec![TableEntry::Draw {
        table_id: Uuid::nil(),
        count: 0,
    }];
    let table = TableEngine {
        draw: DrawRule::Weighted,
        rows: vec![row],
        description: String::new(),
    };
    assert!(table.validate().is_err());
}

#[test]
fn a_nested_draw_count_over_the_max_is_rejected() {
    let mut row = weighted_row(1, "a");
    row.results = vec![TableEntry::Draw {
        table_id: Uuid::nil(),
        count: MAX_NESTED_DRAW_COUNT + 1,
    }];
    let table = TableEngine {
        draw: DrawRule::Weighted,
        rows: vec![row],
        description: String::new(),
    };
    assert!(table.validate().is_err());
}

#[test]
fn a_nested_draw_count_at_the_max_is_accepted() {
    let mut row = weighted_row(1, "a");
    row.results = vec![TableEntry::Draw {
        table_id: Uuid::nil(),
        count: MAX_NESTED_DRAW_COUNT,
    }];
    let table = TableEngine {
        draw: DrawRule::Weighted,
        rows: vec![row],
        description: String::new(),
    };
    assert!(table.validate().is_ok());
}

#[test]
fn normalize_engine_table_round_trips() {
    let body = serde_json::json!({
        "draw": { "kind": "weighted" },
        "rows": [{ "weight": 1, "label": "a", "results": [] }],
        "description": ""
    });
    let normalized = crate::data::engine::normalize_engine_opt(TABLE_DOC_TYPE, Some(&body))
        .unwrap()
        .unwrap();
    let typed: TableEngine = serde_json::from_value(normalized).unwrap();
    assert_eq!(typed.rows.len(), 1);
}

#[test]
fn validate_engine_rejects_a_table_body_on_a_non_engine_doc_type() {
    let body = serde_json::json!({
        "draw": { "kind": "weighted" },
        "rows": [],
        "description": ""
    });
    assert!(crate::data::engine::validate_engine("item", Some(&body)).is_err());
}

#[test]
fn a_table_document_cannot_have_a_parent() {
    let doc = table_doc_with_parent(Some(Uuid::new_v4()));
    assert!(crate::data::validation::validate_containment(&doc).is_err());
}

#[test]
fn a_top_level_table_document_passes_containment() {
    let doc = table_doc_with_parent(None);
    assert!(crate::data::validation::validate_containment(&doc).is_ok());
}

fn table_doc_with_parent(parent_id: Option<Uuid>) -> Document {
    let world_id = Uuid::new_v4();
    serde_json::from_value(serde_json::json!({
        "id": Uuid::new_v4(),
        "scope": { "kind": "world", "world_id": world_id },
        "doc_type": TABLE_DOC_TYPE,
        "schema_version": 1,
        "parent_id": parent_id,
        "engine": {
            "draw": { "kind": "weighted" },
            "rows": [],
            "description": ""
        },
        "system": {},
        "created_at": 0,
        "updated_at": 0
    }))
    .unwrap()
}
