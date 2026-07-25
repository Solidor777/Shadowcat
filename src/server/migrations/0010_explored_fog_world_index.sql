-- World deletion (B1) purges explored_fog by world_id (the column 0007
-- denormalized for exactly this); index it so the purge is not a full scan.
-- No user_id index: user deletion is a rare admin op and the fog write path
-- is hot — a purge-by-user scan is the right trade.
CREATE INDEX idx_explored_fog_world ON explored_fog(world_id);
