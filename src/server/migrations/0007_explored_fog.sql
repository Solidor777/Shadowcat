-- Per-(scene, player) explored fog-of-war memory (M9c). NOT a document: explored
-- area is per-recipient SECRET memory (documents broadcast to all members), so it
-- lives in its own table and is dispatched per-recipient over the `vision` channel.
-- `cells` is a serialized sparse set of grid-cell coords (8 bytes/cell: i32 i, i32 j,
-- little-endian), bounded by O(explored area) and accumulated monotonically (revisits
-- add nothing). world_id is denormalized so a future world/scene-deletion path can purge
-- rows by world (no such path exists yet — worlds are not deletable; rows orphan harmlessly
-- since reads key on the exact (scene_id, user_id) UUIDs, which are never reused). See TODO.md.
CREATE TABLE explored_fog (
  world_id  TEXT NOT NULL,
  scene_id  TEXT NOT NULL,
  user_id   TEXT NOT NULL,
  cells     BLOB NOT NULL,
  PRIMARY KEY (scene_id, user_id)
);
