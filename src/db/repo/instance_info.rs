//! Every 38385 an instance has published, and the fee that was in force at a
//! given moment.
//!
//! Append-only, with no projection: `instances` already answers "what is this
//! instance called", and everything else a 38385 carries — the fee above all
//! — is only useful *as history*.
//!
//! # Why the whole fee history is kept
//!
//! Phase 3 divides completed volume by the fee in force **when the trade
//! happened** (`docs/SPEC.md` §6.6). An instance that raised its fee last
//! month would otherwise have every trade it ever made valued at the new rate,
//! silently rewriting a year of history. [`fee_in_force`] is that lookup.

use sqlx::{Executor, Sqlite};

use crate::ingest::parse::info::InstanceInfo;

/// Stores one published version, ignoring a version already known.
pub async fn insert_version<'e, E>(executor: E, info: &InstanceInfo) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT OR IGNORE INTO instance_info (
             event_id, pubkey, created_at, fee, max_order_amount, min_order_amount,
             fiat_currencies, mostro_version, protocol_version, ln_networks, bond_enabled
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&info.event_id)
    .bind(&info.pubkey)
    .bind(info.created_at)
    .bind(info.fee)
    .bind(info.max_order_amount)
    .bind(info.min_order_amount)
    .bind(&info.fiat_currencies)
    .bind(&info.mostro_version)
    .bind(&info.protocol_version)
    .bind(&info.ln_networks)
    .bind(info.bond_enabled)
    .execute(executor)
    .await?;

    Ok(())
}

/// The fee `pubkey` had published at `at_ts`, if it had published one by then.
///
/// The most recent version at or before `at_ts` **that names a fee**. A later
/// version that omits the tag is saying nothing about the fee, not that it has
/// dropped to zero — a third of the corpus omits some field or other — so it
/// does not hide the last figure actually published.
///
/// `None` means the instance had published no fee yet at that moment, which is
/// a real answer and not a zero: valuing a trade at a fee nobody announced
/// would invent volume.
///
/// Two versions can share a `created_at`. NIP-01 retains the lexicographically
/// lowest event id in that case, and so does this lookup — a fee that
/// disagreed with the version the relays keep would follow every inferred
/// volume figure derived from it.
pub async fn fee_in_force<'e, E>(
    executor: E,
    pubkey: &str,
    at_ts: i64,
) -> Result<Option<f64>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_scalar::<_, Option<f64>>(
        "SELECT fee FROM instance_info
         WHERE pubkey = ? AND created_at <= ? AND fee IS NOT NULL
         ORDER BY created_at DESC, event_id ASC LIMIT 1",
    )
    .bind(pubkey)
    .bind(at_ts)
    .fetch_optional(executor)
    .await
    .map(Option::flatten)
}

/// Every version `pubkey` has published, oldest first.
pub async fn versions<'e, E>(executor: E, pubkey: &str) -> Result<Vec<InstanceInfo>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, InfoRow>(
        "SELECT event_id, pubkey, created_at, fee, max_order_amount, min_order_amount,
                fiat_currencies, mostro_version, protocol_version, ln_networks, bond_enabled
         FROM instance_info WHERE pubkey = ? ORDER BY created_at, event_id",
    )
    .bind(pubkey)
    .fetch_all(executor)
    .await
    .map(|rows| rows.into_iter().map(InfoRow::into_info).collect())
}

/// The most recent version `pubkey` has published, if any.
pub async fn latest<'e, E>(executor: E, pubkey: &str) -> Result<Option<InstanceInfo>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, InfoRow>(
        "SELECT event_id, pubkey, created_at, fee, max_order_amount, min_order_amount,
                fiat_currencies, mostro_version, protocol_version, ln_networks, bond_enabled
         FROM instance_info WHERE pubkey = ? ORDER BY created_at DESC, event_id ASC LIMIT 1",
    )
    .bind(pubkey)
    .fetch_optional(executor)
    .await
    .map(|row| row.map(InfoRow::into_info))
}

/// The `instance_info` row exactly as SQLite holds it.
///
/// Only `bond_enabled` needs converting: SQLite has no boolean, and the column
/// is an INTEGER that the parser produced from a `bond` tag.
#[derive(sqlx::FromRow)]
struct InfoRow {
    event_id: String,
    pubkey: String,
    created_at: i64,
    fee: Option<f64>,
    max_order_amount: Option<i64>,
    min_order_amount: Option<i64>,
    fiat_currencies: Option<String>,
    mostro_version: Option<String>,
    protocol_version: Option<String>,
    ln_networks: Option<String>,
    bond_enabled: Option<i64>,
}

impl InfoRow {
    fn into_info(self) -> InstanceInfo {
        InstanceInfo {
            event_id: self.event_id,
            pubkey: self.pubkey,
            created_at: self.created_at,
            fee: self.fee,
            max_order_amount: self.max_order_amount,
            min_order_amount: self.min_order_amount,
            fiat_currencies: self.fiat_currencies,
            mostro_version: self.mostro_version,
            protocol_version: self.protocol_version,
            ln_networks: self.ln_networks,
            bond_enabled: self.bond_enabled.map(|stored| stored != 0),
        }
    }
}

/// Empties the table; see [`super::orders::clear_versions`].
pub async fn clear<'e, E>(executor: E) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query("DELETE FROM instance_info")
        .execute(executor)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
