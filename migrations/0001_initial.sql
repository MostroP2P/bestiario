-- Initial schema. Transcribed from docs/SPEC.md §4, which stays the source of
-- truth: this file is generated from the fenced SQL block in that section and
-- should be regenerated rather than edited in place when the spec changes.
--
-- Conventions: timestamps are unix seconds stored as INTEGER, sats are
-- INTEGER, fiat amounts are REAL.
--
-- Two deliberate absences of a foreign key:
--   * dev_fees.order_id has none, because an 8383 may arrive before -- or
--     entirely without -- the 38383 it names: relay retention for dev fees is
--     a year against roughly a fortnight for orders (§2.2).
--   * dispute_versions has no order reference at all, because kind 38386 does
--     not publish one (§2.3).

-- Raw event: dedup by id, signature already verified. Source of truth for re-derivation.
CREATE TABLE events (
  id          TEXT PRIMARY KEY,          -- event id hex
  pubkey      TEXT NOT NULL,             -- instance
  kind        INTEGER NOT NULL,
  created_at  INTEGER NOT NULL,
  d_tag       TEXT,                      -- NULL for 8383
  raw_json    TEXT NOT NULL,
  relay_url   TEXT NOT NULL,             -- first relay it was seen on
  seen_at     INTEGER NOT NULL
);
CREATE INDEX events_kind_created ON events(kind, created_at);
CREATE INDEX events_pubkey_kind_d ON events(pubkey, kind, d_tag);

CREATE TABLE instances (
  pubkey        TEXT PRIMARY KEY,
  name          TEXT,
  name_seen_at  INTEGER,
  first_seen_at INTEGER NOT NULL,
  last_seen_at  INTEGER NOT NULL
);

CREATE TABLE instance_names (
  pubkey   TEXT NOT NULL,
  name     TEXT NOT NULL,
  seen_at  INTEGER NOT NULL,
  PRIMARY KEY (pubkey, name)
);

-- Every 38385 version seen.
CREATE TABLE instance_info (
  event_id            TEXT PRIMARY KEY REFERENCES events(id),
  pubkey              TEXT NOT NULL,
  created_at          INTEGER NOT NULL,
  fee                 REAL,              -- fraction, e.g. 0.006
  max_order_amount    INTEGER,
  min_order_amount    INTEGER,
  fiat_currencies     TEXT,              -- csv as published
  mostro_version      TEXT,
  protocol_version    TEXT,
  ln_networks         TEXT,
  bond_enabled        INTEGER
);

-- Every 38383 version seen.
CREATE TABLE order_versions (
  event_id     TEXT PRIMARY KEY REFERENCES events(id),
  order_id     TEXT NOT NULL,            -- d tag
  pubkey       TEXT NOT NULL,
  created_at   INTEGER NOT NULL,
  kind         TEXT NOT NULL,            -- buy | sell
  status       TEXT NOT NULL,            -- pending | in-progress | success | canceled
  fiat_code    TEXT NOT NULL,
  amount_sats  INTEGER NOT NULL,
  fiat_amount  REAL,                     -- NULL if range
  fiat_min     REAL,
  fiat_max     REAL,
  payment_methods TEXT NOT NULL,         -- csv
  premium      REAL NOT NULL,
  network      TEXT,
  expires_at   INTEGER
);
CREATE INDEX order_versions_order ON order_versions(order_id, created_at);

-- Projection: latest known state per order (derivable from order_versions).
CREATE TABLE orders (
  order_id        TEXT PRIMARY KEY,
  pubkey          TEXT NOT NULL,
  first_seen_at   INTEGER NOT NULL,      -- created_at of the first version
  last_updated_at INTEGER NOT NULL,
  final_status    TEXT NOT NULL,
  kind            TEXT NOT NULL,
  fiat_code       TEXT NOT NULL,
  amount_sats     INTEGER NOT NULL,      -- from the latest version
  fiat_amount     REAL,
  payment_methods TEXT NOT NULL,
  premium         REAL NOT NULL,
  network         TEXT,
  success_at      INTEGER,               -- created_at of the success version
  canceled_at     INTEGER
);
CREATE INDEX orders_pubkey_status ON orders(pubkey, final_status);
CREATE INDEX orders_success_at ON orders(success_at);

CREATE TABLE dev_fees (
  event_id     TEXT PRIMARY KEY REFERENCES events(id),
  pubkey       TEXT NOT NULL,
  order_id     TEXT NOT NULL,            -- no FK: may arrive before the order
  amount_sats  INTEGER NOT NULL,
  payment_hash TEXT NOT NULL,
  destination  TEXT,
  network      TEXT,
  created_at   INTEGER NOT NULL,
  is_duplicate INTEGER NOT NULL DEFAULT 0 -- >1 dev fee for the same order
);
CREATE INDEX dev_fees_order ON dev_fees(order_id);

CREATE TABLE dispute_versions (
  event_id     TEXT PRIMARY KEY REFERENCES events(id),
  dispute_id   TEXT NOT NULL,
  pubkey       TEXT NOT NULL,
  created_at   INTEGER NOT NULL,         -- event created_at
  status       TEXT NOT NULL,
  initiator    TEXT,
  opened_at    INTEGER                   -- created_at tag
);
CREATE INDEX dispute_versions_dispute ON dispute_versions(dispute_id, created_at);

CREATE TABLE disputes (                  -- projection: latest state
  dispute_id   TEXT PRIMARY KEY,
  pubkey       TEXT NOT NULL,
  opened_at    INTEGER,
  last_updated_at INTEGER NOT NULL,
  final_status TEXT NOT NULL,
  initiator    TEXT
);

CREATE TABLE rates (
  event_id     TEXT PRIMARY KEY REFERENCES events(id),
  pubkey       TEXT NOT NULL,
  published_at INTEGER NOT NULL,
  source       TEXT,
  rates_json   TEXT NOT NULL             -- {"USD": 50000.0, ...}
);
CREATE INDEX rates_pubkey_time ON rates(pubkey, published_at);

CREATE TABLE relays (
  url          TEXT PRIMARY KEY,
  source       TEXT NOT NULL,            -- config | nip65:<pubkey>
  first_seen_at INTEGER NOT NULL
);

-- Sync cursor for resumption.
CREATE TABLE sync_state (
  relay_url    TEXT NOT NULL,
  kind         INTEGER NOT NULL,
  last_created_at INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  PRIMARY KEY (relay_url, kind)
);
