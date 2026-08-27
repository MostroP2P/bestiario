//! Disputes and taken orders as `stats::disputes` sees them (`docs/SPEC.md`
//! §6.7).
//!
//! The dispute read adds two facts to the projection: an opening time for
//! a dispute whose versions never carried the `created_at` tag (the first
//! version seen), and the first terminal version, which is when it was
//! resolved. The second read is the population disputes arise from — orders
//! that found a taker — dated by the first `in-progress` version, or by the
//! settlement when the walk never saw one.
//!
//! The network filter applies to neither read. Kind 38386 publishes no
//! `network` tag (§2.3), so disputes cannot be attributed to a network;
//! filtering only the orders would divide the disputes of every network by
//! the orders of one. Both are therefore network-blind, and the command
//! refuses `--network` rather than print a report that pretends otherwise.

use sqlx::{Executor, QueryBuilder, Sqlite};

use crate::db::repo::decode;
use crate::ingest::parse::dispute::{Initiator, Status};
use crate::stats::disputes::{self, Dispute, DisputeData, Taken};

use super::{Scope, instance_label};

/// Everything in `scope` the §6.7 figures need.
pub async fn load<'e, E>(executor: E, scope: &Scope) -> Result<DisputeData, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite> + Copy,
{
    Ok(DisputeData {
        disputes: disputes(executor, scope).await?,
        taken: taken(executor, scope).await?,
    })
}

async fn disputes<'e, E>(executor: E, scope: &Scope) -> Result<Vec<Dispute>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT d.dispute_id, d.pubkey, i.name AS instance_name,
                COALESCE(d.opened_at, (SELECT MIN(v.created_at) FROM dispute_versions v
                                        WHERE v.dispute_id = d.dispute_id)) AS opened_at,
                d.final_status AS status, d.initiator,
                (SELECT MIN(v.created_at) FROM dispute_versions v
                  WHERE v.dispute_id = d.dispute_id
                    AND v.status IN ('seller-refunded', 'settled', 'released')) AS resolved_at,
                (SELECT v.status FROM dispute_versions v
                  WHERE v.dispute_id = d.dispute_id
                    AND v.status IN ('seller-refunded', 'settled', 'released')
                  ORDER BY v.created_at, v.event_id LIMIT 1) AS outcome
         FROM disputes d
         LEFT JOIN instances i ON i.pubkey = d.pubkey
         WHERE 1 = 1",
    );
    scope.apply_instance(&mut query, "d");
    query.push(" ORDER BY opened_at, d.dispute_id");

    query
        .build_query_as::<DisputeRow>()
        .fetch_all(executor)
        .await?
        .into_iter()
        .map(DisputeRow::into_dispute)
        .collect()
}

async fn taken<'e, E>(executor: E, scope: &Scope) -> Result<Vec<Taken>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT o.order_id, o.pubkey, i.name AS instance_name,
                COALESCE((SELECT MIN(v.created_at) FROM order_versions v
                           WHERE v.order_id = o.order_id AND v.status = 'in-progress'),
                         o.success_at) AS left_pending_at
         FROM orders o
         LEFT JOIN instances i ON i.pubkey = o.pubkey
         WHERE 1 = 1",
    );
    scope.apply_instance(&mut query, "o");
    query.push(" ORDER BY left_pending_at, o.order_id");

    Ok(query
        .build_query_as::<TakenRow>()
        .fetch_all(executor)
        .await?
        .into_iter()
        .filter_map(TakenRow::into_taken)
        .collect())
}

#[derive(sqlx::FromRow)]
struct DisputeRow {
    dispute_id: String,
    pubkey: String,
    instance_name: Option<String>,
    opened_at: i64,
    status: String,
    initiator: Option<String>,
    resolved_at: Option<i64>,
    outcome: Option<String>,
}

impl DisputeRow {
    fn into_dispute(self) -> Result<Dispute, sqlx::Error> {
        let initiator = match self.initiator.as_deref() {
            None => None,
            Some(wire) => Some(initiator(decode("initiator", Initiator::parse(wire))?)),
        };

        let outcome = match self.outcome.as_deref() {
            None => None,
            Some(wire) => Some(status(decode("outcome", Status::parse(wire))?)),
        };

        Ok(Dispute {
            dispute_id: self.dispute_id,
            instance: instance_label(&self.pubkey, self.instance_name.as_deref()),
            opened_at: self.opened_at,
            status: status(decode("final_status", Status::parse(&self.status))?),
            initiator,
            resolved_at: self.resolved_at,
            outcome,
        })
    }
}

#[derive(sqlx::FromRow)]
struct TakenRow {
    order_id: String,
    pubkey: String,
    instance_name: Option<String>,
    left_pending_at: Option<i64>,
}

impl TakenRow {
    /// `None` for an order that never left `pending`.
    fn into_taken(self) -> Option<Taken> {
        Some(Taken {
            order_id: self.order_id,
            instance: instance_label(&self.pubkey, self.instance_name.as_deref()),
            left_pending_at: self.left_pending_at?,
        })
    }
}

/// The parser's status as the stats crate's; the `match` is the check that
/// the two vocabularies agree.
fn status(status: Status) -> disputes::Status {
    match status {
        Status::Initiated => disputes::Status::Initiated,
        Status::InProgress => disputes::Status::InProgress,
        Status::SellerRefunded => disputes::Status::SellerRefunded,
        Status::Settled => disputes::Status::Settled,
        Status::Released => disputes::Status::Released,
    }
}

fn initiator(initiator: Initiator) -> disputes::Initiator {
    match initiator {
        Initiator::Buyer => disputes::Initiator::Buyer,
        Initiator::Seller => disputes::Initiator::Seller,
    }
}

#[cfg(test)]
mod tests;
