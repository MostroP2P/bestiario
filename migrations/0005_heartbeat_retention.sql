-- Heartbeat retention (docs/SPEC.md §8.1 step 6a), applied to what was
-- archived before the rule existed.
--
-- mostrod republishes its relay list (10002) about once a minute and its info
-- (38385) and rates (30078) about every five minutes. Every copy used to be
-- archived, which grew one month of production data past a gigabyte and
-- left the replica too large to restore on the instance that writes it.
--
-- The same two rules the pipeline now applies at ingest:
--   * a 10002 or 38385 whose content and tags (in any order) repeat the
--     version immediately before it (same publisher, kind and address) is
--     dropped;
--   * of the 30078 snapshots a publisher sent within one hour of
--     `published_at`, only the first is kept.
--
-- Deletes only; the space comes back when the pool reclaims it at startup
-- (`db::reclaim_space`), since VACUUM cannot run inside a migration.

CREATE TEMP TABLE redundant_heartbeats (id TEXT PRIMARY KEY);

INSERT INTO redundant_heartbeats (id)
SELECT id FROM (
  SELECT
    id,
    content,
    tags,
    LAG(content) OVER w AS previous_content,
    LAG(tags) OVER w AS previous_tags
  FROM (
    -- Tags compared as a set: mostrod keeps its relays in a hash set, so an
    -- unchanged relay list is republished in a different order every time.
    SELECT
      id, pubkey, kind, d_tag, created_at,
      json_extract(raw_json, '$.content') AS content,
      (SELECT group_concat(value, char(31))
         FROM (SELECT value FROM json_each(raw_json, '$.tags') ORDER BY value)) AS tags
    FROM events
    WHERE kind IN (10002, 38385)
  )
  WINDOW w AS (PARTITION BY pubkey, kind, d_tag ORDER BY created_at, id)
)
WHERE content = previous_content AND tags = previous_tags;

INSERT OR IGNORE INTO redundant_heartbeats (id)
SELECT event_id FROM (
  SELECT
    event_id,
    ROW_NUMBER() OVER (
      PARTITION BY pubkey, published_at - (published_at % 3600)
      ORDER BY published_at, event_id
    ) AS position
  FROM rates
)
WHERE position > 1;

DELETE FROM instance_info WHERE event_id IN (SELECT id FROM redundant_heartbeats);
DELETE FROM rates WHERE event_id IN (SELECT id FROM redundant_heartbeats);
DELETE FROM events WHERE id IN (SELECT id FROM redundant_heartbeats);

DROP TABLE redundant_heartbeats;
