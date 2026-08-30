use super::*;

#[test]
fn compile_regex_rejects_oversize_and_invalid_patterns() {
    assert!(compile_regex("^cr.pt$").is_ok());
    let long = "a".repeat(MAX_REGEX_BYTES + 1);
    assert!(matches!(compile_regex(&long), Err(AppError::BadRequest(_))));
    assert!(matches!(compile_regex("("), Err(AppError::BadRequest(_))));
    // A pattern whose compiled program would blow the size cap is refused,
    // not compiled.
    assert!(matches!(
        compile_regex("(a{1000}){1000}"),
        Err(AppError::BadRequest(_))
    ));
}

#[test]
fn cursor_round_trips_and_rejects_garbage() {
    let c = AssetCursor {
        sort_key: "map of crypt.png".into(),
        id: Uuid::from_u128(7),
    };
    let text = encode_cursor(&c);
    assert!(!text.contains(CURSOR_SEP));
    assert_eq!(decode_cursor(&text).unwrap(), c);
    // A sort key containing the separator still round-trips: the id is
    // split off from the RIGHT.
    let tricky = AssetCursor {
        sort_key: format!("a{CURSOR_SEP}b"),
        id: Uuid::from_u128(8),
    };
    assert_eq!(decode_cursor(&encode_cursor(&tricky)).unwrap(), tricky);
    assert!(matches!(decode_cursor("!!"), Err(AppError::BadRequest(_))));
    assert!(matches!(
        decode_cursor(&base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("no-sep")),
        Err(AppError::BadRequest(_))
    ));
}

#[test]
fn parse_validates_every_parameter() {
    let bare = AssetQuery::default();
    assert!(bare.is_bare());
    let q = AssetQuery {
        folder: Some("root".into()),
        tags: Some(" hero, ,image".into()),
        kind: Some("other".into()),
        sort: Some("size".into()),
        limit: Some(2),
        name: Some("".into()),
        ..AssetQuery::default()
    };
    assert!(!q.is_bare());
    let p = parse(q).unwrap();
    assert_eq!(p.filter.folder, Some(FolderFilter::Root));
    assert_eq!(p.filter.tags, vec!["hero".to_string(), "image".to_string()]);
    assert_eq!(p.filter.kind, Some(AssetKind::Other));
    assert_eq!(p.filter.name, None, "empty name is no filter");
    assert_eq!(p.sort, AssetSort::Size);
    assert_eq!(p.limit, 2);

    for bad in [
        AssetQuery {
            folder: Some("not-a-uuid".into()),
            ..AssetQuery::default()
        },
        AssetQuery {
            kind: Some("video".into()),
            ..AssetQuery::default()
        },
        AssetQuery {
            sort: Some("random".into()),
            ..AssetQuery::default()
        },
        AssetQuery {
            limit: Some(0),
            ..AssetQuery::default()
        },
        AssetQuery {
            limit: Some(MAX_LIMIT + 1),
            ..AssetQuery::default()
        },
        AssetQuery {
            cursor: Some("!!".into()),
            ..AssetQuery::default()
        },
    ] {
        assert!(matches!(parse(bad), Err(AppError::BadRequest(_))));
    }
}
