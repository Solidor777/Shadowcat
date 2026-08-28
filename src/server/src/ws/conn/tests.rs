use super::*;
use serde_json::json;

// Dual-write fixture helpers (`ws_engine`/`token_engine`) live in `ws::test_support`,
// shared with `ws::room`'s test module.
use crate::ws::test_support::{token_engine, ws_engine};

/// Deterministic broadcast-`Lagged` → resync guard, driven directly against the
/// generic `egress_loop` with a credit-gated in-process sink — no real socket, so
/// it does not depend on any OS's TCP buffer sizing (a socket-backpressure approach
/// is non-portable: `SO_SNDBUF`/`SO_RCVBUF` are advisory and each OS
/// clamps/autotunes them differently). The sink starts with exactly one credit
/// (consumed by `Welcome`); with zero credits the egress drains at most one
/// broadcast event before parking on the gated send, so publishing
/// `30 >> capacity(8)` events overflows the broadcast ring deterministically.
/// Granting credits unblocks the egress, which then observes `Lagged`, replays
/// from the ring/log, and converges to the authoritative tail with no gaps/dups.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn egress_lag_triggers_resync_and_converges() {
    use crate::data::command::Operation;
    use crate::data::document::WorldRole;
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::sync::Semaphore;

    // A `Sink<Message>` whose readiness is gated by a semaphore credit; accepted
    // frames are forwarded to an unbounded channel the test drains. Each send
    // consumes one credit (the permit is `forget`-ten), so the test controls
    // exactly how many frames the egress may emit, and thus when it stalls.
    struct GatedSink {
        out: mpsc::UnboundedSender<Message>,
        credits: Arc<Semaphore>,
        acquiring: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    }
    impl Sink<Message> for GatedSink {
        type Error = ();
        fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), ()>> {
            let this = self.as_mut().get_mut();
            if this.acquiring.is_none() {
                let sem = this.credits.clone();
                // The semaphore never closes in-test, so the acquire cannot fail.
                this.acquiring = Some(Box::pin(async move {
                    sem.acquire_owned().await.unwrap().forget()
                }));
            }
            match this.acquiring.as_mut().unwrap().as_mut().poll(cx) {
                Poll::Ready(()) => {
                    this.acquiring = None;
                    Poll::Ready(Ok(()))
                }
                Poll::Pending => Poll::Pending,
            }
        }
        fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), ()> {
            let _ = self.get_mut().out.send(item);
            Ok(())
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn msg_text(m: &Message) -> &str {
        match m {
            Message::Text(t) => t.as_str(),
            _ => "",
        }
    }

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let author = repo
        .create_user("a", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", author, 0).await.unwrap();
    let ctx = PermissionContext {
        user_id: author,
        world_role: WorldRole::Gm,
    };

    // Ring capacity 8: fewer than the 30 events published while the egress is gated.
    let reg = crate::ws::room::RoomRegistry::with_capacity(8);
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let (rx, current_seq) = room.subscribe();

    let credits = Arc::new(Semaphore::new(1)); // one credit: the `Welcome` send
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
    // Held open so the egress never sees its ingress channel close mid-test.
    let (etx, erx) = mpsc::channel::<Egress>(8);
    let sink = GatedSink {
        out: out_tx,
        credits: credits.clone(),
        acquiring: None,
    };
    let egress = tokio::spawn(egress_loop(
        sink,
        rx,
        erx,
        EgressConnState {
            room: room.clone(),
            repo: repo.clone(),
            ctx,
            current_seq,
            modules_dir: std::path::PathBuf::from(
                "nonexistent-test-modules-dir-for-egress-lag-test",
            ),
            module_scan_cache: Arc::new(crate::modules::ModuleScanCache::new()),
        },
    ));

    // Drain the `Welcome` (consumes the sole credit); the egress now has zero
    // credits, so it can pull at most one broadcast event before parking.
    let welcome = tokio::time::timeout(std::time::Duration::from_secs(5), out_rx.recv())
        .await
        .expect("egress did not emit Welcome")
        .expect("egress sink closed before Welcome");
    let wv: serde_json::Value = serde_json::from_str(msg_text(&welcome)).unwrap();
    assert_eq!(wv["type"], "welcome");

    // Publish 30 world docs. With the egress gated, far more than capacity(8)
    // accumulate unread in the broadcast ring and overflow it.
    for n in 0..30u128 {
        let mut doc = crate::data::document::tests::world_scoped_doc(
            world.id,
            Uuid::from_u128(1000 + n),
            "actor",
        );
        doc.owner = Some(author);
        room.publish(
            repo.as_ref(),
            &ctx,
            vec![Operation::Create { doc }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    }

    // Release the gate; the egress completes its pending send, then observes
    // `Lagged` on the next `recv` and resyncs from the ring/log.
    credits.add_permits(10_000);

    // Convergence: collected Event seqs reach the authoritative tail (30).
    let mut seqs = vec![];
    while seqs.last().copied() != Some(30) {
        let m = tokio::time::timeout(std::time::Duration::from_secs(5), out_rx.recv())
            .await
            .expect("egress stalled before converging")
            .expect("egress sink closed before converging");
        let v: serde_json::Value = serde_json::from_str(msg_text(&m)).unwrap();
        if v["type"] == "event" {
            seqs.push(v["command"]["seq"].as_i64().unwrap());
        }
    }

    // The lag path fired deterministically (the regression guard a larger ring
    // could not provide).
    assert!(
        room.stats.lagged_drops.load(Ordering::Relaxed) > 0,
        "the lag-driven resync path must fire deterministically"
    );
    assert_eq!(*seqs.last().unwrap(), 30);
    let mut sorted = seqs.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(seqs, sorted, "no duplicates or reordering after resync");

    drop(etx);
    let _ = egress.await;
}

#[tokio::test]
async fn welcome_unions_enabled_modules_requirements_with_gm_authored_ones() {
    use crate::data::document::CapabilityRequirement;

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("actors-plus")).unwrap();
    std::fs::write(
        dir.path().join("actors-plus").join("module.json"),
        format!(
            r#"{{"id":"actors-plus","version":"1.0.0","engines":{{"shadowcat":"^{}"}},"requirements":[{{"path_prefix":"/system/plus","caps":["plus:write"]}}]}}"#,
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", gm, 0).await.unwrap();
    // A GM-authored requirement, unrelated to any module.
    repo.set_world_cap_requirements(
        world.id,
        &[CapabilityRequirement {
            path_prefix: "/system/vision".into(),
            caps: ["dnd5e:gm_vision".to_string()].into_iter().collect(),
        }],
    )
    .await
    .unwrap();

    // With nothing enabled, only the GM-authored requirement is published.
    let reqs = welcome_capability_requirements(
        repo.as_ref(),
        world.id,
        dir.path(),
        &Arc::new(crate::modules::ModuleScanCache::new()),
    )
    .await;
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].path_prefix, "/system/vision");

    // Enabling the module adds its requirement WITHOUT removing the GM's own.
    repo.set_world_enabled_modules(world.id, &["actors-plus".to_string()])
        .await
        .unwrap();
    let reqs = welcome_capability_requirements(
        repo.as_ref(),
        world.id,
        dir.path(),
        &Arc::new(crate::modules::ModuleScanCache::new()),
    )
    .await;
    assert_eq!(reqs.len(), 2);
    assert!(reqs.iter().any(|r| r.path_prefix == "/system/vision"));
    assert!(reqs.iter().any(|r| r.path_prefix == "/system/plus"));

    // world_cap_requirements itself is never mutated by this — the raw GM
    // record still holds exactly its one original entry.
    assert_eq!(
        repo.world_cap_requirements(world.id).await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn welcome_capability_requirements_unions_caps_for_the_same_path_prefix() {
    use crate::data::document::CapabilityRequirement;
    use std::collections::BTreeSet;

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("scene-mod")).unwrap();
    std::fs::write(
        dir.path().join("scene-mod").join("module.json"),
        format!(
            r#"{{"id":"scene-mod","version":"1.0.0","engines":{{"shadowcat":"^{}"}},"requirements":[{{"path_prefix":"/scene","caps":["write"]}}]}}"#,
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", gm, 0).await.unwrap();
    // A GM-authored requirement on "/scene" requiring cap "read", plus a
    // module declaring a requirement on the SAME "/scene" prefix requiring
    // "write".
    repo.set_world_cap_requirements(
        world.id,
        &[CapabilityRequirement {
            path_prefix: "/scene".into(),
            caps: ["read".to_string()].into_iter().collect(),
        }],
    )
    .await
    .unwrap();
    repo.set_world_enabled_modules(world.id, &["scene-mod".to_string()])
        .await
        .unwrap();

    let reqs = welcome_capability_requirements(
        repo.as_ref(),
        world.id,
        dir.path(),
        &Arc::new(crate::modules::ModuleScanCache::new()),
    )
    .await;

    let scene_reqs: Vec<_> = reqs.iter().filter(|r| r.path_prefix == "/scene").collect();
    assert_eq!(
        scene_reqs.len(),
        1,
        "must not emit two entries for the same path_prefix"
    );
    assert_eq!(
        scene_reqs[0].caps,
        BTreeSet::from(["read".to_string(), "write".to_string()]),
        "caps from both sources must be unioned, not one dropped"
    );
}

/// A module that is enabled but whose on-disk manifest
/// declares an engine range the RUNNING server no longer satisfies (a version
/// downgrade, or a manifest edited after enable) must NOT publish its
/// requirements into the advisory Welcome union — mirroring the enable-time
/// `engine_compat_ok` gate in `module_routes::set_world_enabled_modules`: engine
/// compatibility is enforced both at enable time and again on every Welcome load,
/// not just once. Simulated by storing the id directly via `set_world_enabled_modules`
/// (bypassing the HTTP enable-time gate) against a manifest declaring `^99.0.0`.
#[tokio::test]
async fn welcome_excludes_requirements_from_an_enabled_but_now_incompatible_module() {
    use crate::data::document::CapabilityRequirement;

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("stale-mod")).unwrap();
    std::fs::write(
        dir.path().join("stale-mod").join("module.json"),
        r#"{"id":"stale-mod","version":"1.0.0","engines":{"shadowcat":"^99.0.0"},"requirements":[{"path_prefix":"/system/stale","caps":["stale:write"]}]}"#,
    )
    .unwrap();

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.set_world_cap_requirements(
        world.id,
        &[CapabilityRequirement {
            path_prefix: "/system/vision".into(),
            caps: ["dnd5e:gm_vision".to_string()].into_iter().collect(),
        }],
    )
    .await
    .unwrap();

    // Enabled directly at the repo layer (bypassing the HTTP enable-time
    // engine-compat gate), simulating a module that was compatible at enable
    // time but no longer is (server downgrade / manifest edit).
    repo.set_world_enabled_modules(world.id, &["stale-mod".to_string()])
        .await
        .unwrap();

    let reqs = welcome_capability_requirements(
        repo.as_ref(),
        world.id,
        dir.path(),
        &Arc::new(crate::modules::ModuleScanCache::new()),
    )
    .await;
    assert_eq!(
        reqs.len(),
        1,
        "an incompatible enabled module must not contribute requirements"
    );
    assert_eq!(reqs[0].path_prefix, "/system/vision");
    assert!(!reqs.iter().any(|r| r.path_prefix == "/system/stale"));
}

/// Behavior-preservation test for the `spawn_blocking` wrap around
/// `scan_installed_modules` in `welcome_capability_requirements`: proves the
/// Welcome path still resolves an enabled module's declared requirements
/// correctly when the scan runs off the async worker thread. Not a
/// red/green blocking-detection test (blocking-vs-non-blocking isn't
/// directly unit-testable) — mirrors
/// `welcome_unions_enabled_modules_requirements_with_gm_authored_ones`'s
/// setup, verifying the `spawn_blocking`-wrapped path yields results
/// identical to the direct (non-blocking) path.
#[tokio::test]
async fn welcome_capability_requirements_still_resolves_module_requirements_via_spawn_blocking() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("blocking-mod")).unwrap();
    std::fs::write(
        dir.path().join("blocking-mod").join("module.json"),
        format!(
            r#"{{"id":"blocking-mod","version":"1.0.0","engines":{{"shadowcat":"^{}"}},"requirements":[{{"path_prefix":"/system/blocking","caps":["blocking:write"]}}]}}"#,
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.set_world_enabled_modules(world.id, &["blocking-mod".to_string()])
        .await
        .unwrap();

    let reqs = welcome_capability_requirements(
        repo.as_ref(),
        world.id,
        dir.path(),
        &Arc::new(crate::modules::ModuleScanCache::new()),
    )
    .await;
    assert!(
        reqs.iter().any(|r| r.path_prefix == "/system/blocking"),
        "module-declared requirements must still resolve correctly when scan runs via spawn_blocking"
    );
}

/// Wiring regression: `welcome_capability_requirements` reuses ONE shared
/// `ModuleScanCache` across two calls, with an in-place `module.json` edit
/// in between (same shape as
/// `modules::tests::module_scan_cache_detects_an_in_place_manifest_edit`).
/// Proves the Welcome path routes through `ModuleScanCache::get_or_scan`
/// in a way that does NOT reintroduce staleness for the guarantee this
/// function's own doc comment already makes ("a module that has gone
/// incompatible since being enabled... stops contributing") — it does
/// not, by itself, prove caching happened at all (see
/// `modules::tests::module_scan_cache_detects_an_in_place_manifest_edit`
/// for that).
#[tokio::test]
async fn welcome_capability_requirements_reflects_an_in_place_manifest_edit_through_a_shared_cache()
{
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("editable-mod")).unwrap();
    std::fs::write(
        dir.path().join("editable-mod").join("module.json"),
        format!(
            r#"{{"id":"editable-mod","version":"1.0.0","engines":{{"shadowcat":"^{}"}},"requirements":[{{"path_prefix":"/system/editable","caps":["v1:write"]}}]}}"#,
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.set_world_enabled_modules(world.id, &["editable-mod".to_string()])
        .await
        .unwrap();

    let cache = Arc::new(crate::modules::ModuleScanCache::new());
    let reqs1 = welcome_capability_requirements(repo.as_ref(), world.id, dir.path(), &cache).await;
    assert!(reqs1.iter().any(|r| r.path_prefix == "/system/editable"));
    assert!(!reqs1
        .iter()
        .any(|r| r.caps.contains("v2:write") && r.path_prefix == "/system/editable"));

    // In-place edit: same folder, no add/remove under `dir`, so the parent
    // directory's own mtime does not change — the manifest's own mtime is
    // the only signal this edit produces.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(
        dir.path().join("editable-mod").join("module.json"),
        format!(
            r#"{{"id":"editable-mod","version":"1.0.0","engines":{{"shadowcat":"^{}"}},"requirements":[{{"path_prefix":"/system/editable","caps":["v2:write"]}}]}}"#,
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();

    let reqs2 = welcome_capability_requirements(repo.as_ref(), world.id, dir.path(), &cache).await;
    let editable: Vec<_> = reqs2
        .iter()
        .filter(|r| r.path_prefix == "/system/editable")
        .collect();
    assert_eq!(editable.len(), 1);
    assert!(
        editable[0].caps.contains("v2:write"),
        "the second call must reflect the edited manifest, not a stale cached scan"
    );
    assert!(!editable[0].caps.contains("v1:write"));
}

/// Build the square `GridShape` companion map the production `enrich_vision_explored` captures
/// via `SceneEcs::scene_grid_shapes` — one `SquareGrid` per scene at its cell size, so a
/// square-grid test indexes explored fog byte-identically to the production path.
fn square_grid_shapes(
    grid: &std::collections::HashMap<Uuid, f64>,
) -> std::collections::HashMap<Uuid, Box<dyn crate::scene::grid_shape::GridShape + Send + Sync>> {
    grid.iter()
        .map(|(&scene, &cell)| {
            (
                scene,
                Box::new(crate::scene::grid_shape::SquareGrid {
                    cell,
                    rule: crate::scene::pathfinding::DiagonalRule::Chebyshev,
                }) as Box<dyn crate::scene::grid_shape::GridShape + Send + Sync>,
            )
        })
        .collect()
}

/// The dispatch-layer accumulation: a masked vision payload grows + persists the player's
/// explored fog and gains a scene-tagged `explored` set; a revisit re-emits without growing; a
/// GM `mode:"all"` payload is untouched (no fog → no explored).
#[tokio::test]
async fn enrich_accumulates_persists_and_emits_explored() {
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let world = Uuid::from_u128(1);
    let scene = Uuid::from_u128(10);
    let user = Uuid::from_u128(20);
    let grid = std::collections::HashMap::from([(scene, 100.0)]);
    let grid_shapes = square_grid_shapes(&grid);

    // A masked payload with a visibility polygon covering a 3×3 cell block in `scene`.
    let mut payload = json!({
        "mode": "masked",
        "polygons": [{ "scene": scene, "points": [0.0, 0.0, 300.0, 0.0, 300.0, 300.0, 0.0, 300.0] }]
    });
    enrich_vision_explored(&mut payload, &grid, &grid_shapes, &repo, world, user, true).await;

    // The payload gained a scene-tagged explored cell set (9 cells × 2 coords).
    let explored = payload["explored"].as_array().unwrap();
    assert_eq!(explored.len(), 1);
    assert_eq!(explored[0]["scene"], json!(scene));
    assert_eq!(explored[0]["cell"], json!(100.0));
    assert_eq!(explored[0]["cells"].as_array().unwrap().len(), 9 * 2);

    // It persisted: a fresh read returns the same 9 cells.
    let stored = crate::scene::explored::ExploredSet::from_bytes(
        &repo.get_explored(scene, user).await.unwrap().unwrap(),
        crate::scene::GridKind::Square,
    );
    assert_eq!(stored.len(), 9);

    // A revisit of the same area re-emits the same explored without growing the stored set.
    let mut again = json!({
        "mode": "masked",
        "polygons": [{ "scene": scene, "points": [0.0, 0.0, 300.0, 0.0, 300.0, 300.0, 0.0, 300.0] }]
    });
    enrich_vision_explored(&mut again, &grid, &grid_shapes, &repo, world, user, true).await;
    assert_eq!(
        again["explored"][0]["cells"].as_array().unwrap().len(),
        9 * 2
    );
    assert_eq!(
        crate::scene::explored::ExploredSet::from_bytes(
            &repo.get_explored(scene, user).await.unwrap().unwrap(),
            crate::scene::GridKind::Square,
        )
        .len(),
        9,
        "revisiting adds no cells"
    );

    // A GM payload (no fog) is left untouched — no explored memory.
    let mut gm = json!({ "mode": "all" });
    enrich_vision_explored(&mut gm, &grid, &grid_shapes, &repo, world, user, true).await;
    assert_eq!(gm, json!({ "mode": "all" }));
}

/// A scene absent from BOTH grid maps has no live scene document — `enrich_vision_explored`
/// must skip it (fail closed), never synthesize a fallback square grid to index against.
#[tokio::test]
async fn enrich_skips_scene_absent_from_grid_maps() {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let world = Uuid::from_u128(1);
    let user = Uuid::from_u128(2);
    let ghost = Uuid::from_u128(0xDEAD);
    let grid: std::collections::HashMap<Uuid, f64> = std::collections::HashMap::new();
    let shapes = square_grid_shapes(&grid);

    let mut payload = json!({
        "mode": "masked",
        "polygons": [{ "scene": ghost, "points": [0.0, 0.0, 200.0, 0.0, 200.0, 200.0] }]
    });
    enrich_vision_explored(
        &mut payload,
        &grid,
        &shapes,
        repo.as_ref(),
        world,
        user,
        true,
    )
    .await;

    let explored = payload.get("explored").and_then(|e| e.as_array());
    assert!(
        explored.map(|a| a.is_empty()).unwrap_or(true),
        "no explored entry for a scene with no grid entry"
    );
}

/// A GM see-as-player view (`accumulate = false`) emits the target's stored explored but is a
/// read-only observer: it never grows the target's persisted memory from the GM's session.
#[tokio::test]
async fn enrich_see_as_player_is_read_only() {
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let world = Uuid::from_u128(1);
    let scene = Uuid::from_u128(10);
    let target = Uuid::from_u128(20);
    let grid = std::collections::HashMap::from([(scene, 100.0)]);
    let grid_shapes = square_grid_shapes(&grid);

    // Seed the target with one explored cell (as if they'd been there).
    let mut seed = crate::scene::explored::ExploredSet::new();
    seed.mark_polygons(
        &[vec![0.0, 0.0, 100.0, 0.0, 100.0, 100.0, 0.0, 100.0]],
        &crate::scene::grid_shape::SquareGrid {
            cell: 100.0,
            rule: crate::scene::pathfinding::DiagonalRule::Chebyshev,
        },
        100.0,
    );
    repo.set_explored(
        world,
        scene,
        target,
        &seed.to_bytes(crate::scene::GridKind::Square),
    )
    .await
    .unwrap();

    // The GM views as the target over a polygon covering a 3×3 block (would mark 9 cells if it
    // accumulated). Read-only: emits the stored 1 cell, persists nothing new.
    let mut payload = json!({
        "mode": "masked",
        "polygons": [{ "scene": scene, "points": [0.0, 0.0, 300.0, 0.0, 300.0, 300.0, 0.0, 300.0] }]
    });
    enrich_vision_explored(
        &mut payload,
        &grid,
        &grid_shapes,
        &repo,
        world,
        target,
        false,
    )
    .await;
    assert_eq!(
        payload["explored"][0]["cells"].as_array().unwrap().len(),
        2, // one stored cell × 2 coords
        "emits only the target's stored explored"
    );
    assert_eq!(
        crate::scene::explored::ExploredSet::from_bytes(
            &repo.get_explored(scene, target).await.unwrap().unwrap(),
            crate::scene::GridKind::Square,
        )
        .len(),
        1,
        "see-as did not grow the target's persisted memory"
    );
}

/// `handle_pathfind` replies to the requesting connection only (one-shot).
/// GM gets PathResult (no mask). Non-GM in a dark scene (movementRestriction="visible",
/// env_intensity=0, no placed lights) gets PathError "unreachable" — empty mask blocks all cells.
#[tokio::test]
async fn pathfind_handler_gm_ok_nongm_dark_unreachable() {
    use crate::auth::role::ServerRole;
    use crate::data::document::{DocRole, WorldRole};
    use crate::data::membership::PermissionContext;
    use crate::ws::room::RoomRegistry;
    use serde_json::json;

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let author = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", author, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: author,
        world_role: WorldRole::Gm,
    };

    let p = repo
        .create_user("player", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world.id, p, WorldRole::Player)
        .await
        .unwrap();
    let player = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;

    let (scene_id, token_id, ws_id) = (
        Uuid::from_u128(0xA001),
        Uuid::from_u128(0xA002),
        Uuid::from_u128(0xA003),
    );

    // World-settings: visible restriction, totally dark (env_intensity=0, no placed lights).
    // A non-GM's visible_cells mask is therefore empty; all non-GM moves are blocked.
    let mut ws = wdoc(world.id, ws_id, "world-settings");
    ws.owner = Some(author);
    ws.system = json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#000000", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "partialCellLeniency": false
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    });
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: ws }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Scene with a 100-unit grid.
    let mut scene = wdoc(world.id, scene_id, "scene");
    scene.owner = Some(author);
    scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Player-owned token at (50,50) = cell (0,0). The player sees nothing (dark scene).
    let mut token = wdoc(world.id, token_id, "token");
    token.parent_id = Some(scene_id);
    token.owner = Some(p);
    token.permissions.users.insert(p, DocRole::Owner);
    token.engine = Some(token_engine(50.0, 50.0));
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: token }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let rid = Uuid::from_u128(0xF001);

    // GM: unconstrained (no mask) → PathResult for any reachable goal.
    let gm_result = handle_pathfind(
        PathfindRequest {
            request_id: rid,
            scene: scene_id,
            start: (50.0, 50.0),
            waypoints: vec![(250.0, 50.0)],
            footprint_radius: 0.1,
            token: None,
        },
        &gm_ctx,
        &room,
        repo.as_ref(),
    )
    .await;
    assert!(
        matches!(gm_result, ServerMsg::PathResult { .. }),
        "GM should get PathResult; got {gm_result:?}"
    );

    // Non-GM in a dark scene: mask is empty → every cell is out-of-mask → PathError "unreachable".
    // This is the documented fail-closed behaviour: dark scene + visible restriction freezes movement.
    let player_result = handle_pathfind(
        PathfindRequest {
            request_id: rid,
            scene: scene_id,
            start: (50.0, 50.0),
            waypoints: vec![(250.0, 50.0)],
            footprint_radius: 0.1,
            token: None,
        },
        &player,
        &room,
        repo.as_ref(),
    )
    .await;
    assert!(
        matches!(player_result, ServerMsg::PathError { ref message, .. } if message == "unreachable"),
        "non-GM in dark scene should be unreachable; got {player_result:?}"
    );
}

/// `scene_ping_permitted` admits a token-less reader and refuses a foreign-world scene, a
/// scene whose `permissions.default` denies READ, and a ghost id.
#[tokio::test]
async fn scene_ping_guard_admits_reader_refuses_foreign_and_hidden() {
    use crate::auth::role::ServerRole;
    use crate::data::document::{DocRole, WorldRole};
    use crate::data::membership::PermissionContext;
    use crate::ws::room::RoomRegistry;

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", gm, 0).await.unwrap();
    let other_world = repo.create_world_owned("X", gm, 0).await.unwrap();
    let p = repo
        .create_user("player", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world.id, p, WorldRole::Spectator)
        .await
        .unwrap();
    let spectator = PermissionContext {
        user_id: p,
        world_role: WorldRole::Spectator,
    };
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let gm_ctx_other = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };

    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let room_other = reg
        .get_or_create(repo.as_ref(), other_world.id)
        .await
        .unwrap()
        .unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;

    let (vis_id, foreign_id, hidden_id) = (
        Uuid::from_u128(0xB001),
        Uuid::from_u128(0xB002),
        Uuid::from_u128(0xB003),
    );

    // A readable scene in this world (default DocRole is `None`, so an explicit
    // `Observer` default is required for the spectator to hold READ).
    let mut vis = wdoc(world.id, vis_id, "scene");
    vis.owner = Some(gm);
    vis.permissions.default = DocRole::Observer;
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: vis }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // A scene in a DIFFERENT world (same GM authoring it).
    let mut foreign = wdoc(other_world.id, foreign_id, "scene");
    foreign.owner = Some(gm);
    room_other
        .publish(
            repo.as_ref(),
            &gm_ctx_other,
            vec![crate::data::command::Operation::Create { doc: foreign }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    // A scene in this world whose default grants nobody READ.
    let mut hidden = wdoc(world.id, hidden_id, "scene");
    hidden.owner = Some(gm);
    hidden.permissions.default = DocRole::None;
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: hidden }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Token-less spectator may ping the scene they can read...
    assert!(scene_ping_permitted(vis_id, &spectator, world.id, repo.as_ref()).await);
    // ...but not a scene in another world, a scene that denies READ, or a ghost id.
    assert!(!scene_ping_permitted(foreign_id, &spectator, world.id, repo.as_ref()).await);
    assert!(!scene_ping_permitted(hidden_id, &spectator, world.id, repo.as_ref()).await);
    assert!(
        !scene_ping_permitted(Uuid::from_u128(0xDEAD), &spectator, world.id, repo.as_ref()).await
    );
}

/// A `Pathfind` naming a scene the requester controls no token in is refused, even when that
/// scene is `unrestricted` (no visibility mask to fail closed on). Otherwise a player could
/// route-preview inside a scene they have never entered and read its `blocksMove` wall layout
/// off the returned polyline. The GM is unaffected, and the requester's own scene still routes.
#[tokio::test]
async fn pathfind_refuses_a_scene_the_requester_controls_no_token_in() {
    use crate::auth::role::ServerRole;
    use crate::data::document::{DocRole, WorldRole};
    use crate::data::membership::PermissionContext;
    use crate::ws::room::RoomRegistry;

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let author = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", author, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: author,
        world_role: WorldRole::Gm,
    };
    let p = repo
        .create_user("player", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world.id, p, WorldRole::Player)
        .await
        .unwrap();
    let player = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let (scene_a, scene_b, token_id, wall_id, ws_id) = (
        Uuid::from_u128(0xB001),
        Uuid::from_u128(0xB002),
        Uuid::from_u128(0xB003),
        Uuid::from_u128(0xB004),
        Uuid::from_u128(0xB005),
    );

    // Unrestricted world-wide: no visibility mask anywhere, so nothing else fails closed.
    let mut ws = wdoc(world.id, ws_id, "world-settings");
    ws.owner = Some(author);
    ws.system = json!({
        "scene": {
            "losRestriction": false, "fog": false,
            "lightingEnabled": false, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "unrestricted",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    });
    ws.engine = Some(ws_engine(ws.system.clone()));
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: ws }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    for id in [scene_a, scene_b] {
        let mut scene = wdoc(world.id, id, "scene");
        scene.owner = Some(author);
        scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    }

    // The player's only token lives in A.
    let mut token = wdoc(world.id, token_id, "token");
    token.parent_id = Some(scene_a);
    token.owner = Some(p);
    token.permissions.users.insert(p, DocRole::Owner);
    token.engine = Some(token_engine(50.0, 50.0));
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: token }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // B holds wall geometry the player must not be able to probe.
    let mut wall = wdoc(world.id, wall_id, "wall");
    wall.parent_id = Some(scene_b);
    wall.owner = Some(author);
    wall.engine =
        Some(json!({ "seg": { "x1": 200, "y1": -100, "x2": 200, "y2": 100 }, "blocksMove": true }));
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: wall }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let rid = Uuid::from_u128(0xF002);

    let leaked = handle_pathfind(
        PathfindRequest {
            request_id: rid,
            scene: scene_b,
            start: (50.0, 50.0),
            waypoints: vec![(450.0, 50.0)],
            footprint_radius: 0.1,
            token: None,
        },
        &player,
        &room,
        repo.as_ref(),
    )
    .await;
    assert!(
        matches!(leaked, ServerMsg::PathError { .. }),
        "a player with no token in B must not route inside B; got {leaked:?}"
    );

    // The same cross-scene probe, now naming a token the requester DOES own in scene A. The
    // presence gate must still refuse for scene B: naming a token is not presence in a scene.
    let leaked_with_named_token = handle_pathfind(
        PathfindRequest {
            request_id: rid,
            scene: scene_b,
            start: (50.0, 50.0),
            waypoints: vec![(250.0, 50.0)],
            footprint_radius: 0.4,
            token: Some(token_id),
        },
        &player,
        &room,
        repo.as_ref(),
    )
    .await;
    assert!(
        matches!(leaked_with_named_token, ServerMsg::PathError { .. }),
        "naming an owned token grants no presence in another scene; got {leaked_with_named_token:?}"
    );

    // Control: the player's own scene still routes (the guard cannot break play).
    let own = handle_pathfind(
        PathfindRequest {
            request_id: rid,
            scene: scene_a,
            start: (50.0, 50.0),
            waypoints: vec![(450.0, 50.0)],
            footprint_radius: 0.1,
            token: None,
        },
        &player,
        &room,
        repo.as_ref(),
    )
    .await;
    assert!(
        matches!(own, ServerMsg::PathResult { .. }),
        "the requester's own scene must still route; got {own:?}"
    );

    // The GM routes in any scene — presence is a non-GM gate only.
    let gm = handle_pathfind(
        PathfindRequest {
            request_id: rid,
            scene: scene_b,
            start: (50.0, 50.0),
            waypoints: vec![(450.0, 50.0)],
            footprint_radius: 0.1,
            token: None,
        },
        &gm_ctx,
        &room,
        repo.as_ref(),
    )
    .await;
    assert!(
        matches!(gm, ServerMsg::PathResult { .. }),
        "GM routing is unaffected; got {gm:?}"
    );
}

/// `Pathfind` naming a token the requester does not effectively own is refused generically —
/// the named token is a footprint SOURCE, not a delegated presence grant, so an attacker
/// cannot read a stranger's token size (or probe gaps sized to it) by naming an id they do
/// not control. Both requesters have their OWN token in the scene, so the Step-0 presence
/// gate passes for both — isolating this test to the token-ownership check (Step 4).
#[tokio::test]
async fn pathfind_naming_an_unowned_token_is_refused() {
    use crate::auth::role::ServerRole;
    use crate::data::document::{DocRole, WorldRole};
    use crate::data::membership::PermissionContext;
    use crate::ws::room::RoomRegistry;

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let author = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", author, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: author,
        world_role: WorldRole::Gm,
    };
    let pa = repo
        .create_user("player-a", None, ServerRole::User, 0)
        .await
        .unwrap();
    let pb = repo
        .create_user("player-b", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world.id, pa, WorldRole::Player)
        .await
        .unwrap();
    repo.add_member(world.id, pb, WorldRole::Player)
        .await
        .unwrap();
    let player_a = PermissionContext {
        user_id: pa,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let (scene_id, ws_id, token_a, token_b) = (
        Uuid::from_u128(0xC001),
        Uuid::from_u128(0xC002),
        Uuid::from_u128(0xC003),
        Uuid::from_u128(0xC004),
    );

    let mut ws = wdoc(world.id, ws_id, "world-settings");
    ws.owner = Some(author);
    ws.system = json!({
        "scene": {
            "losRestriction": false, "fog": false,
            "lightingEnabled": false, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "unrestricted",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    });
    ws.engine = Some(ws_engine(ws.system.clone()));
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: ws }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut scene = wdoc(world.id, scene_id, "scene");
    scene.owner = Some(author);
    scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut ta = wdoc(world.id, token_a, "token");
    ta.parent_id = Some(scene_id);
    ta.owner = Some(pa);
    ta.permissions.users.insert(pa, DocRole::Owner);
    ta.engine = Some(token_engine(50.0, 50.0));
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: ta }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Player B's own token — the one player A will name without owning it.
    let mut tb = wdoc(world.id, token_b, "token");
    tb.parent_id = Some(scene_id);
    tb.owner = Some(pb);
    tb.permissions.users.insert(pb, DocRole::Owner);
    tb.engine = Some(token_engine(60.0, 60.0));
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: tb }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let rid = Uuid::from_u128(0xF010);
    let res = handle_pathfind(
        PathfindRequest {
            request_id: rid,
            scene: scene_id,
            start: (50.0, 50.0),
            waypoints: vec![(250.0, 50.0)],
            footprint_radius: 0.4,
            token: Some(token_b),
        },
        &player_a,
        &room,
        repo.as_ref(),
    )
    .await;
    assert!(
        matches!(res, ServerMsg::PathError { .. }),
        "an unowned token is refused generically; got {res:?}"
    );
}

/// Guards the cross-scene axis: the requester has their OWN token in the named scene (so
/// Step-0 presence passes), but names a token they own in a DIFFERENT scene — the footprint
/// derivation (Step 4) must still refuse, isolating this from the presence gate.
#[tokio::test]
async fn pathfind_naming_a_token_in_another_scene_is_refused() {
    use crate::auth::role::ServerRole;
    use crate::data::document::{DocRole, WorldRole};
    use crate::data::membership::PermissionContext;
    use crate::ws::room::RoomRegistry;

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let author = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", author, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: author,
        world_role: WorldRole::Gm,
    };
    let p = repo
        .create_user("player", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world.id, p, WorldRole::Player)
        .await
        .unwrap();
    let player = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let (scene_a, scene_b, ws_id, token_in_a, token_in_b) = (
        Uuid::from_u128(0xC101),
        Uuid::from_u128(0xC102),
        Uuid::from_u128(0xC103),
        Uuid::from_u128(0xC104),
        Uuid::from_u128(0xC105),
    );

    let mut ws = wdoc(world.id, ws_id, "world-settings");
    ws.owner = Some(author);
    ws.system = json!({
        "scene": {
            "losRestriction": false, "fog": false,
            "lightingEnabled": false, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "unrestricted",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    });
    ws.engine = Some(ws_engine(ws.system.clone()));
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: ws }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    for id in [scene_a, scene_b] {
        let mut scene = wdoc(world.id, id, "scene");
        scene.owner = Some(author);
        scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    }

    // The token the attacker will name while requesting B — owned by the player, but
    // parented to A.
    let mut ta = wdoc(world.id, token_in_a, "token");
    ta.parent_id = Some(scene_a);
    ta.owner = Some(p);
    ta.permissions.users.insert(p, DocRole::Owner);
    ta.engine = Some(token_engine(50.0, 50.0));
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: ta }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // The player's OWN token in B, so the Step-0 presence gate for B passes independently of
    // the named token — isolating this test to Step 4's cross-scene check.
    let mut tb = wdoc(world.id, token_in_b, "token");
    tb.parent_id = Some(scene_b);
    tb.owner = Some(p);
    tb.permissions.users.insert(p, DocRole::Owner);
    tb.engine = Some(token_engine(50.0, 50.0));
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: tb }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let rid = Uuid::from_u128(0xF011);
    let res = handle_pathfind(
        PathfindRequest {
            request_id: rid,
            scene: scene_b,
            start: (50.0, 50.0),
            waypoints: vec![(250.0, 50.0)],
            footprint_radius: 0.4,
            token: Some(token_in_a),
        },
        &player,
        &room,
        repo.as_ref(),
    )
    .await;
    assert!(
        matches!(res, ServerMsg::PathError { .. }),
        "a token outside the named scene is refused; got {res:?}"
    );
}

/// Builds a scene with a 1-cell-wide corridor (walls at x=100 and x=200, cell=100) and one
/// owned, actor-linked token whose real footprint (radius 1.0 cell, `circle` shape, size
/// 2.0x2.0) is far too wide for the 100-unit corridor (which admits at most radius 0.5 cell).
/// Shared by the "ignores a lying wire footprint" (Step 4 derives and refuses) and "no token
/// honors the wire value" (Step 4 is skipped entirely) tests below.
async fn harness_with_narrow_corridor_and_large_owned_token() -> (
    Arc<SqliteRepository>,
    Arc<crate::ws::room::Room>,
    crate::data::membership::PermissionContext,
    Uuid,
    Uuid,
) {
    use crate::auth::role::ServerRole;
    use crate::data::document::{DocRole, WorldRole};
    use crate::data::membership::PermissionContext;
    use crate::ws::room::RoomRegistry;

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let author = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", author, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: author,
        world_role: WorldRole::Gm,
    };
    let p = repo
        .create_user("player", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world.id, p, WorldRole::Player)
        .await
        .unwrap();
    let player = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let (scene_id, ws_id, wall_1, wall_2, actor_id, token_id) = (
        Uuid::from_u128(0xC201),
        Uuid::from_u128(0xC202),
        Uuid::from_u128(0xC203),
        Uuid::from_u128(0xC204),
        Uuid::from_u128(0xC205),
        Uuid::from_u128(0xC206),
    );

    let mut ws = wdoc(world.id, ws_id, "world-settings");
    ws.owner = Some(author);
    ws.system = json!({
        "scene": {
            "losRestriction": false, "fog": false,
            "lightingEnabled": false, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "unrestricted",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    });
    ws.engine = Some(ws_engine(ws.system.clone()));
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: ws }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut scene = wdoc(world.id, scene_id, "scene");
    scene.owner = Some(author);
    scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Two parallel walls one cell apart (x=100, x=200), forming a north-south corridor a
    // vertical route can traverse (at x=150) without crossing either wall directly.
    for (id, x) in [(wall_1, 100.0), (wall_2, 200.0)] {
        let mut wall = wdoc(world.id, id, "wall");
        wall.parent_id = Some(scene_id);
        wall.owner = Some(author);
        wall.engine = Some(
            json!({ "seg": { "x1": x, "y1": -400.0, "x2": x, "y2": 400.0 }, "blocksMove": true }),
        );
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc: wall }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    }

    // The linked actor supplies the token's REAL size: a 2.0x2.0 circle ⇒ radius 1.0 cell —
    // far wider than the 100-unit (1-cell) corridor's 0.5-cell admissible radius.
    let mut actor = wdoc(world.id, actor_id, "actor");
    actor.owner = Some(author);
    actor.engine = Some(json!({
        "displayName": "Fixture Actor",
        "visual": { "kind": "image", "asset": "a.png" },
        "size": { "w": 2.0, "h": 2.0 },
        "shape": "circle",
        "conditions": [],
        "prototype": true,
    }));
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: actor }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut token = wdoc(world.id, token_id, "token");
    token.parent_id = Some(scene_id);
    token.owner = Some(p);
    token.permissions.users.insert(p, DocRole::Owner);
    token.engine = Some(json!({
        "x": 150.0, "y": 350.0, "w": 100.0, "h": 100.0, "rotation": 0.0,
        "actor_id": actor_id.to_string(),
    }));
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: token }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    (repo, room, player, scene_id, token_id)
}

/// The wire value lies (0.01, tiny) but the named token's real, derived footprint (radius 1.0
/// cell) does not fit the 1-cell corridor: the derived value must win, refusing the route.
#[tokio::test]
async fn pathfind_naming_an_owned_token_ignores_a_lying_wire_footprint() {
    let (repo, room, player, scene_id, token_id) =
        harness_with_narrow_corridor_and_large_owned_token().await;
    let rid = Uuid::from_u128(0xF012);
    let res = handle_pathfind(
        PathfindRequest {
            request_id: rid,
            scene: scene_id,
            start: (150.0, 350.0),
            waypoints: vec![(150.0, -350.0)],
            footprint_radius: 0.01,
            token: Some(token_id),
        },
        &player,
        &room,
        repo.as_ref(),
    )
    .await;
    assert!(
        matches!(res, ServerMsg::PathError { .. }),
        "the derived footprint does not fit the corridor, so no route is returned; got {res:?}"
    );
}

/// The SAME corridor and token, but no token is named: the wire's tiny 0.01 radius is honored
/// verbatim and fits, so the token-less preview routes through.
#[tokio::test]
async fn pathfind_without_a_token_uses_the_wire_footprint() {
    let (repo, room, player, scene_id, _token_id) =
        harness_with_narrow_corridor_and_large_owned_token().await;
    let rid = Uuid::from_u128(0xF013);
    let res = handle_pathfind(
        PathfindRequest {
            request_id: rid,
            scene: scene_id,
            start: (150.0, 350.0),
            waypoints: vec![(150.0, -350.0)],
            footprint_radius: 0.01,
            token: None,
        },
        &player,
        &room,
        repo.as_ref(),
    )
    .await;
    assert!(
        matches!(res, ServerMsg::PathResult { .. }),
        "a token-less preview honors the wire radius; got {res:?}"
    );
}

/// `resolve_token_footprint` returns `None` for a derived radius over `MAX_FOOTPRINT_CELLS` —
/// the handler must refuse, never fall back to the wire value (which would reopen the
/// understated-footprint hole a derived footprint exists to close).
#[tokio::test]
async fn pathfind_refuses_an_oversized_derived_footprint() {
    use crate::auth::role::ServerRole;
    use crate::data::document::{DocRole, WorldRole};
    use crate::data::membership::PermissionContext;
    use crate::ws::room::RoomRegistry;

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let author = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", author, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: author,
        world_role: WorldRole::Gm,
    };
    let p = repo
        .create_user("player", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world.id, p, WorldRole::Player)
        .await
        .unwrap();
    let player = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let (scene_id, ws_id, actor_id, token_id) = (
        Uuid::from_u128(0xC301),
        Uuid::from_u128(0xC302),
        Uuid::from_u128(0xC303),
        Uuid::from_u128(0xC304),
    );

    let mut ws = wdoc(world.id, ws_id, "world-settings");
    ws.owner = Some(author);
    ws.system = json!({
        "scene": {
            "losRestriction": false, "fog": false,
            "lightingEnabled": false, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "unrestricted",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    });
    ws.engine = Some(ws_engine(ws.system.clone()));
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: ws }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut scene = wdoc(world.id, scene_id, "scene");
    scene.owner = Some(author);
    scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // w=h=1000 ⇒ ~707 cells, far over MAX_FOOTPRINT_CELLS (64.0).
    let mut actor = wdoc(world.id, actor_id, "actor");
    actor.owner = Some(author);
    actor.engine = Some(json!({
        "displayName": "Fixture Actor",
        "visual": { "kind": "image", "asset": "a.png" },
        "size": { "w": 1000.0, "h": 1000.0 },
        "shape": "square",
        "conditions": [],
        "prototype": true,
    }));
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: actor }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut token = wdoc(world.id, token_id, "token");
    token.parent_id = Some(scene_id);
    token.owner = Some(p);
    token.permissions.users.insert(p, DocRole::Owner);
    token.engine = Some(json!({
        "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0,
        "actor_id": actor_id.to_string(),
    }));
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: token }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let rid = Uuid::from_u128(0xF014);
    let res = handle_pathfind(
        PathfindRequest {
            request_id: rid,
            scene: scene_id,
            start: (50.0, 50.0),
            waypoints: vec![(250.0, 50.0)],
            footprint_radius: 0.4,
            token: Some(token_id),
        },
        &player,
        &room,
        repo.as_ref(),
    )
    .await;
    assert!(matches!(res, ServerMsg::PathError { .. }), "got {res:?}");
}

/// `handle_move_request` executes a move, broadcasts `MoveStream` to the room,
/// and returns no success frame to the requester. The broadcast carries non-empty
/// samples terminating at the goal. A rejected move still yields `MoveError` to
/// the requester only.
#[tokio::test]
async fn handle_move_request_broadcasts_move_stream_no_etx_on_success() {
    use crate::auth::role::ServerRole;
    use crate::data::document::{DocRole, WorldRole};
    use crate::data::membership::PermissionContext;
    use crate::ws::protocol::ServerMsg;
    use crate::ws::room::RoomRegistry;

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let author = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", author, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: author,
        world_role: WorldRole::Gm,
    };

    let p = repo
        .create_user("player", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world.id, p, WorldRole::Player)
        .await
        .unwrap();
    let player = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;

    let (scene_id, token_id, ws_id) = (
        Uuid::from_u128(0xB001),
        Uuid::from_u128(0xB002),
        Uuid::from_u128(0xB003),
    );

    // World-settings: unrestricted movement so the player token can move freely.
    let mut ws = wdoc(world.id, ws_id, "world-settings");
    ws.owner = Some(author);
    ws.system = json!({
        "scene": {
            "losRestriction": false, "fog": false,
            "lightingEnabled": false, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "unrestricted",
            "partialCellLeniency": false
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    });
    ws.engine = Some(ws_engine(ws.system.clone()));
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: ws }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Scene with a 100-unit grid.
    let mut scene = wdoc(world.id, scene_id, "scene");
    scene.owner = Some(author);
    scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Player-owned token at (50,50). Start = cell-center (0,0); goal = (150,50) = one step right.
    let mut token = wdoc(world.id, token_id, "token");
    token.parent_id = Some(scene_id);
    token.owner = Some(p);
    token.permissions.users.insert(p, DocRole::Owner);
    token.engine = Some(token_engine(50.0, 50.0));
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: token }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Subscribe BEFORE issuing the request so the broadcast is not missed.
    let (mut rx, _) = room.subscribe();

    let request_id = Uuid::from_u128(7);
    let expected_goal = [150.0_f64, 50.0_f64];

    // Success: handle_move_request returns None (no etx frame to the requester).
    let result = handle_move_request(
        &room,
        repo.as_ref(),
        &player,
        scene_id,
        token_id,
        vec![[50.0, 50.0], [150.0, 50.0]],
        request_id,
    )
    .await;
    assert!(
        result.is_none(),
        "success path must return None (no etx frame); got {result:?}"
    );

    // The broadcast ring must contain a MoveStream observable on a second subscriber.
    // broadcast_aux sends to existing receivers; rx was subscribed before the call.
    let bcast = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            match rx.recv().await {
                Ok(crate::ws::room::RoomEvent::Other(msg)) => {
                    if matches!(msg.as_ref(), ServerMsg::MoveStream { .. }) {
                        return Some((*msg).clone());
                    }
                    // Skip other out-of-band frames.
                }
                Ok(crate::ws::room::RoomEvent::Event(_)) => {
                    // Skip: a committed Event frame (e.g. the move's position Update), not
                    // MoveStream.
                }
                Err(_) => return None,
            }
        }
    })
    .await
    .expect("timed out waiting for MoveStream broadcast")
    .expect("receiver closed before MoveStream");

    match bcast {
        ServerMsg::MoveStream {
            request_id: rid,
            token_id: tid,
            mover,
            stop,
            samples,
            mover_vision,
            ..
        } => {
            assert_eq!(rid, request_id, "request_id must be correlated");
            assert_eq!(tid, token_id, "token_id must match");
            assert_eq!(mover, p, "mover must be the player");
            assert_eq!(stop, expected_goal, "stop must equal the goal");
            assert!(!samples.is_empty(), "samples must be non-empty");
            assert!(
                (samples[0].t_ms - 0.0).abs() < 1e-9,
                "first sample t_ms must be 0"
            );
            assert_eq!(
                samples.last().unwrap().pos,
                expected_goal,
                "last sample pos must equal stop"
            );
            assert!(
                mover_vision.is_some(),
                "a non-GM mover must get a progressive vision sweep, even in an \
                 Unrestricted-mode scene (gated on role, not restriction mode)"
            );
        }
        other => panic!("expected MoveStream, got {other:?}"),
    }

    // Rejection: a move for a non-existent token yields MoveError to etx only.
    let bad_token = Uuid::from_u128(0xDEAD);
    let err_result = handle_move_request(
        &room,
        repo.as_ref(),
        &player,
        scene_id,
        bad_token,
        vec![[50.0, 50.0], [150.0, 50.0]],
        Uuid::from_u128(8),
    )
    .await;
    assert!(
        matches!(err_result, Some(ServerMsg::MoveError { .. })),
        "rejection must return MoveError; got {err_result:?}"
    );
}

/// A token-less player (masked + empty polygons) accumulates nothing and emits empty explored
/// → full fog. No per-scene secret memory is fabricated.
#[tokio::test]
async fn enrich_token_less_player_emits_no_explored() {
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let grid = std::collections::HashMap::new();
    let grid_shapes = square_grid_shapes(&grid);
    let mut payload = json!({ "mode": "masked", "polygons": [] });
    enrich_vision_explored(
        &mut payload,
        &grid,
        &grid_shapes,
        &repo,
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        true,
    )
    .await;
    assert_eq!(payload["explored"].as_array().unwrap().len(), 0);
}

// ─── clip_move_stream: per-recipient secrecy boundary tests ───────────────

/// Shared setup for `clip_move_stream` integration tests: creates an in-memory world, a GM
/// user, an observer player user, one scene, and optionally an observer token + a wall doc.
/// Returns `(room, gm_ctx, observer_ctx, scene_id)`.
///
/// world-settings are omitted — `player_vision_polygons` only needs tokens + walls.
async fn setup_clip_room(
    obs_token_pos: Option<(f64, f64)>,
    wall_system: Option<serde_json::Value>,
    wall_gm_only: bool,
) -> (
    Arc<crate::ws::room::Room>,
    PermissionContext,
    PermissionContext,
    Uuid,
) {
    use crate::auth::role::ServerRole;
    use crate::data::command::Operation;
    use crate::data::document::{DocRole, WorldRole};
    use crate::ws::room::RoomRegistry;

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };

    let obs = repo
        .create_user("obs", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world.id, obs, WorldRole::Player)
        .await
        .unwrap();
    let obs_ctx = PermissionContext {
        user_id: obs,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;

    let scene_id = Uuid::from_u128(0xE001);
    let mut scene = wdoc(world.id, scene_id, "scene");
    scene.owner = Some(gm);
    scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    if let Some(pos) = obs_token_pos {
        let token_id = Uuid::from_u128(0xE002);
        let mut tok = wdoc(world.id, token_id, "token");
        tok.parent_id = Some(scene_id);
        tok.owner = Some(obs);
        tok.permissions.users.insert(obs, DocRole::Owner);
        tok.engine = Some(token_engine(pos.0, pos.1));
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![Operation::Create { doc: tok }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    }

    if let Some(ws) = wall_system {
        let wall_id = Uuid::from_u128(0xE003);
        let mut wall = wdoc(world.id, wall_id, "wall");
        wall.parent_id = Some(scene_id);
        wall.owner = Some(gm);
        wall.engine = Some(ws);
        if wall_gm_only {
            // gm_only wall: DocRole::None means players cannot read the doc;
            // sight_walls uses the FULL wall set regardless (permission-blind).
            wall.permissions.default = DocRole::None;
        }
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![Operation::Create { doc: wall }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    }

    (room, gm_ctx, obs_ctx, scene_id)
}

/// The mover (ctx.user_id == frame.mover) receives their own full frame
/// unchanged — all samples and `mover_vision` are forwarded verbatim.
#[tokio::test]
async fn clip_mover_receives_full_frame() {
    use crate::data::document::WorldRole;
    use crate::ws::protocol::{PosSample, VisionSample};

    let (room, _, _, scene_id) = setup_clip_room(None, None, false).await;

    let mover_id = Uuid::from_u128(0xAABB);
    // ctx.user_id == mover → mover branch fires before GM / observer branches.
    let ctx = PermissionContext {
        user_id: mover_id,
        world_role: WorldRole::Player,
    };

    let samples = vec![
        PosSample {
            t_ms: 0.0,
            pos: [50.0, 50.0],
        },
        PosSample {
            t_ms: 200.0,
            pos: [150.0, 50.0],
        },
    ];
    let mv = Some(vec![VisionSample {
        t_ms: 0.0,
        polygons: vec![vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]]],
    }]);
    let frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(1),
        token_id: Uuid::from_u128(2),
        mover: mover_id,
        scene: scene_id,
        start_server_ms: 1000.0,
        duration_ms: 400.0,
        stop: [150.0, 50.0],
        samples: samples.clone(),
        mover_vision: mv.clone(),
        cost: Some(2.0),
        truncated: Some(false),
    };

    let result = clip_move_stream(&frame, &ctx, None, &room).await;

    assert!(result.is_some(), "mover must receive a frame");
    match result.unwrap() {
        ServerMsg::MoveStream {
            samples: s,
            mover_vision: mv_out,
            cost,
            ..
        } => {
            assert_eq!(s, samples, "mover receives all samples unchanged");
            assert_eq!(mv_out, mv, "mover receives mover_vision unchanged");
            assert_eq!(cost, Some(2.0), "mover receives the true cost unchanged");
        }
        other => panic!("expected MoveStream, got {other:?}"),
    }
}

/// An observer with no token in the scene has empty vision polygons → every sample is
/// outside their vision → the frame is suppressed entirely (None, not empty-samples Some).
#[tokio::test]
async fn clip_observer_no_token_suppressed() {
    use crate::ws::protocol::PosSample;

    // No observer token in the scene — player_vision_polygons returns empty.
    let (room, _, obs_ctx, scene_id) = setup_clip_room(None, None, false).await;

    let mover_id = Uuid::from_u128(0xAABB);
    let frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(1),
        token_id: Uuid::from_u128(2),
        mover: mover_id,
        scene: scene_id,
        start_server_ms: 1000.0,
        duration_ms: 600.0,
        stop: [250.0, 50.0],
        samples: vec![
            PosSample {
                t_ms: 0.0,
                pos: [50.0, 50.0],
            },
            PosSample {
                t_ms: 200.0,
                pos: [150.0, 50.0],
            },
            PosSample {
                t_ms: 400.0,
                pos: [250.0, 50.0],
            },
        ],
        mover_vision: None,
        cost: Some(2.0),
        truncated: Some(false),
    };

    let result = clip_move_stream(&frame, &obs_ctx, None, &room).await;

    assert!(
        result.is_none(),
        "observer with no token must receive no frame (suppressed); got {result:?}"
    );
}

/// An observer whose token is on the near side of a `blocksSight` wall sees only samples
/// on their side. The clipped frame carries those samples with `mover_vision = None`.
///
/// Setup: observer token at (50,50); vertical wall at x=100. Samples at (50,50), (150,50),
/// (250,50). Only (50,50) is on the near side — the other two are occluded.
#[tokio::test]
async fn clip_observer_sees_near_side_prefix() {
    use crate::ws::protocol::PosSample;

    let wall_sys = json!({
        "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 },
        "blocksSight": true
    });
    let (room, _, obs_ctx, scene_id) =
        setup_clip_room(Some((50.0, 50.0)), Some(wall_sys), false).await;

    let mover_id = Uuid::from_u128(0xAABB);
    let frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(1),
        token_id: Uuid::from_u128(2),
        mover: mover_id,
        scene: scene_id,
        start_server_ms: 1000.0,
        duration_ms: 600.0,
        stop: [250.0, 50.0],
        samples: vec![
            PosSample {
                t_ms: 0.0,
                pos: [50.0, 50.0], // near side — observer can see this
            },
            PosSample {
                t_ms: 200.0,
                pos: [150.0, 50.0], // behind wall — occluded
            },
            PosSample {
                t_ms: 400.0,
                pos: [250.0, 50.0], // further behind wall — occluded
            },
        ],
        mover_vision: None,
        cost: Some(2.0),
        truncated: Some(false),
    };

    let result = clip_move_stream(&frame, &obs_ctx, None, &room).await;

    assert!(
        result.is_some(),
        "partial-visibility observer must receive a clipped frame"
    );
    match result.unwrap() {
        ServerMsg::MoveStream {
            samples: s,
            mover_vision: mv,
            stop: out_stop,
            duration_ms: out_duration_ms,
            cost,
            ..
        } => {
            assert_eq!(
                s.len(),
                1,
                "only one sample (near side) should be visible; got {} samples: {s:?}",
                s.len()
            );
            assert_eq!(
                s[0].pos,
                [50.0_f64, 50.0_f64],
                "visible sample must be (50,50)"
            );
            assert_eq!(mv, None, "mover_vision must be None for observers");
            // stop and duration_ms must be clipped to the last visible sample,
            // NOT the true goal/full travel distance.
            assert_eq!(
                out_stop,
                [50.0_f64, 50.0_f64],
                "stop must be clipped to last visible sample pos, not the true goal"
            );
            assert!(
                (out_duration_ms - 0.0_f64).abs() < 1e-9,
                "duration_ms must be clipped to last visible sample t_ms (0 ms), got {out_duration_ms}"
            );
            // Secrecy: a clipped observer must never learn the
            // true authoritative cost — it may reflect secret (gm_only) region terrain
            // the observer's clipped samples never reveal.
            assert_eq!(
                cost, None,
                "cost must be nulled for a clipped observer (secrecy: must not disclose \
                 authoritative cost, which may include secret-region terrain)"
            );
        }
        other => panic!("expected MoveStream, got {other:?}"),
    }
}

/// Same near-side/occluded clip boundary as `clip_observer_sees_near_side_prefix`, but over
/// a genuinely any-angle (non-axis-aligned) path — proves the per-recipient egress clip
/// is engine-agnostic geometry, unaffected by whether the sampled polyline is grid-stepped or
/// continuous. Wall at x=100 (unchanged); observer at (50,50) sees anything with
/// x<100 regardless of y, so the diagonal y-offsets below don't change the visibility split.
#[tokio::test]
async fn clip_observer_sees_near_side_prefix_any_angle_diagonal_path() {
    use crate::ws::protocol::PosSample;

    let wall_sys = json!({
        "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 },
        "blocksSight": true
    });
    let (room, _, obs_ctx, scene_id) =
        setup_clip_room(Some((50.0, 50.0)), Some(wall_sys), false).await;

    let mover_id = Uuid::from_u128(0xAABB);
    let frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(1),
        token_id: Uuid::from_u128(2),
        mover: mover_id,
        scene: scene_id,
        start_server_ms: 1000.0,
        duration_ms: 1500.0,
        stop: [310.0, 10.0],
        samples: vec![
            PosSample {
                t_ms: 0.0,
                pos: [50.0, 60.0], // near side, diagonal offset — visible
            },
            PosSample {
                t_ms: 750.0,
                pos: [140.0, 95.0], // behind wall, diagonal — occluded
            },
            PosSample {
                t_ms: 1500.0,
                pos: [310.0, 10.0], // further behind wall, diagonal — occluded
            },
        ],
        mover_vision: None,
        cost: Some(3.0),
        truncated: Some(false),
    };

    let result = clip_move_stream(&frame, &obs_ctx, None, &room).await;

    assert!(
        result.is_some(),
        "partial-visibility observer must receive a clipped frame"
    );
    match result.unwrap() {
        ServerMsg::MoveStream {
            samples: s,
            mover_vision: mv,
            stop: out_stop,
            duration_ms: out_duration_ms,
            cost,
            truncated,
            ..
        } => {
            assert_eq!(
                s.len(),
                1,
                "only the near-side diagonal sample is visible; got {} samples: {s:?}",
                s.len()
            );
            assert_eq!(s[0].pos, [50.0_f64, 60.0_f64]);
            assert_eq!(mv, None, "mover_vision must be None for observers");
            assert_eq!(
                out_stop,
                [50.0_f64, 60.0_f64],
                "stop clips to the last visible sample, not the true diagonal goal"
            );
            // duration_ms must be clipped to the last visible
            // sample's t_ms, NOT the true goal/full travel duration (mirrors the
            // axis-aligned sibling `clip_observer_sees_near_side_prefix`).
            assert!(
                (out_duration_ms - 0.0_f64).abs() < 1e-9,
                "duration_ms must be clipped to last visible sample t_ms (0 ms), got {out_duration_ms}"
            );
            assert_eq!(cost, None, "cost must be nulled for a clipped observer");
            assert_eq!(
                truncated, None,
                "truncated must be nulled for a clipped observer"
            );
        }
        other => panic!("expected MoveStream, got {other:?}"),
    }
}

/// A `gm_only` (`DocRole::None`) `blocksSight` wall bounds the observer's authoritative
/// vision identically to a normal wall — `sight_walls` is permission-blind, returning
/// every wall regardless of visibility tier. When the mover's entire path lies behind
/// the secret wall, the frame is
/// fully suppressed: the observer receives zero `MoveStream` frames, not an empty-sample one.
#[tokio::test]
async fn clip_gm_only_wall_suppresses_observer() {
    use crate::ws::protocol::PosSample;

    // gm_only wall at x=100; observer token at (50,50) cannot see x>100.
    // All mover samples are beyond the wall → every sample is occluded → suppress.
    let wall_sys = json!({
        "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 },
        "blocksSight": true
    });
    let (room, _, obs_ctx, scene_id) =
        setup_clip_room(Some((50.0, 50.0)), Some(wall_sys), true /* gm_only */).await;

    let mover_id = Uuid::from_u128(0xAABB);
    let frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(1),
        token_id: Uuid::from_u128(2),
        mover: mover_id,
        scene: scene_id,
        start_server_ms: 1000.0,
        duration_ms: 400.0,
        stop: [250.0, 50.0],
        samples: vec![
            PosSample {
                t_ms: 0.0,
                pos: [150.0, 50.0], // behind gm_only wall — occluded
            },
            PosSample {
                t_ms: 200.0,
                pos: [250.0, 50.0], // further behind — also occluded
            },
        ],
        mover_vision: None,
        cost: Some(2.0),
        truncated: Some(false),
    };

    let result = clip_move_stream(&frame, &obs_ctx, None, &room).await;

    // Must be None (fully suppressed), NOT Some(MoveStream { samples: [], .. }).
    // The secrecy invariant: zero frames sent, never an empty-samples frame.
    assert!(
        result.is_none(),
        "observer behind gm_only wall must receive zero MoveStream frames (None, not \
         Some(empty)); got {result:?}"
    );
}

/// A GM who is NOT the mover receives ALL position samples regardless of LOS, with
/// `mover_vision` nulled and the full `stop`/`duration_ms` intact.
///
/// Invariants verified:
/// - GM branch fires before observer branch (ctx.user_id != mover, but Gm role).
/// - All samples pass through unfiltered.
/// - `mover_vision` is never forwarded to anyone but the mover.
/// - `stop` and `duration_ms` are the full values (no clip for GM).
#[tokio::test]
async fn clip_gm_receives_all_samples_mover_vision_nulled() {
    use crate::data::document::WorldRole;
    use crate::ws::protocol::{PosSample, VisionSample};

    // Wall at x=100; the mover's samples cross to the far side, but a GM sees everything.
    let wall_sys = json!({
        "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 },
        "blocksSight": true
    });
    let (room, gm_ctx, _, scene_id) = setup_clip_room(None, Some(wall_sys), false).await;

    // GM is NOT the mover.
    let mover_id = Uuid::from_u128(0xAABB);
    assert_ne!(
        gm_ctx.user_id, mover_id,
        "GM must not be the mover in this test"
    );
    assert_eq!(gm_ctx.world_role, WorldRole::Gm);

    let samples = vec![
        PosSample {
            t_ms: 0.0,
            pos: [50.0, 50.0],
        },
        PosSample {
            t_ms: 200.0,
            pos: [150.0, 50.0],
        }, // behind wall — still visible to GM
        PosSample {
            t_ms: 400.0,
            pos: [250.0, 50.0],
        },
    ];
    let true_stop = [250.0_f64, 50.0_f64];
    let true_duration_ms = 600.0_f64;
    let mv = Some(vec![VisionSample {
        t_ms: 0.0,
        polygons: vec![vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]]],
    }]);
    let frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(1),
        token_id: Uuid::from_u128(2),
        mover: mover_id,
        scene: scene_id,
        start_server_ms: 1000.0,
        duration_ms: true_duration_ms,
        stop: true_stop,
        samples: samples.clone(),
        mover_vision: mv,
        cost: Some(2.0),
        truncated: Some(false),
    };

    let result = clip_move_stream(&frame, &gm_ctx, None, &room).await;

    assert!(result.is_some(), "GM must receive a frame");
    match result.unwrap() {
        ServerMsg::MoveStream {
            samples: s,
            mover_vision: mv_out,
            stop: out_stop,
            duration_ms: out_duration_ms,
            cost,
            ..
        } => {
            assert_eq!(s, samples, "GM receives all samples unchanged");
            assert_eq!(mv_out, None, "mover_vision must be nulled for GM observers");
            assert_eq!(out_stop, true_stop, "GM receives the full stop (no clip)");
            assert!(
                (out_duration_ms - true_duration_ms).abs() < 1e-9,
                "GM receives the full duration_ms (no clip)"
            );
            assert_eq!(
                cost,
                Some(2.0),
                "GM receives the true cost unchanged (trusted, full information)"
            );
        }
        other => panic!("expected MoveStream, got {other:?}"),
    }
}

/// A GM previewing AS a specific player (`see_as = Some(target)`) whose token is in the
/// move's scene has their OWN view narrowed to what that player would actually see mid-move,
/// via the same clip path a real observer gets. Setup mirrors
/// `clip_observer_sees_near_side_prefix`: target token at (50,50), vertical `blocksSight`
/// wall at x=100, samples at (50,50)/(150,50)/(250,50). Only the near-side sample is visible.
///
/// This is the core behavior change — before threading `see_as`, the GM branch ALWAYS
/// returned the full unclipped stream regardless of the active see-as target.
#[tokio::test]
async fn clip_gm_see_as_clips_to_target_vision() {
    use crate::ws::protocol::PosSample;

    let wall_sys = json!({
        "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 },
        "blocksSight": true
    });
    // `obs` is the see-as target: a player with a token at (50,50).
    let (room, gm_ctx, target_ctx, scene_id) =
        setup_clip_room(Some((50.0, 50.0)), Some(wall_sys), false).await;

    let mover_id = Uuid::from_u128(0xAABB);
    let frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(1),
        token_id: Uuid::from_u128(2),
        mover: mover_id,
        scene: scene_id,
        start_server_ms: 1000.0,
        duration_ms: 600.0,
        stop: [250.0, 50.0],
        samples: vec![
            PosSample {
                t_ms: 0.0,
                pos: [50.0, 50.0], // near side — target can see this
            },
            PosSample {
                t_ms: 200.0,
                pos: [150.0, 50.0], // behind wall — occluded from the target
            },
            PosSample {
                t_ms: 400.0,
                pos: [250.0, 50.0], // further behind — occluded from the target
            },
        ],
        mover_vision: None,
        cost: Some(2.0),
        truncated: Some(false),
    };

    // GM previewing as `target`.
    let result = clip_move_stream(&frame, &gm_ctx, Some(target_ctx), &room).await;

    assert!(
        result.is_some(),
        "GM see-as preview with a visible sample must receive a clipped frame"
    );
    match result.unwrap() {
        ServerMsg::MoveStream {
            samples: s,
            mover_vision: mv,
            stop: out_stop,
            duration_ms: out_duration_ms,
            cost,
            truncated,
            ..
        } => {
            assert_eq!(
                s.len(),
                1,
                "GM see-as must be clipped to the target's vision (near-side only); got {} \
                 samples: {s:?}",
                s.len()
            );
            assert_eq!(s[0].pos, [50.0_f64, 50.0_f64]);
            assert_eq!(mv, None, "mover_vision stays None for a see-as preview");
            assert_eq!(
                out_stop,
                [50.0_f64, 50.0_f64],
                "stop clips to the target's last visible sample, not the true goal"
            );
            assert!(
                (out_duration_ms - 0.0_f64).abs() < 1e-9,
                "duration_ms clips to the target's last visible sample t_ms, got {out_duration_ms}"
            );
            assert_eq!(
                cost, None,
                "cost nulled for a clipped see-as preview (same secrecy as a real observer)"
            );
            assert_eq!(
                truncated, None,
                "truncated nulled for a clipped see-as preview (same secrecy as a real observer)"
            );
        }
        other => panic!("expected MoveStream, got {other:?}"),
    }
}

/// A see-as whose target has NO vision source in the move's scene (their token is in a
/// DIFFERENT scene) must NOT clip — the see-as is not applicable to this scene, so the GM
/// keeps the full unclipped stream (today's plain-GM behavior). Scene-exactness guard: the
/// target's token lives in `scene_id`, but the move targets an unrelated scene, so
/// `player_vision_polygons(target)` (tagged with `scene_id`) filters to empty for the move's
/// scene.
#[tokio::test]
async fn clip_gm_see_as_different_scene_not_clipped() {
    use crate::ws::protocol::PosSample;

    // Target token at (50,50) in `scene_id`; a wall that WOULD occlude if it applied.
    let wall_sys = json!({
        "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 },
        "blocksSight": true
    });
    let (room, gm_ctx, target_ctx, _scene_id) =
        setup_clip_room(Some((50.0, 50.0)), Some(wall_sys), false).await;

    // The move happens in a DIFFERENT scene where the target has no token.
    let other_scene = Uuid::from_u128(0xE099);
    let mover_id = Uuid::from_u128(0xAABB);
    let samples = vec![
        PosSample {
            t_ms: 0.0,
            pos: [50.0, 50.0],
        },
        PosSample {
            t_ms: 200.0,
            pos: [150.0, 50.0],
        },
        PosSample {
            t_ms: 400.0,
            pos: [250.0, 50.0],
        },
    ];
    let frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(1),
        token_id: Uuid::from_u128(2),
        mover: mover_id,
        scene: other_scene,
        start_server_ms: 1000.0,
        duration_ms: 600.0,
        stop: [250.0, 50.0],
        samples: samples.clone(),
        mover_vision: None,
        cost: Some(2.0),
        truncated: Some(false),
    };

    let result = clip_move_stream(&frame, &gm_ctx, Some(target_ctx), &room).await;

    assert!(
        result.is_some(),
        "different-scene see-as must not suppress the GM's frame"
    );
    match result.unwrap() {
        ServerMsg::MoveStream {
            samples: s, cost, ..
        } => {
            assert_eq!(
                s, samples,
                "a see-as for a different scene must not clip — GM keeps all samples"
            );
            assert_eq!(
                cost,
                Some(2.0),
                "a see-as for a different scene must not null the GM's cost"
            );
        }
        other => panic!("expected MoveStream, got {other:?}"),
    }
}

/// A see-as target who IS in the move's scene but cannot see ANY sample (the whole move is
/// behind a `gm_only` wall, from the target's viewpoint) is suppressed entirely — the GM's
/// faithful preview shows zero frames, exactly like the previewed player would receive.
#[tokio::test]
async fn clip_gm_see_as_fully_occluded_suppressed() {
    use crate::ws::protocol::PosSample;

    // gm_only wall at x=100; target token at (50,50) cannot see x>100.
    let wall_sys = json!({
        "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 },
        "blocksSight": true
    });
    let (room, gm_ctx, target_ctx, scene_id) =
        setup_clip_room(Some((50.0, 50.0)), Some(wall_sys), true /* gm_only */).await;

    let mover_id = Uuid::from_u128(0xAABB);
    let frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(1),
        token_id: Uuid::from_u128(2),
        mover: mover_id,
        scene: scene_id,
        start_server_ms: 1000.0,
        duration_ms: 400.0,
        stop: [250.0, 50.0],
        samples: vec![
            PosSample {
                t_ms: 0.0,
                pos: [150.0, 50.0], // behind the wall from the target's viewpoint
            },
            PosSample {
                t_ms: 200.0,
                pos: [250.0, 50.0], // further behind
            },
        ],
        mover_vision: None,
        cost: Some(2.0),
        truncated: Some(false),
    };

    let result = clip_move_stream(&frame, &gm_ctx, Some(target_ctx), &room).await;

    assert!(
        result.is_none(),
        "a see-as preview of a wholly-occluded move must be suppressed (None); got {result:?}"
    );
}

/// Register an in-flight stream for `mover` in `scene` whose vision timeline is `vision`.
/// `start_ms` is the stream's `start_server_ms`; it stays unexpired for an hour.
async fn register_timeline(
    room: &crate::ws::room::Room,
    token: Uuid,
    mover: Uuid,
    scene: Uuid,
    start_ms: i64,
    vision: Vec<crate::ws::protocol::VisionSample>,
) {
    use crate::ws::room::ActiveStream;
    let frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(0x7777),
        token_id: token,
        mover,
        scene,
        start_server_ms: start_ms as f64,
        duration_ms: 3_600_000.0,
        stop: [0.0, 0.0],
        samples: vec![crate::ws::protocol::PosSample {
            t_ms: 0.0,
            pos: [0.0, 0.0],
        }],
        mover_vision: Some(vision),
        cost: Some(0.0),
        truncated: Some(false),
    };
    room.register_stream_for_test(
        token,
        ActiveStream {
            mover,
            scene,
            start_ms,
            end_ms: start_ms + 3_600_000,
            frame: Arc::new(frame),
        },
    )
    .await;
}

/// A big square covering x∈[x0,x1], y∈[0,100].
fn band(x0: f64, x1: f64) -> Vec<Vec<[f64; 2]>> {
    vec![vec![[x0, 0.0], [x1, 0.0], [x1, 100.0], [x0, 100.0]]]
}

/// Observer at (50,50) behind a wall at x=100 — committed vision never sees x>100. The
/// observer's OWN in-flight sweep (started before A) sees x∈[100,300] from its second sample
/// (t=200 after its start). A's samples at (150,50)/(250,50) at A-times 0/200 fall at absolute
/// instants where the sweep shows sample 0 (band 0..100 → occluded) then sample 1 (band → visible).
#[tokio::test]
async fn clip_observer_mid_move_admits_samples_its_own_sweep_will_reveal() {
    use crate::ws::protocol::{PosSample, VisionSample};
    let wall_sys =
        json!({ "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 }, "blocksSight": true });
    let (room, _, obs_ctx, scene_id) =
        setup_clip_room(Some((50.0, 50.0)), Some(wall_sys), false).await;
    let now = crate::ws::time::now_millis();
    register_timeline(
        &room,
        Uuid::from_u128(0xE002),
        obs_ctx.user_id,
        scene_id,
        now,
        vec![
            VisionSample {
                t_ms: 0.0,
                polygons: band(0.0, 100.0),
            },
            VisionSample {
                t_ms: 200.0,
                polygons: band(100.0, 300.0),
            },
        ],
    )
    .await;
    let frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(1),
        token_id: Uuid::from_u128(2),
        mover: Uuid::from_u128(0xAABB),
        scene: scene_id,
        start_server_ms: (now + 100) as f64,
        duration_ms: 400.0,
        stop: [250.0, 50.0],
        samples: vec![
            PosSample {
                t_ms: 0.0,
                pos: [150.0, 50.0],
            }, // abs now+100 → sweep sample 0 → hidden
            PosSample {
                t_ms: 200.0,
                pos: [250.0, 50.0],
            }, // abs now+300 → sweep sample 1 → visible
        ],
        mover_vision: None,
        cost: Some(2.0),
        truncated: Some(false),
    };
    let out = clip_move_stream(&frame, &obs_ctx, None, &room)
        .await
        .expect("one sample visible");
    let ServerMsg::MoveStream {
        samples,
        stop,
        duration_ms,
        mover_vision,
        cost,
        truncated,
        ..
    } = out
    else {
        panic!()
    };
    assert_eq!(
        samples,
        vec![PosSample {
            t_ms: 200.0,
            pos: [250.0, 50.0]
        }]
    );
    assert_eq!(stop, [250.0, 50.0]);
    assert!((duration_ms - 200.0).abs() < 1e-9);
    assert_eq!(
        (mover_vision, cost, truncated),
        (None, None, None),
        "observer secrecy nulls unchanged"
    );
}

/// Same geometry, but the observer's sweep starts AFTER every sample of the move: the
/// timeline never applies and committed vision (blocked by the wall) suppresses the frame —
/// closing this ordering needs the observer's own move to re-emit the concurrent stream, not
/// this clip.
#[tokio::test]
async fn clip_ignores_a_timeline_that_starts_after_the_move() {
    use crate::ws::protocol::{PosSample, VisionSample};
    let wall_sys =
        json!({ "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 }, "blocksSight": true });
    let (room, _, obs_ctx, scene_id) =
        setup_clip_room(Some((50.0, 50.0)), Some(wall_sys), false).await;
    let now = crate::ws::time::now_millis();
    register_timeline(
        &room,
        Uuid::from_u128(0xE002),
        obs_ctx.user_id,
        scene_id,
        now + 10_000,
        vec![VisionSample {
            t_ms: 0.0,
            polygons: band(100.0, 300.0),
        }],
    )
    .await;
    let frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(1),
        token_id: Uuid::from_u128(2),
        mover: Uuid::from_u128(0xAABB),
        scene: scene_id,
        start_server_ms: now as f64,
        duration_ms: 400.0,
        stop: [250.0, 50.0],
        samples: vec![
            PosSample {
                t_ms: 0.0,
                pos: [150.0, 50.0],
            },
            PosSample {
                t_ms: 200.0,
                pos: [250.0, 50.0],
            },
        ],
        mover_vision: None,
        cost: Some(2.0),
        truncated: Some(false),
    };
    assert!(clip_move_stream(&frame, &obs_ctx, None, &room)
        .await
        .is_none());
}

/// GM see-as: the target's timeline, not the GM's own, drives the clip.
#[tokio::test]
async fn clip_gm_see_as_uses_the_targets_timeline() {
    use crate::ws::protocol::{PosSample, VisionSample};
    let wall_sys =
        json!({ "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 }, "blocksSight": true });
    let (room, gm_ctx, obs_ctx, scene_id) =
        setup_clip_room(Some((50.0, 50.0)), Some(wall_sys), false).await;
    let now = crate::ws::time::now_millis();
    register_timeline(
        &room,
        Uuid::from_u128(0xE002),
        obs_ctx.user_id,
        scene_id,
        now,
        vec![VisionSample {
            t_ms: 0.0,
            polygons: band(100.0, 300.0),
        }],
    )
    .await;
    let frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(1),
        token_id: Uuid::from_u128(2),
        mover: Uuid::from_u128(0xAABB),
        scene: scene_id,
        start_server_ms: (now + 50) as f64,
        duration_ms: 400.0,
        stop: [250.0, 50.0],
        samples: vec![
            PosSample {
                t_ms: 0.0,
                pos: [150.0, 50.0],
            },
            PosSample {
                t_ms: 200.0,
                pos: [250.0, 50.0],
            },
        ],
        mover_vision: None,
        cost: Some(2.0),
        truncated: Some(false),
    };
    let out = clip_move_stream(&frame, &gm_ctx, Some(obs_ctx), &room)
        .await
        .expect("target sees both");
    let ServerMsg::MoveStream { samples, cost, .. } = out else {
        panic!()
    };
    assert_eq!(samples.len(), 2);
    assert_eq!(
        cost, None,
        "a see-as clip narrows the GM to observer secrecy"
    );
}

/// A `Sink<Message>` that collects every sent frame into a `Vec`, for tests that inspect
/// serialized output directly rather than driving a real socket.
struct CollectingSink(Vec<Message>);
impl futures_util::Sink<Message> for CollectingSink {
    type Error = std::convert::Infallible;
    fn poll_ready(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn start_send(self: std::pin::Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        self.get_mut().0.push(item);
        Ok(())
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn e2e_replay_redacts_a_field_that_was_gm_only_at_commit_after_the_override_is_later_widened()
{
    // A field hidden (GmOnly) across several historical writes, then WIDENED to fully
    // public in a LATER command, must still redact its intermediate historical values on
    // replay — reading the current value as public does not make its whole secret evolution
    // public. This is the discriminating shape: by the time redaction runs, hidden_current
    // is EMPTY (the override was widened), so ONLY hidden_commit (the commit-time override
    // set) keeps the historical value hidden; an implementation that redacted against current
    // policy alone would leak it here.
    use crate::data::command::{FieldChange, Operation};
    use crate::data::document::{DocRole, PermissionSet, WorldRole};
    use crate::data::membership::PermissionContext;
    use crate::data::sqlite::SqliteRepository;

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
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };

    let mut perms = PermissionSet {
        default: DocRole::Observer,
        ..Default::default()
    };
    perms.property_overrides.insert(
        "/system/secret".into(),
        crate::data::document::Visibility::GmOnly,
    );
    let mut d = crate::data::document::tests::world_scoped_doc(w.id, Uuid::new_v4(), "actor");
    d.permissions = perms;
    d.system = serde_json::json!({ "secret": "S0" });
    let doc_id = d.id;
    repo.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create { doc: d }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Historical, intermediate value — GmOnly at commit — this is the value that must
    // never reach the player, at any resync, no matter how the override later changes.
    repo.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Update {
            doc_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/system/secret".into(),
                old: serde_json::json!("S0"),
                new: serde_json::json!("S1_NEVER_RELEASED"),
            }],
        }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // The GM later widens the override to All — the CURRENT value becomes public, but the
    // historical S1_NEVER_RELEASED value must remain hidden on replay.
    repo.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Update {
            doc_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/permissions/property_overrides/~1system~1secret".into(),
                old: serde_json::json!("gm_only"),
                new: serde_json::json!("all"),
            }],
        }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let reg = crate::ws::room::RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let (frames, _src) = room.resync_range(&repo, 1).await.unwrap();
    let player_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let world_defaults = crate::data::document::WorldCapDefaults::default();
    let mut sink = CollectingSink(Vec::new());
    for f in &frames {
        send_room_event(&mut sink, &repo, &room, &player_ctx, &world_defaults, f)
            .await
            .unwrap();
    }
    let mut saw_the_permission_widening_op = false;
    for msg in &sink.0 {
        let text = match msg {
            Message::Text(t) => t.as_str(),
            _ => continue,
        };
        let v: serde_json::Value = serde_json::from_str(text).unwrap();
        if v["type"] != "event" {
            continue;
        }
        for op in v["command"]["ops"].as_array().unwrap() {
            if op["op"] != "update" {
                continue;
            }
            for ch in op["changes"].as_array().unwrap() {
                if ch["path"] == "/permissions/property_overrides/~1system~1secret" {
                    saw_the_permission_widening_op = true;
                }
                assert_ne!(
                    ch["new"], "S1_NEVER_RELEASED",
                    "a value that was GmOnly at commit must stay hidden even after the \
                     override is later widened to All"
                );
            }
        }
    }
    assert!(
        saw_the_permission_widening_op,
        "sanity check: the widening command itself must be visible to the player \
         (only the earlier GmOnly value must stay hidden)"
    );
}
