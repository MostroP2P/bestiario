-- When each order was created, as published in the NIP-69 `created_at` tag
-- of kind 38383 (MostroP2P/mostro#971, nostr-protocol/nips#2476).
--
-- The event's own `created_at` is a *revision* time: kind 38383 is
-- addressable, and a relay keeps only the latest state. An order first seen
-- mid-flight — the norm in a backfill — has `first_seen_at` at that later
-- revision. The tag is the same on every revision, so it dates the order
-- even when its early versions were never seen.
--
-- NULL for events without the tag (nodes that predate it). Existing rows
-- stay NULL until `rebuild --from-raw` re-reads them from the archive.
ALTER TABLE order_versions ADD COLUMN order_created_at INTEGER;
ALTER TABLE orders ADD COLUMN order_created_at INTEGER;
