//! Per-world derived scene ECS. Hydrated from documents (#5); never persisted,
//! never authoritative. Holds one hecs entity per scene-entity document so
//! engine-owned systems (M9 vision, M10 pathfinding) can query spatial state.

use std::collections::HashMap;

use uuid::Uuid;

use crate::data::command::{set_pointer, Operation};
use crate::data::document::Document;
use crate::data::membership::PermissionContext;

/// A hydrated scene-entity document, one per hecs entity.
pub struct SceneEntity {
    pub doc: Document,
}

/// A document is scene runtime state if it is a scene or a child of one.
pub fn is_scene_entity(doc: &Document) -> bool {
    doc.doc_type == "scene" || doc.parent_id.is_some()
}

/// The per-world derived world. Writes are serialized by the caller
/// (`Room::publish` under `publish_guard`); reads (derived recompute) take a
/// shared borrow.
pub struct SceneEcs {
    world: hecs::World,
    index: HashMap<Uuid, hecs::Entity>,
    /// Per-world seq of the last command reflected in this ECS. Updated under
    /// the same `scene.write()` lock as the entities in `Room::publish`, so a
    /// reader holding the read lock sees a consistent `(entities, seq)` pair and
    /// the derived `computed_at_seq` watermark can never be below the state it
    /// describes (#2).
    committed_seq: i64,
}

impl SceneEcs {
    pub fn new() -> Self {
        Self {
            world: hecs::World::new(),
            index: HashMap::new(),
            committed_seq: 0,
        }
    }

    /// Hydrate from a document set (scene entities only; others are ignored),
    /// reflecting state as of `seq` (the world's current seq at hydration).
    pub fn from_documents(docs: Vec<Document>, seq: i64) -> Self {
        let mut ecs = Self::new();
        ecs.committed_seq = seq;
        for doc in docs {
            if is_scene_entity(&doc) {
                let id = doc.id;
                let e = ecs.world.spawn((SceneEntity { doc },));
                ecs.index.insert(id, e);
            }
        }
        ecs
    }

    /// Record the seq of the command just applied (called under the write lock).
    pub fn set_committed_seq(&mut self, seq: i64) {
        self.committed_seq = seq;
    }

    /// The seq the ECS currently reflects — emitted as `computed_at_seq`.
    pub fn committed_seq(&self) -> i64 {
        self.committed_seq
    }

    /// Reflect one already-committed authoritative op into the derived world.
    pub fn apply_op(&mut self, op: &Operation) {
        match op {
            Operation::Create { doc } if is_scene_entity(doc) => {
                if let Some(&e) = self.index.get(&doc.id) {
                    let _ = self.world.despawn(e);
                }
                let e = self.world.spawn((SceneEntity { doc: doc.clone() },));
                self.index.insert(doc.id, e);
            }
            Operation::Update { doc_id, changes } => {
                // An Update never changes scene-entity membership: `parent_id`
                // and `doc_type` are envelope fields, immutable via field-path
                // Update (`required_cap_for_path` maps them to no capability).
                // INVARIANT: if `parent_id` becomes mutable, this arm must
                // re-evaluate `is_scene_entity` and spawn/despawn accordingly.
                // TODO: re-evaluate is_scene_entity here once parent_id is mutable.
                if let Some(&e) = self.index.get(doc_id) {
                    if let Ok(mut comp) = self.world.get::<&mut SceneEntity>(e) {
                        // Mirror the same field-path changes apply_intent applied
                        // to SQLite, via Value round-trip (server stays
                        // structural-only; no semantic interpretation).
                        if let Ok(mut v) = serde_json::to_value(&comp.doc) {
                            for ch in changes {
                                let _ = set_pointer(&mut v, &ch.path, ch.new.clone());
                            }
                            if let Ok(updated) = serde_json::from_value::<Document>(v) {
                                comp.doc = updated;
                            }
                        }
                    }
                }
            }
            Operation::Delete { doc } => {
                if let Some(e) = self.index.remove(&doc.id) {
                    let _ = self.world.despawn(e);
                }
            }
            Operation::Create { .. } => {} // non-scene document: ignored
        }
    }

    /// Count of hydrated scene entities (the M8a identity payload source).
    pub fn entity_count(&self) -> usize {
        self.index.len()
    }

    /// Engine-owned movement collision (M9a, the second ARCHITECTURE #6 geometric
    /// exception). True if moving token `token_id` from its committed `system.(x,y)` to
    /// `(new_x,new_y)` crosses any `blocksMove` wall in the token's scene. Reads the
    /// authoritative current position from the ECS — never the client's claimed pre-image.
    pub fn blocks_move(&self, token_id: Uuid, new_x: f64, new_y: f64) -> bool {
        let (x0, y0, scene) = {
            let Some(&e) = self.index.get(&token_id) else {
                return false;
            };
            let Ok(tok) = self.world.get::<&SceneEntity>(e) else {
                return false;
            };
            if tok.doc.doc_type != "token" {
                return false;
            }
            let Some(x0) = sys_f64(&tok.doc, "/x") else {
                return false;
            };
            let Some(y0) = sys_f64(&tok.doc, "/y") else {
                return false;
            };
            let Some(scene) = tok.doc.parent_id else {
                return false;
            };
            (x0, y0, scene)
        };
        let a0 = (x0, y0);
        let a1 = (new_x, new_y);
        for w in self.world.query::<&SceneEntity>().iter() {
            if w.doc.doc_type != "wall" || w.doc.parent_id != Some(scene) {
                continue;
            }
            if w.doc
                .system
                .pointer("/blocksMove")
                .and_then(|v| v.as_bool())
                != Some(true)
            {
                continue;
            }
            let (Some(x1), Some(y1), Some(x2), Some(y2)) = (
                sys_f64(&w.doc, "/seg/x1"),
                sys_f64(&w.doc, "/seg/y1"),
                sys_f64(&w.doc, "/seg/x2"),
                sys_f64(&w.doc, "/seg/y2"),
            ) else {
                continue;
            };
            if segments_cross(a0, a1, (x1, y1), (x2, y2)) {
                return true;
            }
        }
        false
    }
}

/// Read an `f64` from a document's opaque `system` body via JSON pointer (ints coerce).
fn sys_f64(doc: &Document, pointer: &str) -> Option<f64> {
    doc.system.pointer(pointer).and_then(|v| v.as_f64())
}

/// Signed area ×2 of triangle abc; >0 = ccw, <0 = cw, 0 = collinear.
fn orient(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

/// Is `p` within the axis-aligned bounding box of segment `ab` (collinearity assumed)?
fn on_segment(a: (f64, f64), b: (f64, f64), p: (f64, f64)) -> bool {
    p.0 >= a.0.min(b.0) && p.0 <= a.0.max(b.0) && p.1 >= a.1.min(b.1) && p.1 <= a.1.max(b.1)
}

/// Do segments `p1p2` and `p3p4` intersect (proper crossing or a touching endpoint /
/// T-junction)? Source: standard orientation/cross-product segment-intersection test
/// (CLRS "Determining whether two segments intersect"). A move that merely touches a wall
/// counts as blocked (conservative — a token cannot end on or graze a wall).
fn segments_cross(p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), p4: (f64, f64)) -> bool {
    let d1 = orient(p3, p4, p1);
    let d2 = orient(p3, p4, p2);
    let d3 = orient(p1, p2, p3);
    let d4 = orient(p1, p2, p4);
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    (d1 == 0.0 && on_segment(p3, p4, p1))
        || (d2 == 0.0 && on_segment(p3, p4, p2))
        || (d3 == 0.0 && on_segment(p1, p2, p3))
        || (d4 == 0.0 && on_segment(p1, p2, p4))
}

impl Default for SceneEcs {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute a derived payload for `channel` from the scene ECS, for one
/// recipient. Returns `None` for unknown channels (→ SceneError). `ctx` is
/// accepted so M9 vision can derive per recipient; the identity payload is
/// non-sensitive and global.
// `ecs` is read only by the debug-gated channel arm; it is genuinely unused in
// release until M9's vision channel consumes it.
#[cfg_attr(not(debug_assertions), allow(unused_variables))]
pub fn compute_derived(
    channel: &str,
    ecs: &SceneEcs,
    _ctx: &PermissionContext,
) -> Option<serde_json::Value> {
    match channel {
        // Seam proof only; replaced when M9 vision lands. Absent in release.
        #[cfg(debug_assertions)]
        "identity" => Some(serde_json::json!({ "entity_count": ecs.entity_count() })),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(id: u128, parent: Option<u128>, ty: &str) -> Document {
        let mut d = crate::data::document::tests::world_scoped_doc(
            Uuid::from_u128(9),
            Uuid::from_u128(id),
            ty,
        );
        d.parent_id = parent.map(Uuid::from_u128);
        d
    }

    #[test]
    fn hydrate_counts_scene_entities_only() {
        let ecs = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                doc(11, Some(10), "token"),
                doc(99, None, "actor"), // not a scene entity → ignored
            ],
            0,
        );
        assert_eq!(ecs.entity_count(), 2);
        assert_eq!(ecs.committed_seq(), 0);
    }

    #[test]
    fn apply_op_create_update_delete() {
        let mut ecs = SceneEcs::new();
        ecs.apply_op(&Operation::Create {
            doc: doc(11, Some(10), "token"),
        });
        assert_eq!(ecs.entity_count(), 1);
        ecs.apply_op(&Operation::Update {
            doc_id: Uuid::from_u128(11),
            changes: vec![crate::data::command::FieldChange {
                path: "/system/x".into(),
                old: json!(null),
                new: json!(5),
            }],
        });
        let e = ecs.index[&Uuid::from_u128(11)];
        let comp = ecs.world.get::<&SceneEntity>(e).unwrap();
        assert_eq!(comp.doc.system["x"], json!(5));
        drop(comp);
        ecs.apply_op(&Operation::Delete {
            doc: doc(11, Some(10), "token"),
        });
        assert_eq!(ecs.entity_count(), 0);
    }

    fn entity_doc(id: u128, parent: u128, ty: &str, system: serde_json::Value) -> Document {
        let mut d = doc(id, Some(parent), ty);
        d.system = system;
        d
    }

    #[test]
    fn segments_cross_truth_table() {
        assert!(segments_cross(
            (0.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
            (10.0, 0.0)
        )); // X crossing
        assert!(!segments_cross(
            (0.0, 0.0),
            (10.0, 0.0),
            (0.0, 5.0),
            (10.0, 5.0)
        )); // parallel
        assert!(!segments_cross(
            (0.0, 0.0),
            (5.0, 0.0),
            (10.0, 0.0),
            (15.0, 0.0)
        )); // collinear disjoint
        assert!(segments_cross(
            (0.0, 0.0),
            (5.0, 0.0),
            (5.0, 0.0),
            (5.0, 5.0)
        )); // touching endpoint (T)
        assert!(segments_cross(
            (0.0, 0.0),
            (5.0, 10.0),
            (0.0, 5.0),
            (10.0, 5.0)
        )); // crossing
    }

    #[test]
    fn blocks_move_only_for_crossing_blocksmove_walls_in_scene() {
        let scene = 10u128;
        let mut ecs = SceneEcs::from_documents(
            vec![
                doc(scene, None, "scene"),
                entity_doc(11, scene, "token", json!({ "x": 0, "y": 0 })),
                entity_doc(
                    12,
                    scene,
                    "wall",
                    json!({ "seg": {"x1":0,"y1":10,"x2":10,"y2":0}, "blocksMove": true }),
                ),
            ],
            0,
        );
        // (0,0)->(10,10) crosses the wall line x+y=10.
        assert!(ecs.blocks_move(Uuid::from_u128(11), 10.0, 10.0));
        // (0,0)->(1,1) stays on the near side (sum 2 < 10) → no crossing.
        assert!(!ecs.blocks_move(Uuid::from_u128(11), 1.0, 1.0));
        // A blocksMove:false wall does not block.
        ecs.apply_op(&Operation::Update {
            doc_id: Uuid::from_u128(12),
            changes: vec![crate::data::command::FieldChange {
                path: "/system/blocksMove".into(),
                old: json!(true),
                new: json!(false),
            }],
        });
        assert!(!ecs.blocks_move(Uuid::from_u128(11), 10.0, 10.0));
    }

    #[test]
    fn blocks_move_ignores_walls_in_other_scenes_and_non_tokens() {
        let ecs = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                doc(20, None, "scene"),
                entity_doc(11, 10, "token", json!({ "x": 0, "y": 0 })),
                entity_doc(
                    22,
                    20,
                    "wall",
                    json!({ "seg": {"x1":0,"y1":10,"x2":10,"y2":0}, "blocksMove": true }),
                ),
            ],
            0,
        );
        // The wall is in scene 20; the token is in scene 10 → no block.
        assert!(!ecs.blocks_move(Uuid::from_u128(11), 10.0, 10.0));
        // An unknown / non-token id never blocks.
        assert!(!ecs.blocks_move(Uuid::from_u128(22), 5.0, 5.0));
    }

    #[test]
    fn committed_seq_tracks_last_applied_command() {
        // The watermark is the seq emitted as `computed_at_seq`; it advances only
        // via set_committed_seq, called under the same write lock as apply_op so a
        // reader never sees a watermark ahead of (or behind) the entities.
        let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 3);
        assert_eq!(ecs.committed_seq(), 3);
        ecs.set_committed_seq(7);
        assert_eq!(ecs.committed_seq(), 7);
    }
}
