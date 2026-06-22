-- Per-(scene, player) explored fog-of-war memory (M9c). NOT a document: explored
-- area is per-recipient SECRET memory (documents broadcast to all members), so it
-- lives in its own table and is dispatched per-recipient over the `vision` channel.
-- `cells` is a serialized sparse set of grid-cell coords (8 bytes/cell: i32 i, i32 j,
-- little-endian), bounded by O(explored area) and accumulated monotonically (revisits
-- add nothing). world_id is denormalized for world-scoped cleanup.
CREATE TABLE explored_fog (
  world_id  TEXT NOT NULL,
  scene_id  TEXT NOT NULL,
  user_id   TEXT NOT NULL,
  cells     BLOB NOT NULL,
  PRIMARY KEY (scene_id, user_id)
);
