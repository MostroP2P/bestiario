//! Instances as the bestiary sees them (`docs/SPEC.md` §6.5): the
//! `instances` row plus what its kind 38385 versions have said about it.
//!
//! Each optional field is taken from the *latest version that published
//! it*, not from the latest version wholesale. An instance republishes its
//! info with whatever tags it has at hand, and a newer event that omits
//! `fee` is saying nothing about the fee — the same rule `fee_in_force`
//! applies in `repo::instance_info`. Selecting the newest row as a whole
//! would let a sparse republication erase a fee the instance still charges.

use sqlx::{Executor, QueryBuilder, Sqlite};

use crate::db::repo::csv;
use crate::stats::instances::Profile;

use super::{Scope, instance_label};

/// Every instance in `scope`, oldest first.
///
/// Only the instance half of the scope applies: an instance is not on a
/// network, its orders are.
pub async fn profiles<'e, E>(executor: E, scope: &Scope) -> Result<Vec<Profile>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        "SELECT i.pubkey, i.name, i.first_seen_at, i.last_seen_at,
                {mostro_version}, {protocol_version}, {fee},
                {min_order_amount}, {max_order_amount},
                {fiat_currencies}, {ln_networks}, {bond_enabled}
         FROM instances i
         WHERE 1 = 1",
        mostro_version = latest_published("mostro_version"),
        protocol_version = latest_published("protocol_version"),
        fee = latest_published("fee"),
        min_order_amount = latest_published("min_order_amount"),
        max_order_amount = latest_published("max_order_amount"),
        fiat_currencies = latest_published("fiat_currencies"),
        ln_networks = latest_published("ln_networks"),
        bond_enabled = latest_published("bond_enabled"),
    ));
    scope.apply_instance(&mut query, "i");
    query.push(" ORDER BY i.first_seen_at, i.pubkey");

    Ok(query
        .build_query_as::<Row>()
        .fetch_all(executor)
        .await?
        .into_iter()
        .map(Row::into_profile)
        .collect())
}

/// The subquery for one optional field: its value in the latest 38385 of
/// the instance that carried it, aliased to the column's own name.
///
/// `column` is one of this module's own literals, never input: it is
/// spliced into SQL.
fn latest_published(column: &str) -> String {
    format!(
        "(SELECT v.{column} FROM instance_info v
           WHERE v.pubkey = i.pubkey AND v.{column} IS NOT NULL
           ORDER BY v.created_at DESC, v.event_id ASC LIMIT 1) AS {column}"
    )
}

#[derive(sqlx::FromRow)]
struct Row {
    pubkey: String,
    name: Option<String>,
    first_seen_at: i64,
    last_seen_at: i64,
    mostro_version: Option<String>,
    protocol_version: Option<String>,
    fee: Option<f64>,
    min_order_amount: Option<i64>,
    max_order_amount: Option<i64>,
    fiat_currencies: Option<String>,
    ln_networks: Option<String>,
    bond_enabled: Option<i64>,
}

impl Row {
    fn into_profile(self) -> Profile {
        let split =
            |csv_text: Option<String>| csv_text.map(|text| csv::split(&text)).unwrap_or_default();

        Profile {
            label: instance_label(&self.pubkey, self.name.as_deref()),
            pubkey: self.pubkey,
            name: self.name,
            mostro_version: self.mostro_version,
            protocol_version: self.protocol_version,
            fee: self.fee,
            min_order_sats: self.min_order_amount,
            max_order_sats: self.max_order_amount,
            fiat_currencies: split(self.fiat_currencies),
            ln_networks: split(self.ln_networks),
            bond_enabled: self.bond_enabled.map(|stored| stored != 0),
            first_seen_at: self.first_seen_at,
            last_seen_at: self.last_seen_at,
        }
    }
}

#[cfg(test)]
mod tests;
