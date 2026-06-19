-- Per-user opaque UI session state (active world, active tab, locale, ...).
-- The server stores it verbatim and validates only object-shape + size cap;
-- the client owns the structure. NULL until the first PUT.
ALTER TABLE users ADD COLUMN ui_state TEXT;
