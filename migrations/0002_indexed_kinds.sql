-- Which kinds have actually been asked for, and how far back.
--
-- A kind absent from `events` is ambiguous on its own: either nobody
-- published one, or nobody ever requested them. `backfill --kind 38383` is
-- a supported mode and leaves every other kind untouched, so reading an
-- absent kind as a confirmed zero would report observed zeros for history
-- nothing ever looked at.
--
-- A row here is the explicit answer: the kind was requested, and
-- `indexed_from` is the oldest `created_at` any request for it reached back
-- to — 0 for a walk that asked a relay for everything it holds. The floor
-- only ever deepens: a later, narrower run does not unlearn an earlier
-- deeper one.
CREATE TABLE indexed_kinds (
  kind          INTEGER PRIMARY KEY,
  indexed_from  INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);
