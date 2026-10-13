//! These tests drive the link-preview enrich stage through the ingest path.
//!
//! They call `handle_send_message`/`handle_edit_message` directly, exactly like `chat::tests`,
//! against a real stub `axum` target on `127.0.0.1`. The fixture builds its client through
//! `link_preview::build_client_with_resolve_fn` with a resolver closure that answers every host
//! with the loopback address, so a host name such as `stub.test` reaches the stub. They live in
//! their own `mod` (not folded into `chat::tests`) because the link-preview tests need a stub
//! server and a hyperlink-enabling policy (`hyperlinks_on`) that `chat::tests` does not set up.
//! The same fixture also drives the oEmbed segment tests and the per-channel dice-mode tests.

use super::*;
use crate::auth::role::ServerRole;
use crate::data::document::WorldRole;
use crate::data::sqlite::SqliteRepository;
use crate::ws::room::RoomRegistry;
use axum::routing::get;
use axum::Router;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Spawns a stub HTTP target on a random loopback port, returning the
/// port. Mirrors `link_preview`'s own `spawn_stub` test helper. Callers address it
/// through a domain such as `stub.test` (never a literal `127.0.0.1` URL,
/// which `validate_url` blocks unconditionally regardless of any
/// loopback allowance) — `Fixture::new`'s client resolves every host name to
/// the loopback IP this stub binds.
async fn spawn_stub(router: Router) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    port
}

/// GM + one Player, `chat-settings` seeded with the given `policy` so the
/// enrich stage's gating (`previews_enabled`) is under test control.
struct Fixture {
    repo: SqliteRepository,
    room: std::sync::Arc<Room>,
    ctx: PermissionContext,
    rate: PingRateLimiter,
    preview_client: reqwest::Client,
    preview_cache: LinkPreviewCache,
    preview_rate: PreviewRateLimiter,
}

impl Fixture {
    async fn new(policy: ChatContentPolicy) -> Self {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let player = repo
            .create_user("pl", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        // The send path refuses unregistered channels; "ic" is registered
        // for the per-channel dice-override tests.
        crate::data::world_seed::seed_test_channel_registry(&repo, w.id, &["ic"]).await;
        repo.add_member(w.id, player, WorldRole::Player)
            .await
            .unwrap();
        let gm_ctx = PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        };
        let doc = Document {
            id: Uuid::new_v4(),
            scope: Scope::World { world_id: w.id },
            doc_type: CHAT_SETTINGS_DOC_TYPE.to_string(),
            schema_version: 1,
            name: None,
            source: None,
            base: None,
            owner: Some(gm),
            permissions: PermissionSet::default(),
            embedded: BTreeMap::new(),
            parent_id: None,
            engine: Some(serde_json::to_value(policy).unwrap()),
            system: serde_json::json!({}),
            created_at: 0,
            updated_at: 0,
        };
        repo.apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Create { doc }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        let ctx = PermissionContext {
            user_id: player,
            world_role: WorldRole::Player,
        };
        Fixture {
            repo,
            room,
            ctx,
            rate: PingRateLimiter::new(),
            // Resolves every host name to real loopback, so a domain such as `stub.test`
            // reaches a `127.0.0.1`-bound stub; a literal `127.0.0.1` URL cannot, since
            // `validate_url` rejects that host outright regardless of any loopback allowance
            // (see that fn's doc).
            preview_client: link_preview::build_client_with_resolve_fn(|_host| {
                Ok(vec!["127.0.0.1".parse().unwrap()])
            }),
            preview_cache: LinkPreviewCache::new(),
            preview_rate: PreviewRateLimiter::new(),
        }
    }

    async fn send(&self, content: &str, now: i64) -> Result<Command, SendMessageError> {
        self.send_full(content, now)
            .await
            .map(|(cmd, _pending)| cmd)
    }

    /// Same as `send`, but to an explicit `channel` rather than the
    /// hardcoded `"general"` — needed to exercise per-channel dice-settings
    /// resolution, which `send` alone cannot reach.
    async fn send_channel(
        &self,
        channel: &str,
        content: &str,
        now: i64,
    ) -> Result<Command, SendMessageError> {
        handle_send_message(
            MessageRequestCtx {
                room: &self.room,
                repo: &self.repo,
                ctx: &self.ctx,
                rate: &self.rate,
                vfx_rate: &self.rate,

                preview: LinkPreviewDeps {
                    client: &self.preview_client,
                    cache: &self.preview_cache,
                    rate: &self.preview_rate,
                },
                now,
                budget_per_min: 60,
            },
            channel.into(),
            content.into(),
            None,
            Audience::Public,
        )
        .await
        .map(|opt| opt.expect("a plain message is not a /fx command"))
        .map(|(cmd, _pending)| cmd)
    }

    /// Seeds a `dice-settings` doc with the given `engine` JSON via the
    /// test-only raw insert (`SqliteRepository::seed_document_unvalidated`)
    /// — `Fixture` retains no GM `PermissionContext` to drive a normal
    /// `apply_intent` Create, and a well-formed `channel_overrides` body
    /// doesn't need ingress validation to exercise `handle_send_message`'s
    /// channel-threading. `owner` must be a real created user (an FK), so
    /// this uses the fixture's own player id.
    async fn seed_dice_settings(&self, engine: serde_json::Value) {
        let doc = Document {
            id: Uuid::new_v4(),
            scope: Scope::World {
                world_id: self.room.world_id,
            },
            doc_type: DICE_SETTINGS_DOC_TYPE.to_string(),
            schema_version: 1,
            name: None,
            source: None,
            base: None,
            owner: Some(self.ctx.user_id),
            permissions: PermissionSet::default(),
            embedded: BTreeMap::new(),
            parent_id: None,
            engine: Some(engine),
            system: serde_json::json!({}),
            created_at: 0,
            updated_at: 0,
        };
        self.repo.seed_document_unvalidated(&doc).await.unwrap();
    }

    /// Same as `send`, but returns the full `(Command, Vec<PendingEnrichment>)`
    /// tuple — `send` discards the pending half, which hides whether
    /// `enrich`'s image-candidate queue actually reaches this call's
    /// caller.
    async fn send_full(
        &self,
        content: &str,
        now: i64,
    ) -> Result<(Command, Vec<PendingEnrichment>), SendMessageError> {
        handle_send_message(
            MessageRequestCtx {
                room: &self.room,
                repo: &self.repo,
                ctx: &self.ctx,
                rate: &self.rate,
                vfx_rate: &self.rate,

                preview: LinkPreviewDeps {
                    client: &self.preview_client,
                    cache: &self.preview_cache,
                    rate: &self.preview_rate,
                },
                now,
                budget_per_min: 60,
            },
            "general".into(),
            content.into(),
            None,
            Audience::Public,
        )
        .await
        .map(|opt| opt.expect("a plain message is not a /fx command"))
    }

    async fn edit(
        &self,
        message_id: Uuid,
        content: &str,
        now: i64,
    ) -> Result<Command, SendMessageError> {
        handle_edit_message(
            MessageRequestCtx {
                room: &self.room,
                repo: &self.repo,
                ctx: &self.ctx,
                rate: &self.rate,
                vfx_rate: &self.rate,

                preview: LinkPreviewDeps {
                    client: &self.preview_client,
                    cache: &self.preview_cache,
                    rate: &self.preview_rate,
                },
                now,
                budget_per_min: 60,
            },
            message_id,
            content.into(),
        )
        .await
        .map(|(cmd, _pending)| cmd)
    }

    async fn stored_engine(&self, cmd: &Command) -> MessageEngine {
        let doc_id = match &cmd.ops[0] {
            Operation::Create { doc } => doc.id,
            Operation::Update { doc_id, .. } => *doc_id,
            Operation::Move { doc_id, .. } => *doc_id,
            Operation::Delete { doc } => doc.id,
        };
        let doc = self.repo.get_document(doc_id).await.unwrap().unwrap();
        serde_json::from_value(doc.engine.unwrap()).unwrap()
    }
}

/// `markdown` must be on alongside `hyperlinks` for a plain `http://...`
/// URL to actually become an `<a href>` — `sanitize`'s bare-toggle
/// `hyperlinks` only controls whether ammonia PERMITS the `<a>` tag once
/// produced; producing one at all needs markdown link syntax
/// (`[label](url)`) run through `pulldown-cmark`. Bodies built via `hyperlinks_on`
/// use that syntax accordingly.
fn hyperlinks_on() -> ChatContentPolicy {
    ChatContentPolicy {
        markdown: Some(true),
        hyperlinks: Some(true),
        ..Default::default()
    }
}

#[tokio::test]
async fn normal_message_with_link_gets_trailing_preview() {
    let addr = spawn_stub(Router::new().route(
        "/",
        get(|| async { axum::response::Html("<title>Hello</title>") }),
    ))
    .await;
    let f = Fixture::new(hyperlinks_on()).await;
    let cmd = f
        .send(&format!("check out [link](http://stub.test:{addr}/)"), 1)
        .await
        .unwrap();
    let sys = f.stored_engine(&cmd).await;
    match sys.content.last() {
        Some(Segment::LinkPreview { url, title, .. }) => {
            assert!(url.contains(&addr.to_string()));
            assert_eq!(title, "Hello");
        }
        other => panic!("expected a trailing LinkPreview segment, got {other:?}"),
    }
    // The preview is APPENDED — the original Html run is still first.
    assert!(matches!(sys.content.first(), Some(Segment::Html { .. })));
}

#[tokio::test]
async fn page_with_og_image_yields_a_pending_image_job_and_no_asset_id_yet() {
    let addr = spawn_stub(Router::new().route(
        "/",
        get(|| async {
            axum::response::Html(
                r#"<title>Hello</title><meta property="og:image" content="https://og.example/pic.png">"#,
            )
        }),
    ))
    .await;
    let f = Fixture::new(hyperlinks_on()).await;
    let (cmd, pending) = f
        .send_full(&format!("check out [link](http://stub.test:{addr}/)"), 1)
        .await
        .unwrap();
    assert_eq!(
        pending.len(),
        1,
        "an og:image candidate must queue exactly one PendingEnrichment"
    );
    assert!(matches!(pending[0], PendingEnrichment::PreviewImage { .. }));
    let sys = f.stored_engine(&cmd).await;
    match sys.content.last() {
        Some(Segment::LinkPreview { image_asset_id, .. }) => {
            assert_eq!(
                *image_asset_id, None,
                "the synchronous scrape never asset-ifies the image itself"
            );
        }
        other => panic!("expected a trailing LinkPreview segment, got {other:?}"),
    }
}

#[tokio::test]
async fn redirected_preview_keys_its_image_job_and_cache_row_on_the_posted_candidate() {
    let addr = spawn_stub(
        Router::new()
            .route(
                "/start",
                get(|| async { axum::response::Redirect::to("/final") }),
            )
            .route(
                "/final",
                get(|| async {
                    axum::response::Html(
                        r#"<title>Landed</title><meta property="og:image" content="https://og.example/pic.png">"#,
                    )
                }),
            ),
    )
    .await;
    let f = Fixture::new(hyperlinks_on()).await;
    let candidate = format!("http://stub.test:{addr}/start");
    let (cmd, pending) = f
        .send_full(&format!("check out [link]({candidate})"), 1)
        .await
        .unwrap();
    let PendingEnrichment::PreviewImage {
        cache_urls,
        preview_url,
        ..
    } = &pending[0]
    else {
        panic!("expected a PreviewImage job, got {:?}", pending[0]);
    };
    assert_eq!(*cache_urls, vec![candidate.clone()]);
    assert!(
        preview_url.ends_with("/final"),
        "the segment url is the post-redirect address, got {preview_url}"
    );
    let sys = f.stored_engine(&cmd).await;
    match sys.content.last() {
        Some(Segment::LinkPreview { url, .. }) => assert_eq!(url, preview_url),
        other => panic!("expected a trailing LinkPreview segment, got {other:?}"),
    }
    assert!(
        f.repo
            .get_link_preview_cache(&cache_urls[0])
            .await
            .unwrap()
            .is_some(),
        "the persisted row is keyed on the posted candidate"
    );
    assert_eq!(
        f.repo.get_link_preview_cache(preview_url).await.unwrap(),
        None,
        "no row exists under the post-redirect address"
    );
}

#[tokio::test]
async fn page_without_og_image_yields_no_pending_jobs() {
    let addr = spawn_stub(Router::new().route(
        "/",
        get(|| async { axum::response::Html("<title>Hello</title>") }),
    ))
    .await;
    let f = Fixture::new(hyperlinks_on()).await;
    let (_cmd, pending) = f
        .send_full(&format!("check out [link](http://stub.test:{addr}/)"), 1)
        .await
        .unwrap();
    assert!(
        pending.is_empty(),
        "a page with no og:image/link[rel=image_src] must queue no pending jobs"
    );
}

#[tokio::test]
async fn same_url_twice_hits_the_cache_one_fetch_total() {
    static HITS: AtomicUsize = AtomicUsize::new(0);
    let addr = spawn_stub(Router::new().route(
        "/",
        get(|| async {
            HITS.fetch_add(1, Ordering::SeqCst);
            axum::response::Html("<title>Cached</title>")
        }),
    ))
    .await;
    let f = Fixture::new(hyperlinks_on()).await;
    let url = format!("http://stub.test:{addr}/");
    let first = f.send(&format!("one [x]({url})"), 1).await.unwrap();
    let second = f.send(&format!("two [x]({url})"), 2).await.unwrap();
    for cmd in [&first, &second] {
        let sys = f.stored_engine(cmd).await;
        assert!(matches!(
            sys.content.last(),
            Some(Segment::LinkPreview { .. })
        ));
    }
    assert_eq!(
        HITS.load(Ordering::SeqCst),
        1,
        "second send must hit the cache, not re-fetch"
    );
}

#[tokio::test]
async fn max_previews_per_message_caps_four_links_to_three() {
    let addr = spawn_stub(Router::new().route(
        "/{id}",
        get(|| async { axum::response::Html("<title>P</title>") }),
    ))
    .await;
    let f = Fixture::new(hyperlinks_on()).await;
    let body = format!(
        "[a](http://stub.test:{addr}/a) [b](http://stub.test:{addr}/b) [c](http://stub.test:{addr}/c) [d](http://stub.test:{addr}/d)"
    );
    let cmd = f.send(&body, 1).await.unwrap();
    let sys = f.stored_engine(&cmd).await;
    let preview_count = sys
        .content
        .iter()
        .filter(|s| matches!(s, Segment::LinkPreview { .. }))
        .count();
    assert_eq!(
        preview_count, MAX_PREVIEWS_PER_MESSAGE,
        "got {:?}",
        sys.content
    );
}

#[tokio::test]
async fn failing_fetch_degrades_no_card_message_still_posts() {
    let addr = spawn_stub(Router::new().route(
        "/",
        get(|| async { axum::http::StatusCode::INTERNAL_SERVER_ERROR }),
    ))
    .await;
    let f = Fixture::new(hyperlinks_on()).await;
    let cmd = f
        .send(&format!("broken [link](http://stub.test:{addr}/)"), 1)
        .await
        .unwrap();
    let sys = f.stored_engine(&cmd).await;
    assert!(
        !sys.content
            .iter()
            .any(|s| matches!(s, Segment::LinkPreview { .. })),
        "a failing fetch must not produce a card: {:?}",
        sys.content
    );
    assert!(matches!(sys.content.first(), Some(Segment::Html { .. })));
}

#[tokio::test]
async fn previews_suppressed_when_hyperlinks_off() {
    let addr = spawn_stub(Router::new().route(
        "/",
        get(|| async { axum::response::Html("<title>Nope</title>") }),
    ))
    .await;
    // Default policy: every toggle off, including hyperlinks.
    let f = Fixture::new(ChatContentPolicy::default()).await;
    let cmd = f
        .send(
            &format!("http://stub.test:{addr}/ plain text, no markup"),
            1,
        )
        .await
        .unwrap();
    let sys = f.stored_engine(&cmd).await;
    assert!(
        !sys.content
            .iter()
            .any(|s| matches!(s, Segment::LinkPreview { .. })),
        "hyperlinks off must suppress previews entirely: {:?}",
        sys.content
    );
}

#[tokio::test]
async fn previews_suppressed_when_link_previews_explicitly_off() {
    let addr = spawn_stub(Router::new().route(
        "/",
        get(|| async { axum::response::Html("<title>Nope</title>") }),
    ))
    .await;
    let f = Fixture::new(ChatContentPolicy {
        markdown: Some(true),
        hyperlinks: Some(true),
        link_previews: Some(false),
        ..Default::default()
    })
    .await;
    let cmd = f
        .send(&format!("check [link](http://stub.test:{addr}/)"), 1)
        .await
        .unwrap();
    let sys = f.stored_engine(&cmd).await;
    // Sanity: the link is still rendered as a real anchor — proving the
    // suppression below is `link_previews`-specific, not a side effect
    // of the link never becoming an `<a>` in the first place.
    assert!(matches!(sys.content.first(), Some(Segment::Html { .. })));
    assert!(
        !sys.content
            .iter()
            .any(|s| matches!(s, Segment::LinkPreview { .. })),
        "an explicit link_previews:false must suppress previews: {:?}",
        sys.content
    );
}

#[tokio::test]
async fn burst_of_distinct_urls_hits_the_rate_limit_and_stops_fetching() {
    static HITS: AtomicUsize = AtomicUsize::new(0);
    let addr = spawn_stub(Router::new().route(
        "/{id}",
        get(|| async {
            HITS.fetch_add(1, Ordering::SeqCst);
            axum::response::Html("<title>x</title>")
        }),
    ))
    .await;
    let f = Fixture::new(hyperlinks_on()).await;
    // A per-user PREVIEW_FETCH_PER_MIN budget of 20; drive far more than
    // that many DISTINCT-URL candidates (3 links/message, capped) in the
    // same 60s window so the limiter must reject some fetches outright.
    let mut total_previews = 0usize;
    for i in 0..15u32 {
        let body = format!(
            "[a](http://stub.test:{addr}/{i}a) [b](http://stub.test:{addr}/{i}b) [c](http://stub.test:{addr}/{i}c)"
        );
        let cmd = f.send(&body, 1_000).await.unwrap();
        let sys = f.stored_engine(&cmd).await;
        total_previews += sys
            .content
            .iter()
            .filter(|s| matches!(s, Segment::LinkPreview { .. }))
            .count();
    }
    // 15 messages * 3 distinct URLs each = 45 candidate fetches, but the
    // per-user budget is PREVIEW_FETCH_PER_MIN (20) within the window —
    // every message still posts (enrich degrades, never blocks the
    // send), but strictly fewer previews land than candidate URLs.
    assert!(
        total_previews < 45,
        "the rate limit must have stopped some fetches: {total_previews} previews from 45 candidates"
    );
    assert!(
        HITS.load(Ordering::SeqCst) <= PREVIEW_FETCH_PER_MIN,
        "no more real fetches than the per-user budget: {} hits",
        HITS.load(Ordering::SeqCst)
    );
}

/// A preview is derived, not authored — editing a message's links must
/// re-derive the card set from the NEW content, dropping a stale preview
/// whose link no longer appears.
#[tokio::test]
async fn edit_re_derives_preview_from_new_content() {
    let addr = spawn_stub(Router::new().route(
        "/",
        get(|| async { axum::response::Html("<title>Original</title>") }),
    ))
    .await;
    let f = Fixture::new(hyperlinks_on()).await;
    let sent = f
        .send(&format!("see [link](http://stub.test:{addr}/)"), 1)
        .await
        .unwrap();
    let doc_id = match &sent.ops[0] {
        Operation::Create { doc } => doc.id,
        _ => unreachable!(),
    };
    let sys_before = f.stored_engine(&sent).await;
    assert!(matches!(
        sys_before.content.last(),
        Some(Segment::LinkPreview { .. })
    ));

    let edited = f.edit(doc_id, "no links here anymore", 2).await.unwrap();
    let sys_after = f.stored_engine(&edited).await;
    assert!(
        !sys_after
            .content
            .iter()
            .any(|s| matches!(s, Segment::LinkPreview { .. })),
        "an edit removing the link must drop the stale preview: {:?}",
        sys_after.content
    );
}

/// A `/roll` on a PREVIEW-ENABLED world never accumulates a `LinkPreview`
/// segment: its content is exactly one `RollEmbed`. Pins the
/// EXPLICIT `kind != MessageKind::Roll` enrich guard — a successful roll
/// falls through to the enrich gate (only the roll-execution-FAILURE arm
/// returns early), so the guard, not the incidental absence of `<a href>`
/// in a `RollEmbed`, is what keeps a roll message off the outbound-fetch
/// path if the roll content model ever changes.
#[tokio::test]
async fn roll_message_never_gets_a_link_preview_even_when_previews_enabled() {
    let f = Fixture::new(hyperlinks_on()).await;
    let cmd = f.send("/roll 1d6", 1).await.unwrap();
    let sys = f.stored_engine(&cmd).await;
    assert_eq!(sys.kind, MessageKind::Roll);
    assert_eq!(
        sys.content.len(),
        1,
        "roll content must be one RollEmbed: {:?}",
        sys.content
    );
    assert!(matches!(
        sys.content.first(),
        Some(Segment::RollEmbed { .. })
    ));
    assert!(
        !sys.content
            .iter()
            .any(|s| matches!(s, Segment::LinkPreview { .. })),
        "a roll message must never carry a LinkPreview: {:?}",
        sys.content
    );
}

/// Ingest-level pin: `handle_send_message` resolves the ambient dice
/// context under the SENDING channel, not a hardcoded/ignored one. A
/// bare `t<N>` target (mode-agnostic notation — resolves to
/// `TotalConfig.difficulty` under Total-ambient, or a SuccessCount
/// target under SuccessCount-ambient, per the notation parser's
/// `ParseContext`-driven resolution) sent to "ic" (which carries a
/// SuccessCount channel override) yields `successes: Some(_)`, while the
/// SAME formula sent to "general" (no override, world default Total)
/// yields `successes: None`.
#[tokio::test]
async fn ambient_mode_resolves_per_sending_channel() {
    let f = Fixture::new(ChatContentPolicy::default()).await;
    f.seed_dice_settings(serde_json::json!({
        "mode": "total", "direction": "high_wins",
        "channel_overrides": {
            "ic": { "mode": "success_count", "direction": "high_wins" }
        }
    }))
    .await;

    let ic_cmd = f.send_channel("ic", "/roll 4d6t3", 1).await.unwrap();
    let ic_sys = f.stored_engine(&ic_cmd).await;
    let Segment::RollEmbed {
        outcome: ic_outcome,
        ..
    } = ic_sys.content.first().unwrap()
    else {
        panic!("expected a RollEmbed segment: {:?}", ic_sys.content);
    };
    assert!(
        ic_outcome.successes.is_some(),
        "channel \"ic\" carries a SuccessCount override, so a bare t<N> \
         target must resolve as a success count: {ic_outcome:?}"
    );

    let general_cmd = f.send_channel("general", "/roll 4d6t3", 2).await.unwrap();
    let general_sys = f.stored_engine(&general_cmd).await;
    let Segment::RollEmbed {
        outcome: general_outcome,
        ..
    } = general_sys.content.first().unwrap()
    else {
        panic!("expected a RollEmbed segment: {:?}", general_sys.content);
    };
    assert!(
        general_outcome.successes.is_none(),
        "channel \"general\" carries no override, so it must fall back \
         to the Total world default: {general_outcome:?}"
    );
}

/// An explicit `cs>=N` (or `t<N>`) notation override already forces
/// `SuccessCount` regardless of the AMBIENT resolved dice-settings —
/// per-message overrides are fully satisfied by existing parser
/// precedence, needing no new plumbing. This re-asserts that precedence
/// now that ambient resolution is per-channel, not just per-world: an
/// explicit `cs>=3` still wins even though the sending channel's own
/// override says Total.
#[tokio::test]
async fn inline_success_rule_notation_forces_success_count_despite_a_total_channel_override() {
    let f = Fixture::new(ChatContentPolicy::default()).await;
    f.seed_dice_settings(serde_json::json!({
        "mode": "total", "direction": "high_wins",
        "channel_overrides": {
            "ic": { "mode": "total", "direction": "high_wins" }
        }
    }))
    .await;

    let cmd = f.send_channel("ic", "/roll 4d6cs>=3", 1).await.unwrap();
    let sys = f.stored_engine(&cmd).await;
    assert_eq!(sys.kind, MessageKind::Roll);
    let Segment::RollEmbed { outcome, .. } = sys.content.first().unwrap() else {
        panic!("expected a RollEmbed segment: {:?}", sys.content);
    };
    assert!(
        outcome.successes.is_some(),
        "explicit cs>=N notation must force SuccessCount (successes \
         populated) regardless of the channel's Total-mode ambient \
         override: {outcome:?}"
    );
}

/// SECURITY: inert body text carrying a literal `href="..."` substring —
/// NOT a markdown link, NOT inside an `<a>` tag — must never trigger a
/// fetch. Markdown body text does not escape `"`/`'`, so this prose
/// renders through `ammonia` unchanged as plain text; extraction must
/// stay scoped to a genuine `<a>` tag span (see `extract_href_urls`'s
/// doc), never a raw `href=` substring match anywhere in the run, or the
/// server would fetch an attacker-chosen URL from invisible, non-
/// hyperlink text. Proven at the ingest level (not just unit-level on
/// `extract_href_urls`) via a call-counter stub: zero hits.
#[tokio::test]
async fn inert_href_substring_in_body_text_yields_no_preview_and_no_fetch() {
    static HITS: AtomicUsize = AtomicUsize::new(0);
    let addr = spawn_stub(Router::new().route(
        "/x",
        get(|| async {
            HITS.fetch_add(1, Ordering::SeqCst);
            axum::response::Html("<title>should never be fetched</title>")
        }),
    ))
    .await;
    let f = Fixture::new(hyperlinks_on()).await;
    let cmd = f
        .send(
            &format!("see href=\"http://stub.test:{addr}/x\" for details, no markdown link here"),
            1,
        )
        .await
        .unwrap();
    let sys = f.stored_engine(&cmd).await;
    assert!(matches!(sys.content.first(), Some(Segment::Html { .. })));
    assert!(
        !sys.content
            .iter()
            .any(|s| matches!(s, Segment::LinkPreview { .. })),
        "inert body text with a literal href=\"...\" substring must not yield a preview: {:?}",
        sys.content
    );
    assert_eq!(
        HITS.load(Ordering::SeqCst),
        0,
        "the stub must never be hit for a non-anchor href substring"
    );
}

/// A stored `MessageEngine` with no `LinkPreview` segments still
/// round-trips through the deserializer — the `LinkPreview` segment
/// variant is purely additive, not a breaking schema change.
#[test]
fn stored_message_without_link_preview_segments_still_deserializes() {
    let j = serde_json::json!({
        "channel": "all",
        "user_owner": crate::test_ids::test_uuid(1),
        "kind": "normal",
        "audience": { "kind": "public" },
        "content": [{ "kind": "text", "text": "hi" }],
    });
    let sys: MessageEngine = serde_json::from_value(j).unwrap();
    assert_eq!(sys.kind, MessageKind::Normal);
    assert_eq!(sys.content, vec![Segment::Text { text: "hi".into() }]);
}

/// A URL matching the oEmbed allowlist is routed to a
/// `PendingEnrichment::OEmbed` job and never becomes a generic
/// `Segment::LinkPreview`. The endpoint host itself
/// (`www.youtube.com`) is a fixed real host, so this exercises
/// `match_provider`/`PendingEnrichment` construction only -- no live
/// fetch happens on the synchronous send path either way.
#[tokio::test]
async fn oembed_allowlisted_url_produces_oembed_segment_not_link_preview() {
    let f = Fixture::new(hyperlinks_on()).await;
    let (cmd, pending) = f
        .send_full(
            "check out [video](https://www.youtube.com/watch?v=abc123)",
            1,
        )
        .await
        .unwrap();
    assert_eq!(
        pending.len(),
        1,
        "an allowlisted URL must queue exactly one PendingEnrichment"
    );
    assert!(
        matches!(pending[0], PendingEnrichment::OEmbed { .. }),
        "expected a PendingEnrichment::OEmbed, got {:?}",
        pending[0]
    );
    let sys = f.stored_engine(&cmd).await;
    assert!(
        !sys.content
            .iter()
            .any(|s| matches!(s, Segment::LinkPreview { .. })),
        "an allowlisted oEmbed URL must never also produce a generic LinkPreview: {:?}",
        sys.content
    );
}

/// A message carrying BOTH an allowlisted oEmbed URL and a
/// non-allowlisted URL routes each independently: one
/// `PendingEnrichment::OEmbed` for the allowlisted URL, and the
/// ordinary synchronous generic-preview flow (a trailing
/// `Segment::LinkPreview`) for the other, unaffected.
#[tokio::test]
async fn oembed_and_generic_preview_urls_in_one_message_both_queue_correctly() {
    let addr = spawn_stub(Router::new().route(
        "/",
        get(|| async { axum::response::Html("<title>Generic</title>") }),
    ))
    .await;
    let f = Fixture::new(hyperlinks_on()).await;
    let (cmd, pending) = f
        .send_full(
            &format!(
                "[video](https://www.youtube.com/watch?v=abc123) and [page](http://stub.test:{addr}/)"
            ),
            1,
        )
        .await
        .unwrap();
    assert_eq!(
        pending.len(),
        1,
        "only the allowlisted URL queues a PendingEnrichment (the generic preview fetch is synchronous)"
    );
    assert!(matches!(pending[0], PendingEnrichment::OEmbed { .. }));
    let sys = f.stored_engine(&cmd).await;
    let generic_preview = sys.content.iter().find_map(|s| match s {
        Segment::LinkPreview { url, title, .. } => Some((url.clone(), title.clone())),
        _ => None,
    });
    assert!(
        generic_preview.is_some(),
        "the non-allowlisted URL must still produce its own generic LinkPreview: {:?}",
        sys.content
    );
    let (url, title) = generic_preview.unwrap();
    assert!(url.contains(&addr.to_string()));
    assert_eq!(title, "Generic");
}

#[tokio::test]
async fn unnormalized_candidate_without_a_redirect_keys_its_job_on_the_candidate() {
    let addr = spawn_stub(Router::new().route(
        "/",
        get(|| async {
            axum::response::Html(
                r#"<title>Bare</title><meta property="og:image" content="https://og.example/pic.png">"#,
            )
        }),
    ))
    .await;
    let f = Fixture::new(hyperlinks_on()).await;
    let candidate = format!("http://stub.test:{addr}");
    let (cmd, pending) = f
        .send_full(&format!("check out [link]({candidate})"), 1)
        .await
        .unwrap();
    let PendingEnrichment::PreviewImage {
        cache_urls,
        preview_url,
        ..
    } = &pending[0]
    else {
        panic!("expected a PreviewImage job, got {:?}", pending[0]);
    };
    assert_eq!(*cache_urls, vec![candidate.clone()]);
    assert_eq!(
        *preview_url,
        format!("{candidate}/"),
        "the fetch reports the normalized address, which gains the root path"
    );
    let sys = f.stored_engine(&cmd).await;
    match sys.content.last() {
        Some(Segment::LinkPreview { url, .. }) => assert_eq!(url, preview_url),
        other => panic!("expected a trailing LinkPreview segment, got {other:?}"),
    }
    assert!(
        f.repo
            .get_link_preview_cache(&candidate)
            .await
            .unwrap()
            .is_some(),
        "the persisted row is keyed on the candidate as posted"
    );
    assert_eq!(
        f.repo.get_link_preview_cache(preview_url).await.unwrap(),
        None,
        "no row exists under the normalized address"
    );
}

#[tokio::test]
async fn two_candidates_redirecting_to_one_page_share_one_image_job() {
    let addr = spawn_stub(
        Router::new()
            .route(
                "/a",
                get(|| async {
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    axum::response::Redirect::to("/final")
                }),
            )
            .route(
                "/b",
                get(|| async { axum::response::Redirect::to("/final") }),
            )
            .route(
                "/final",
                get(|| async {
                    axum::response::Html(
                        r#"<title>Landed</title><meta property="og:image" content="https://og.example/pic.png">"#,
                    )
                }),
            ),
    )
    .await;
    let f = Fixture::new(hyperlinks_on()).await;
    let first = format!("http://stub.test:{addr}/a");
    let second = format!("http://stub.test:{addr}/b");
    let (cmd, pending) = f
        .send_full(&format!("see [one]({first}) and [two]({second})"), 1)
        .await
        .unwrap();
    assert_eq!(
        pending.len(),
        1,
        "candidates ending on one address share a job"
    );
    let PendingEnrichment::PreviewImage {
        cache_urls,
        preview_url,
        ..
    } = &pending[0]
    else {
        panic!("expected a PreviewImage job, got {:?}", pending[0]);
    };
    assert_eq!(
        *cache_urls,
        vec![first.clone(), second.clone()],
        "message order, though the second candidate's fetch completes first"
    );
    let sys = f.stored_engine(&cmd).await;
    let urls: Vec<&String> = sys
        .content
        .iter()
        .filter_map(|seg| match seg {
            Segment::LinkPreview { url, .. } => Some(url),
            _ => None,
        })
        .collect();
    assert_eq!(urls, vec![preview_url, preview_url]);
    for candidate in [&first, &second] {
        assert!(
            f.repo
                .get_link_preview_cache(candidate)
                .await
                .unwrap()
                .is_some(),
            "the persisted row of {candidate} is keyed on the candidate"
        );
    }
}

#[tokio::test]
async fn fetched_previews_append_in_message_order_not_completion_order() {
    let addr = spawn_stub(
        Router::new()
            .route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    axum::response::Html("<title>Slow</title>")
                }),
            )
            .route(
                "/fast",
                get(|| async { axum::response::Html("<title>Fast</title>") }),
            ),
    )
    .await;
    let f = Fixture::new(hyperlinks_on()).await;
    let slow = format!("http://stub.test:{addr}/slow");
    let fast = format!("http://stub.test:{addr}/fast");
    let (cmd, _pending) = f
        .send_full(&format!("see [one]({slow}) and [two]({fast})"), 1)
        .await
        .unwrap();
    let sys = f.stored_engine(&cmd).await;
    let titles: Vec<&str> = sys
        .content
        .iter()
        .filter_map(|seg| match seg {
            Segment::LinkPreview { title, .. } => Some(title.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(titles, vec!["Slow", "Fast"]);
}

/// Seeds an asset row and a fresh persisted cache row for `known_url` carrying that asset, so the
/// url resolves from the persisted tier with an `image_asset_id`.
async fn seed_imaged_row(f: &Fixture, known_url: &str) -> Uuid {
    let asset = crate::data::asset::Asset {
        id: Uuid::new_v4(),
        world_id: f.room.world_id,
        storage_key: "k".into(),
        original_name: "og.png".into(),
        content_type: "image/png".into(),
        byte_size: 1,
        created_by: None,
        created_at: 0,
        version: 1,
        folder_id: None,
        tags: vec![],
        derived_tags: vec![],
        meta: crate::data::asset::AssetMeta::unprocessed("image/png", 1),
    };
    f.repo.insert_asset(&asset).await.unwrap();
    f.repo
        .upsert_link_preview_cache(known_url, Some("Landed"), Some("D"), 0)
        .await
        .unwrap();
    f.repo
        .set_link_preview_cache_image(known_url, asset.id)
        .await
        .unwrap();
    asset.id
}

/// A persisted-row candidate that already carries an asset and a fresh candidate whose fetch
/// ends on the persisted candidate's address are the same preview: no job is queued, both
/// segments carry the known asset, and the fresh candidate's row records it, in either message
/// order.
#[tokio::test]
async fn fresh_candidate_ending_on_a_persisted_imaged_address_reuses_its_asset() {
    for known_first in [true, false] {
        let addr = spawn_stub(
            Router::new()
                .route(
                    "/q",
                    get(|| async { axum::response::Redirect::to("/final") }),
                )
                .route(
                    "/final",
                    get(|| async {
                        axum::response::Html(
                            r#"<title>Landed</title><meta property="og:image" content="https://og.example/pic.png">"#,
                        )
                    }),
                ),
        )
        .await;
        let f = Fixture::new(hyperlinks_on()).await;
        let known = format!("http://stub.test:{addr}/final");
        let fresh = format!("http://stub.test:{addr}/q");
        let asset_id = seed_imaged_row(&f, &known).await;
        let body = if known_first {
            format!("see [one]({known}) and [two]({fresh})")
        } else {
            format!("see [two]({fresh}) and [one]({known})")
        };
        let (cmd, pending) = f.send_full(&body, 1).await.unwrap();
        assert!(
            pending.is_empty(),
            "no image job when the address already has an asset: {pending:?}"
        );
        let sys = f.stored_engine(&cmd).await;
        let ids: Vec<Option<Uuid>> = sys
            .content
            .iter()
            .filter_map(|seg| match seg {
                Segment::LinkPreview { image_asset_id, .. } => Some(*image_asset_id),
                _ => None,
            })
            .collect();
        assert_eq!(ids, vec![Some(asset_id), Some(asset_id)]);
        let row = f
            .repo
            .get_link_preview_cache(&fresh)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.image_asset_id, Some(asset_id));
    }
}
