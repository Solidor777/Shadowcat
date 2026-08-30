use super::*;

fn session(id: u128, byte_size: u64, last_touch_ms: i64) -> UploadSession {
    UploadSession {
        id: Uuid::from_u128(id),
        world: Uuid::from_u128(1),
        user: Uuid::from_u128(2),
        name: "big.bin".into(),
        content_type: "application/octet-stream".into(),
        byte_size,
        received: 0,
        folder_id: None,
        tags: vec![],
        staged: PathBuf::from("unused"),
        rate_hit_ms: last_touch_ms,
        last_touch_ms,
        busy: false,
    }
}

#[test]
fn accept_chunk_enforces_next_offset_busy_and_declared_size() {
    let mut s = session(1, 20, 0);
    // Wrong first offset.
    assert_eq!(
        s.accept_chunk(8, 8),
        Err(ChunkReject::OffsetMismatch { expected: 0 })
    );
    // First chunk admitted and marks the session busy.
    assert_eq!(s.accept_chunk(0, 8), Ok(()));
    assert!(s.busy);
    assert_eq!(s.accept_chunk(8, 8), Err(ChunkReject::Busy));
    s.finish_chunk(8, 5);
    assert_eq!(s.received, 8);
    assert_eq!(s.last_touch_ms, 5);
    // A duplicate of the accepted chunk is not a resume: only offset 8 fits.
    assert_eq!(
        s.accept_chunk(0, 8),
        Err(ChunkReject::OffsetMismatch { expected: 8 })
    );
    // Overflowing the declared size is refused before any write.
    assert_eq!(s.accept_chunk(8, 13), Err(ChunkReject::Overflow));
    assert!(!s.busy);
    assert_eq!(s.accept_chunk(8, 12), Ok(()));
    s.finish_chunk(12, 6);
    assert_eq!(s.received, 20);
}

#[test]
fn sweep_returns_only_idle_sessions() {
    let table = UploadSessions::new();
    table.insert(session(1, 10, 0));
    table.insert(session(2, 10, 1_000_000));
    let mut busy = session(3, 10, 0);
    busy.busy = true;
    table.insert(busy);
    assert_eq!(table.len(), 3);

    let now = SESSION_IDLE_MS + 1;
    let swept = table.sweep(now);
    let ids: Vec<Uuid> = swept.iter().map(|s| s.id).collect();
    assert_eq!(ids, vec![Uuid::from_u128(1)], "only the idle, non-busy one");
    assert_eq!(table.len(), 2);
    assert!(table.with(Uuid::from_u128(2), |_| ()).is_some());
    assert!(table.with(Uuid::from_u128(3), |_| ()).is_some());

    let swept = table.sweep(1_000_000 + SESSION_IDLE_MS + 1);
    assert_eq!(swept.len(), 1, "the busy one still never sweeps");
    assert!(!table.is_empty());
}

#[test]
fn validate_tags_trims_dedupes_and_bounds() {
    assert_eq!(
        validate_tags(vec![" hero ".into(), "hero".into(), "map".into()]).unwrap(),
        vec!["hero".to_string(), "map".to_string()]
    );
    assert!(validate_tags(vec!["".into()]).is_err());
    assert!(validate_tags(vec!["x".repeat(MAX_TAG_CHARS + 1)]).is_err());
    assert!(validate_tags(vec!["t".into(); MAX_TAGS + 1]).is_err());
    assert!(validate_tags(vec![]).unwrap().is_empty());
}
