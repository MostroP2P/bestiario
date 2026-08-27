//! Dev fees and settlements as `stats::dev_fees` sees them (`docs/SPEC.md`
//! §6.6).
//!
//! Two reads. The fees carry what the `orders` projection knows about the
//! order each names — whether it exists, and when it completed. The
//! settlements are the completed orders themselves, each with whether any
//! fee names it and what the instance's fee in force was at the time, which
//! is the `fee_in_force` lookup of `repo::instance_info` folded into the
//! query so a network's worth of orders is one round trip, not one per row.

use sqlx::{Executor, QueryBuilder, Sqlite};

use crate::stats::dev_fees::{DevFeeData, Fee, Settlement};

use super::{Scope, instance_label};

/// Everything in `scope` the §6.6 figures need.
///
/// Not windowed, for the same reason as [`super::activity::orders`]: the
/// aggregation dates fees and settlements by different events, and the
/// monthly slices need whatever falls in each month.
pub async fn load<'e, E>(executor: E, scope: &Scope) -> Result<DevFeeData, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite> + Copy,
{
    Ok(DevFeeData {
        fees: fees(executor, scope).await?,
        settlements: settlements(executor, scope).await?,
    })
}

/// Every fee in `scope`, oldest first, with what is known of its order.
async fn fees<'e, E>(executor: E, scope: &Scope) -> Result<Vec<Fee>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT f.event_id, f.order_id, f.pubkey, i.name AS instance_name, f.created_at,
                f.amount_sats, f.is_duplicate,
                o.order_id IS NOT NULL AS order_known,
                CASE WHEN o.final_status = 'success' THEN o.success_at END AS settled_at
         FROM dev_fees f
         LEFT JOIN instances i ON i.pubkey = f.pubkey
         LEFT JOIN orders o ON o.order_id = f.order_id
         WHERE 1 = 1",
    );
    scope.apply(&mut query, "f");
    query.push(" ORDER BY f.created_at, f.event_id");

    Ok(query
        .build_query_as::<FeeRow>()
        .fetch_all(executor)
        .await?
        .into_iter()
        .map(FeeRow::into_fee)
        .collect())
}

/// Every completed order in `scope`, oldest settlement first.
async fn settlements<'e, E>(executor: E, scope: &Scope) -> Result<Vec<Settlement>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT o.order_id, o.pubkey, i.name AS instance_name, o.success_at,
                EXISTS (SELECT 1 FROM dev_fees f WHERE f.order_id = o.order_id) AS has_fee,
                (SELECT ii.fee FROM instance_info ii
                  WHERE ii.pubkey = o.pubkey AND ii.created_at <= o.success_at
                    AND ii.fee IS NOT NULL
                  ORDER BY ii.created_at DESC, ii.event_id ASC LIMIT 1) AS fee_in_force
         FROM orders o
         LEFT JOIN instances i ON i.pubkey = o.pubkey
         WHERE o.final_status = 'success' AND o.success_at IS NOT NULL",
    );
    scope.apply(&mut query, "o");
    query.push(" ORDER BY o.success_at, o.order_id");

    Ok(query
        .build_query_as::<SettlementRow>()
        .fetch_all(executor)
        .await?
        .into_iter()
        .map(SettlementRow::into_settlement)
        .collect())
}

#[derive(sqlx::FromRow)]
struct FeeRow {
    event_id: String,
    order_id: String,
    pubkey: String,
    instance_name: Option<String>,
    created_at: i64,
    amount_sats: i64,
    is_duplicate: i64,
    order_known: i64,
    settled_at: Option<i64>,
}

impl FeeRow {
    fn into_fee(self) -> Fee {
        Fee {
            event_id: self.event_id,
            order_id: self.order_id,
            instance: instance_label(&self.pubkey, self.instance_name.as_deref()),
            created_at: self.created_at,
            amount_sats: self.amount_sats,
            is_duplicate: self.is_duplicate != 0,
            order_known: self.order_known != 0,
            settled_at: self.settled_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct SettlementRow {
    order_id: String,
    pubkey: String,
    instance_name: Option<String>,
    success_at: i64,
    has_fee: i64,
    fee_in_force: Option<f64>,
}

impl SettlementRow {
    fn into_settlement(self) -> Settlement {
        Settlement {
            order_id: self.order_id,
            instance: instance_label(&self.pubkey, self.instance_name.as_deref()),
            success_at: self.success_at,
            has_fee: self.has_fee != 0,
            charges_fee: self.fee_in_force.map(|fee| fee > 0.0),
        }
    }
}

#[cfg(test)]
mod tests;
