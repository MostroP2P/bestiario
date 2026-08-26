//! Every published version of an order, and the projection derived from them.
//!
//! Two tables with one rule between them: `order_versions` is append-only and
//! authoritative, `orders` holds nothing that cannot be recomputed from it
//! (`docs/SPEC.md` §4). [`refresh_projection`] is that recomputation, and
//! `rebuild` in phase 1c is nothing but a loop over it.
//!
//! # Why the projection is recomputed rather than patched
//!
//! Backfill walks *backwards* (SPEC §8.2), so the `success` version of an
//! order routinely lands before the `pending` one it succeeds. A projection
//! updated in place would have to know whether the version it just saw is
//! newer than the one already stored; recomputing from the whole history
//! makes arrival order stop mattering at all.

use sqlx::{Executor, Sqlite};

use crate::ingest::parse::order::{Direction, FiatAmount, OrderVersion, Status};
use crate::network::Network;

use super::decode;

/// One row of `orders`: an order as it currently stands.
#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    pub order_id: String,
    pub pubkey: String,
    /// `created_at` of the earliest version seen.
    pub first_seen_at: i64,
    /// `created_at` of the latest version seen.
    pub last_updated_at: i64,
    pub final_status: Status,
    pub direction: Direction,
    pub fiat_code: String,
    pub amount_sats: i64,
    /// `None` for a range order, which names no single amount.
    pub fiat_amount: Option<f64>,
    pub payment_methods: Vec<String>,
    pub premium: f64,
    pub network: Option<Network>,
    /// `created_at` of the *first* version to reach `success`.
    pub success_at: Option<i64>,
    /// `created_at` of the *first* version to reach `canceled`.
    pub canceled_at: Option<i64>,
}

/// Stores one published version, ignoring a version already known.
///
/// Idempotent by event id: the dedup gate of SPEC §8.1 step 6 normally stops a
/// repeat before it reaches here, but `rebuild --from-raw` replays the version
/// tables directly and must be able to run twice.
pub async fn insert_version<'e, E>(executor: E, version: &OrderVersion) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let (fiat_amount, fiat_min, fiat_max) = match version.fiat {
        FiatAmount::Fixed(amount) => (Some(amount), None, None),
        FiatAmount::Range { min, max } => (None, Some(min), Some(max)),
    };

    sqlx::query(
        "INSERT OR IGNORE INTO order_versions (
             event_id, order_id, pubkey, created_at, kind, status, fiat_code, amount_sats,
             fiat_amount, fiat_min, fiat_max, payment_methods, premium, network, expires_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&version.event_id)
    .bind(&version.order_id)
    .bind(&version.pubkey)
    .bind(version.created_at)
    .bind(version.direction.as_str())
    .bind(version.status.as_str())
    .bind(&version.fiat_code)
    .bind(version.amount_sats)
    .bind(fiat_amount)
    .bind(fiat_min)
    .bind(fiat_max)
    .bind(version.payment_methods.join(","))
    .bind(version.premium)
    .bind(version.network.map(Network::as_str))
    .bind(version.expires_at)
    .execute(executor)
    .await?;

    Ok(())
}

/// Recomputes the `orders` row for `order_id` from every version stored.
///
/// An order with no versions leaves no row: `rebuild --from-raw` empties the
/// version tables first, and a refresh that wrote an empty row would resurrect
/// an order that no longer exists.
///
/// The mutable fields come from the latest version; `success_at` and
/// `canceled_at` from the *first* version to reach that status, because an
/// instance republishes a status when some other field changes and the sale
/// happened at the first one.
pub async fn refresh_projection<'e, E>(executor: E, order_id: &str) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO orders (
             order_id, pubkey, first_seen_at, last_updated_at, final_status, kind, fiat_code,
             amount_sats, fiat_amount, payment_methods, premium, network, success_at, canceled_at
         )
         SELECT latest.order_id, latest.pubkey, span.first_seen_at, span.last_updated_at,
                latest.status, latest.kind, latest.fiat_code, latest.amount_sats,
                latest.fiat_amount, latest.payment_methods, latest.premium, latest.network,
                span.success_at, span.canceled_at
         FROM (
             SELECT * FROM order_versions WHERE order_id = ?1
             ORDER BY created_at DESC, event_id DESC LIMIT 1
         ) AS latest
         JOIN (
             SELECT MIN(created_at) AS first_seen_at,
                    MAX(created_at) AS last_updated_at,
                    MIN(CASE WHEN status = 'success' THEN created_at END) AS success_at,
                    MIN(CASE WHEN status = 'canceled' THEN created_at END) AS canceled_at
             FROM order_versions WHERE order_id = ?1
         ) AS span
         -- `WHERE true` is required, not decoration: without it SQLite reads
         -- the following ON as another join condition rather than as the
         -- upsert clause, and refuses the statement.
         WHERE true
         ON CONFLICT(order_id) DO UPDATE SET
             pubkey = excluded.pubkey,
             first_seen_at = excluded.first_seen_at,
             last_updated_at = excluded.last_updated_at,
             final_status = excluded.final_status,
             kind = excluded.kind,
             fiat_code = excluded.fiat_code,
             amount_sats = excluded.amount_sats,
             fiat_amount = excluded.fiat_amount,
             payment_methods = excluded.payment_methods,
             premium = excluded.premium,
             network = excluded.network,
             success_at = excluded.success_at,
             canceled_at = excluded.canceled_at",
    )
    .bind(order_id)
    .execute(executor)
    .await?;

    Ok(())
}

/// The projected order, if any version of it has been seen.
pub async fn find<'e, E>(executor: E, order_id: &str) -> Result<Option<Order>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query_as::<_, OrderRow>(
        "SELECT order_id, pubkey, first_seen_at, last_updated_at, final_status, kind, fiat_code,
                amount_sats, fiat_amount, payment_methods, premium, network, success_at, canceled_at
         FROM orders WHERE order_id = ?",
    )
    .bind(order_id)
    .fetch_optional(executor)
    .await?;

    row.map(OrderRow::into_order).transpose()
}

/// Every stored version of `order_id`, oldest first.
pub async fn versions<'e, E>(executor: E, order_id: &str) -> Result<Vec<OrderVersion>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, VersionRow>(
        "SELECT event_id, order_id, pubkey, created_at, kind, status, fiat_code, amount_sats,
                fiat_amount, fiat_min, fiat_max, payment_methods, premium, network, expires_at
         FROM order_versions WHERE order_id = ? ORDER BY created_at, event_id",
    )
    .bind(order_id)
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(VersionRow::into_version)
    .collect()
}

/// The `orders` row exactly as SQLite holds it, before the enums are restored.
#[derive(sqlx::FromRow)]
struct OrderRow {
    order_id: String,
    pubkey: String,
    first_seen_at: i64,
    last_updated_at: i64,
    final_status: String,
    kind: String,
    fiat_code: String,
    amount_sats: i64,
    fiat_amount: Option<f64>,
    payment_methods: String,
    premium: f64,
    network: Option<String>,
    success_at: Option<i64>,
    canceled_at: Option<i64>,
}

impl OrderRow {
    fn into_order(self) -> Result<Order, sqlx::Error> {
        Ok(Order {
            order_id: self.order_id,
            pubkey: self.pubkey,
            first_seen_at: self.first_seen_at,
            last_updated_at: self.last_updated_at,
            final_status: decode("final_status", Status::parse(&self.final_status))?,
            direction: decode("kind", Direction::parse(&self.kind))?,
            fiat_code: self.fiat_code,
            amount_sats: self.amount_sats,
            fiat_amount: self.fiat_amount,
            payment_methods: split_methods(&self.payment_methods),
            premium: self.premium,
            network: decode("network", optional_network(self.network.as_deref()))?,
            success_at: self.success_at,
            canceled_at: self.canceled_at,
        })
    }
}

/// The `order_versions` row exactly as SQLite holds it.
#[derive(sqlx::FromRow)]
struct VersionRow {
    event_id: String,
    order_id: String,
    pubkey: String,
    created_at: i64,
    kind: String,
    status: String,
    fiat_code: String,
    amount_sats: i64,
    fiat_amount: Option<f64>,
    fiat_min: Option<f64>,
    fiat_max: Option<f64>,
    payment_methods: String,
    premium: f64,
    network: Option<String>,
    expires_at: i64,
}

impl VersionRow {
    fn into_version(self) -> Result<OrderVersion, sqlx::Error> {
        let fiat = match (self.fiat_amount, self.fiat_min, self.fiat_max) {
            (Some(amount), _, _) => FiatAmount::Fixed(amount),
            (None, Some(min), Some(max)) => FiatAmount::Range { min, max },
            (None, _, _) => {
                return Err(sqlx::Error::Decode(
                    "an order version has neither a fiat amount nor a range".into(),
                ));
            }
        };

        Ok(OrderVersion {
            event_id: self.event_id,
            order_id: self.order_id,
            pubkey: self.pubkey,
            created_at: self.created_at,
            direction: decode("kind", Direction::parse(&self.kind))?,
            status: decode("status", Status::parse(&self.status))?,
            fiat_code: self.fiat_code,
            amount_sats: self.amount_sats,
            fiat,
            payment_methods: split_methods(&self.payment_methods),
            premium: self.premium,
            network: decode("network", optional_network(self.network.as_deref()))?,
            expires_at: self.expires_at,
        })
    }
}

/// The csv the column holds, back into the list the parser produced.
///
/// An empty column is no methods rather than one empty method: `"".split(',')`
/// yields a single empty string, which would show up as a payment method
/// named after nothing.
fn split_methods(csv: &str) -> Vec<String> {
    csv.split(',')
        .map(str::trim)
        .filter(|method| !method.is_empty())
        .map(str::to_string)
        .collect()
}

/// The stored `network`, if the column is not NULL.
fn optional_network(stored: Option<&str>) -> Result<Option<Network>, String> {
    match stored {
        None => Ok(None),
        Some(wire) => Network::from_wire(wire)
            .map(Some)
            .ok_or_else(|| format!("`{wire}` is not a network")),
    }
}

#[cfg(test)]
mod tests;
