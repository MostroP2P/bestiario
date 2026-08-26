//! Dev fees, and which of them actually moved money.
//!
//! A dev fee (kind 8383) is the only proof from outside that a trade
//! completed and settled (`docs/SPEC.md` §2.2), which is why this table has no
//! foreign key on `order_id`: relay retention for fees is a year against
//! roughly a fortnight for the orders they name, so a fee whose 38383 has
//! already expired is the normal case during backfill and not a broken row.
//!
//! # The duplicate flag
//!
//! mostrod issue #620 pays the dev fee twice for the same order. Both events
//! are real, both are signed, and both are kept — dropping one would lose an
//! event the raw table is meant to preserve — but only one of them is a
//! settlement. `is_duplicate` marks the rest so that every later query can say
//! `WHERE is_duplicate = 0` and stop worrying about it.
//!
//! The earliest fee is the canonical one, and the flags are *recomputed* for
//! the whole order on every insert rather than decided when a row arrives.
//! Backfill walks backwards (SPEC §8.2), so the duplicate routinely lands
//! before the payment it duplicates; deciding on arrival would flag whichever
//! one the relay happened to send second.

use sqlx::{Acquire, Executor, Sqlite};

use crate::ingest::parse::dev_fee::DevFee;
use crate::network::Network;

use super::decode;

/// A stored fee, and whether it is the one that counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDevFee {
    pub fee: DevFee,
    /// `true` when an earlier fee already exists for the same order.
    pub is_duplicate: bool,
}

/// Stores `fee` and refreshes the duplicate flags of its order.
///
/// Idempotent: re-ingesting the same event leaves one row, still unflagged.
///
/// This is the one repository function that takes an [`Acquire`] rather than
/// an `Executor`, because it issues two statements. An `Executor` is consumed
/// by a single `execute`, and the pipeline's executor inside a transaction is
/// a `&mut` reference, which cannot be copied for a second use. Acquiring once
/// and reborrowing keeps both statements on the caller's own connection —
/// against a pool as much as inside the transaction of SPEC §8.1 step 7,
/// which is what makes them atomic when it matters.
pub async fn insert<'a, A>(acquirer: A, fee: &DevFee) -> Result<(), sqlx::Error>
where
    A: Acquire<'a, Database = Sqlite>,
{
    let mut connection = acquirer.acquire().await?;

    sqlx::query(
        "INSERT OR IGNORE INTO dev_fees (
             event_id, pubkey, order_id, amount_sats, payment_hash, destination, network,
             created_at, is_duplicate
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0)",
    )
    .bind(&fee.event_id)
    .bind(&fee.pubkey)
    .bind(&fee.order_id)
    .bind(fee.amount_sats)
    .bind(&fee.payment_hash)
    .bind(&fee.destination)
    .bind(fee.network.map(Network::as_str))
    .bind(fee.created_at)
    .execute(&mut *connection)
    .await?;

    refresh_duplicates(&mut *connection, &fee.order_id).await
}

/// Recomputes `is_duplicate` for every fee of `order_id`.
///
/// The earliest `created_at` is canonical, tie-broken by event id so that two
/// fees published in the same second still settle on the same answer whichever
/// order they arrive in.
pub async fn refresh_duplicates<'e, E>(executor: E, order_id: &str) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "UPDATE dev_fees SET is_duplicate = CASE
             WHEN event_id = (
                 SELECT event_id FROM dev_fees WHERE order_id = ?1
                 ORDER BY created_at, event_id LIMIT 1
             ) THEN 0 ELSE 1
         END
         WHERE order_id = ?1",
    )
    .bind(order_id)
    .execute(executor)
    .await?;

    Ok(())
}

/// Every fee stored for `order_id`, oldest first — the canonical one leading.
pub async fn for_order<'e, E>(executor: E, order_id: &str) -> Result<Vec<StoredDevFee>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, FeeRow>(
        "SELECT event_id, pubkey, order_id, amount_sats, payment_hash, destination, network,
                created_at, is_duplicate
         FROM dev_fees WHERE order_id = ? ORDER BY created_at, event_id",
    )
    .bind(order_id)
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(FeeRow::into_stored)
    .collect()
}

/// The `dev_fees` row exactly as SQLite holds it.
#[derive(sqlx::FromRow)]
struct FeeRow {
    event_id: String,
    pubkey: String,
    order_id: String,
    amount_sats: i64,
    payment_hash: String,
    destination: Option<String>,
    network: Option<String>,
    created_at: i64,
    is_duplicate: i64,
}

impl FeeRow {
    fn into_stored(self) -> Result<StoredDevFee, sqlx::Error> {
        let network = match self.network.as_deref() {
            None => None,
            Some(wire) => Some(decode(
                "network",
                Network::from_wire(wire).ok_or_else(|| format!("`{wire}` is not a network")),
            )?),
        };

        Ok(StoredDevFee {
            fee: DevFee {
                event_id: self.event_id,
                pubkey: self.pubkey,
                order_id: self.order_id,
                created_at: self.created_at,
                amount_sats: self.amount_sats,
                payment_hash: self.payment_hash,
                destination: self.destination,
                network,
            },
            is_duplicate: self.is_duplicate != 0,
        })
    }
}

/// Every order a fee has been seen for.
pub async fn order_ids<'e, E>(executor: E) -> Result<Vec<String>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_scalar::<_, String>("SELECT DISTINCT order_id FROM dev_fees ORDER BY order_id")
        .fetch_all(executor)
        .await
}

/// Empties the table; see [`super::orders::clear_versions`].
pub async fn clear<'e, E>(executor: E) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query("DELETE FROM dev_fees")
        .execute(executor)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
