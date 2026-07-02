//! M10f-1 continuous (navmesh) pathfinding adapter. Pure geometry: builds a footprint-inflated
//! `polyanya::Mesh` from a scene's bounds + `blocksMove` wall segments, and queries any-angle
//! routes over it. Engine-owned geometry (ARCHITECTURE §6 exception), mirroring the grid A*
//! router's fail-closed discipline (`scene/pathfinding.rs`) — this checkpoint carries WALLS ONLY;
//! impassable/terrain regions land in M10f-4 (parent spec §7/§10).

#[cfg(test)]
mod smoke {
    // Locks down the real polyanya 0.16 headless API before the adapter is built on top: a bare
    // rectangle with no obstacles, queried start->goal, must return a straight path whose length
    // equals the Euclidean distance.
    #[test]
    fn bare_rectangle_paths_straight_line() {
        let outer = [
            glam::Vec2::new(0.0, 0.0),
            glam::Vec2::new(1000.0, 0.0),
            glam::Vec2::new(1000.0, 1000.0),
            glam::Vec2::new(0.0, 1000.0),
        ];
        let tri = polyanya::Triangulation::from_outer_edges(&outer);
        let mesh = tri.as_navmesh();
        let path = mesh
            .path(glam::Vec2::new(50.0, 50.0), glam::Vec2::new(950.0, 50.0))
            .expect("straight route across an empty rectangle must exist");
        assert!(
            (path.length - 900.0).abs() < 1.0,
            "expected ~900, got {}",
            path.length
        );
        // `Path::path` holds only turning points plus the destination (the start is implicit,
        // per polyanya's `Path` doc comment) — a straight, unobstructed route is a single vertex.
        assert!(!path.path.is_empty(), "path must have at least 1 vertex");
        let last = path.path.last().unwrap();
        assert!(
            (last.x - 950.0).abs() < 1.0 && (last.y - 50.0).abs() < 1.0,
            "last vertex must be the goal, got {:?}",
            last
        );
    }

    // A later checkpoint caches `polyanya::Mesh` behind a `std::sync::Mutex` on `SceneEcs`, which
    // itself lives behind a `tokio::sync::RwLock` shared across connection tasks — this requires
    // `Mesh: Send + Sync`. Assert the bound at the point the dependency enters the tree so a
    // violation surfaces here, not after a cache is already built on top of it.
    #[test]
    fn mesh_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<polyanya::Mesh>();
    }
}
