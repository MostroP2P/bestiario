//! Instances as the bestiary sees them (`docs/SPEC.md` §6.5): the
//! `instances` row joined to the latest kind 38385 the instance published.

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
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT i.pubkey, i.name, i.first_seen_at, i.last_seen_at,
                info.mostro_version, info.protocol_version, info.fee,
                info.min_order_amount, info.max_order_amount,
                info.fiat_currencies, info.ln_networks, info.bond_enabled
         FROM instances i
         LEFT JOIN instance_info info ON info.event_id = (
             SELECT latest.event_id FROM instance_info latest
             WHERE latest.pubkey = i.pubkey
             ORDER BY latest.created_at DESC, latest.event_id ASC LIMIT 1
         )
         WHERE 1 = 1",
    );
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
