-- What has been published, so that a later run can say what changed.
--
-- `docs/NOSTR-PUBLICATION.md` §8 requires a `revision` that starts at 1 and
-- increments on every change of a document's figures, and §1.1 keeps no
-- state outside this archive. Publication history is not derivable from the
-- Mostro events: nothing in `events` records that a document was ever
-- published, or under which revision. So it is recorded here.
--
-- What §9.3 refuses to keep is a cache of *signed events* — state that can
-- disagree with the archive it was derived from. These two tables keep no
-- events and no signatures: they record what a past run concluded, and
-- every document is regenerated from the archive on every run regardless.
--
-- Surviving a relay that pruned the index is the point. Reading the last
-- index back off a relay would work until the day a relay dropped it, and
-- then every revision would silently reset to 1 — which is precisely the
-- claim §8 exists to make trustworthy.

-- One row per document address, holding what was last published under it.
CREATE TABLE published_documents (
  d                TEXT PRIMARY KEY,
  -- The hash of the payload, as the index names it. The comparison that
  -- decides whether the next run publishes this document at all.
  hash             TEXT NOT NULL,
  revision         INTEGER NOT NULL,
  -- Unix seconds: when the figures last changed, not when they were last
  -- looked at. A run that skips a document does not move it.
  updated_at       INTEGER NOT NULL,
  -- NULL together, and only on revision 1, which has nothing to restate.
  restated_at      INTEGER,
  restated_because TEXT
);

-- One row per publication run, for the two facts the next run needs in
-- order to say *why* the figures moved (§8): the schema it published under,
-- and the extent of the archive it read.
--
-- Kept as a log rather than a single overwritten row. The next run reads
-- only the latest, but a publication history is what an operator has to
-- consult when a client reports a figure that changed, and there is no
-- other record of it anywhere.
CREATE TABLE publication_runs (
  snapshot_id    TEXT PRIMARY KEY,
  generated_at   INTEGER NOT NULL,
  schema_version INTEGER NOT NULL,
  -- The coverage the run published, both ends; NULL when the archive held
  -- nothing. A first_event_at that has moved earlier since is a backfill
  -- having reached further back, which is one of the four reasons of §8.
  first_event_at INTEGER,
  last_event_at  INTEGER,
  -- How many events the archive held. Coverage alone cannot see a
  -- backfill that found events *inside* a range already covered — the
  -- floor does not move and the figures do — and telling that apart from
  -- a `rebuild` is the difference between the two reasons of §8 that a
  -- client is most likely to act on.
  events         INTEGER NOT NULL
);

CREATE INDEX idx_publication_runs_generated_at ON publication_runs (generated_at DESC);
