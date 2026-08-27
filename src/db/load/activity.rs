//! Orders as `stats::activity` sees them (`docs/SPEC.md` §6.1).
//!
//! One row per order, read from the projection plus the two facts the
//! projection does not keep: when a taker arrived, which is the first
//! `in-progress` version, and the expiry of the latest version, which
//! decides whether a `pending` order is still open.

use sqlx::{Executor, QueryBuilder, Sqlite};

use crate::db::repo::{csv, decode};
use crate::ingest::parse::order::{Direction, Status};
use crate::stats::activity::{self, Order};

use super::{Scope, instance_label};

/// Every order in `scope`, oldest first.
///
/// Not windowed: the activity figures need the previous period for the
/// deltas and every live order for the "now" counts, so the aggregation
/// decides what falls where.
pub async fn orders<'e, E>(executor: E, scope: &Scope) -> Result<Vec<Order>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT o.order_id, o.pubkey, i.name AS instance_name,
                o.first_seen_at AS created_at, o.final_status AS status, o.kind,
                o.fiat_code, o.payment_methods, o.success_at, o.canceled_at,
                (SELECT MIN(v.created_at) FROM order_versions v
                  WHERE v.order_id = o.order_id AND v.status = 'in-progress') AS taken_at,
                (SELECT v.expires_at FROM order_versions v
                  WHERE v.order_id = o.order_id
                  ORDER BY v.created_at DESC, v.event_id ASC LIMIT 1) AS expires_at
         FROM orders o
         LEFT JOIN instances i ON i.pubkey = o.pubkey
         WHERE 1 = 1",
    );

    if let Some(pubkey) = &scope.pubkey {
        query.push(" AND o.pubkey = ").push_bind(pubkey);
    }
    if !scope.networks.is_empty() {
        query.push(" AND o.network IN (");
        let mut networks = query.separated(", ");
        for network in &scope.networks {
            networks.push_bind(network.as_str());
        }
        query.push(")");
    }
    query.push(" ORDER BY o.first_seen_at, o.order_id");

    query
        .build_query_as::<Row>()
        .fetch_all(executor)
        .await?
        .into_iter()
        .map(Row::into_order)
        .collect()
}

#[derive(sqlx::FromRow)]
struct Row {
    order_id: String,
    pubkey: String,
    instance_name: Option<String>,
    created_at: i64,
    status: String,
    kind: String,
    fiat_code: String,
    payment_methods: String,
    success_at: Option<i64>,
    canceled_at: Option<i64>,
    taken_at: Option<i64>,
    expires_at: Option<i64>,
}

impl Row {
    fn into_order(self) -> Result<Order, sqlx::Error> {
        let instance = instance_label(&self.pubkey, self.instance_name.as_deref());

        Ok(Order {
            order_id: self.order_id,
            pubkey: self.pubkey,
            instance,
            created_at: self.created_at,
            status: status(decode("final_status", Status::parse(&self.status))?),
            direction: direction(decode("kind", Direction::parse(&self.kind))?),
            fiat_code: self.fiat_code,
            payment_methods: csv::split(&self.payment_methods),
            taken_at: self.taken_at,
            success_at: self.success_at,
            canceled_at: self.canceled_at,
            expires_at: self.expires_at,
        })
    }
}

/// The parser's status as the stats crate's. The two enums are the same
/// four values on purpose; this `match` is where the compiler checks it.
fn status(status: Status) -> activity::Status {
    match status {
        Status::Pending => activity::Status::Pending,
        Status::InProgress => activity::Status::InProgress,
        Status::Success => activity::Status::Success,
        Status::Canceled => activity::Status::Canceled,
    }
}

fn direction(direction: Direction) -> activity::Direction {
    match direction {
        Direction::Buy => activity::Direction::Buy,
        Direction::Sell => activity::Direction::Sell,
    }
}

#[cfg(test)]
mod tests;
