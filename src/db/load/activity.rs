//! Orders as `stats::activity` sees them (`docs/SPEC.md` §6.1).
//!
//! One row per order, read from the projection plus the facts the
//! projection does not keep: when a taker arrived, which is the first
//! `in-progress` version; the expiry of the latest version, which decides
//! whether a `pending` order is still open; and the sats and fiat range of
//! the first version, which say whether the order was priced at market and
//! whether it was a range (§4 `price_type`, `range`).

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
    fetch(executor, scope, Filter::All).await
}

/// The orders in `scope` with any version published at `from..until`, plus
/// the `pending` ones still live at `now` — what a timing report needs:
/// every transition that ends in the window, and the current book.
pub async fn lifecycle_in<'e, E>(
    executor: E,
    scope: &Scope,
    from: i64,
    until: i64,
    now: i64,
) -> Result<Vec<Order>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    fetch(executor, scope, Filter::TouchedIn { from, until, now }).await
}

/// Which orders a read returns.
enum Filter {
    All,
    CompletedIn { from: i64, until: i64 },
    TouchedIn { from: i64, until: i64, now: i64 },
}

/// The orders in `scope` that reached `success` at `from..until`, oldest
/// first — what a volume report needs and no more, so that its cost grows
/// with the window asked for and not with the history kept. Reads the
/// `orders_success_at` index.
pub async fn completed_in<'e, E>(
    executor: E,
    scope: &Scope,
    from: i64,
    until: i64,
) -> Result<Vec<Order>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    fetch(executor, scope, Filter::CompletedIn { from, until }).await
}

async fn fetch<'e, E>(executor: E, scope: &Scope, filter: Filter) -> Result<Vec<Order>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT o.order_id, o.pubkey, i.name AS instance_name,
                o.first_seen_at AS created_at, o.final_status AS status, o.kind,
                o.fiat_code, o.payment_methods, o.amount_sats, o.fiat_amount, o.premium,
                o.success_at, o.canceled_at,
                (SELECT v.amount_sats FROM order_versions v
                  WHERE v.order_id = o.order_id
                  ORDER BY v.created_at ASC, v.event_id ASC LIMIT 1) AS first_amount_sats,
                (SELECT v.fiat_min FROM order_versions v
                  WHERE v.order_id = o.order_id
                  ORDER BY v.created_at ASC, v.event_id ASC LIMIT 1) AS first_fiat_min,
                (SELECT v.fiat_max FROM order_versions v
                  WHERE v.order_id = o.order_id
                  ORDER BY v.created_at ASC, v.event_id ASC LIMIT 1) AS first_fiat_max,
                (SELECT v.fiat_code FROM order_versions v
                  WHERE v.order_id = o.order_id
                  ORDER BY v.created_at ASC, v.event_id ASC LIMIT 1) AS first_fiat_code,
                (SELECT v.payment_methods FROM order_versions v
                  WHERE v.order_id = o.order_id
                  ORDER BY v.created_at ASC, v.event_id ASC LIMIT 1) AS first_payment_methods,
                (SELECT v.kind FROM order_versions v
                  WHERE v.order_id = o.order_id
                  ORDER BY v.created_at ASC, v.event_id ASC LIMIT 1) AS first_kind,
                (SELECT MIN(v.created_at) FROM order_versions v
                  WHERE v.order_id = o.order_id AND v.status = 'pending') AS pending_at,
                (SELECT MIN(v.created_at) FROM order_versions v
                  WHERE v.order_id = o.order_id AND v.status = 'in-progress') AS taken_at,
                (SELECT v.expires_at FROM order_versions v
                  WHERE v.order_id = o.order_id
                  ORDER BY v.created_at DESC, v.event_id ASC LIMIT 1) AS expires_at
         FROM orders o
         LEFT JOIN instances i ON i.pubkey = o.pubkey
         WHERE 1 = 1",
    );

    scope.apply(&mut query, "o");
    match filter {
        Filter::All => {}
        Filter::CompletedIn { from, until } => {
            query
                .push(" AND o.final_status = 'success' AND o.success_at >= ")
                .push_bind(from)
                .push(" AND o.success_at < ")
                .push_bind(until);
        }
        Filter::TouchedIn { from, until, now } => {
            query
                .push(" AND (EXISTS (SELECT 1 FROM order_versions v WHERE v.order_id = o.order_id")
                .push(" AND v.created_at >= ")
                .push_bind(from)
                .push(" AND v.created_at < ")
                .push_bind(until)
                .push(") OR (o.final_status = 'pending' AND (SELECT v.expires_at FROM order_versions v")
                .push(" WHERE v.order_id = o.order_id ORDER BY v.created_at DESC, v.event_id ASC LIMIT 1) > ")
                .push_bind(now)
                .push("))");
        }
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
    amount_sats: i64,
    fiat_amount: Option<f64>,
    premium: f64,
    first_amount_sats: Option<i64>,
    first_fiat_min: Option<f64>,
    first_fiat_max: Option<f64>,
    first_fiat_code: Option<String>,
    first_payment_methods: Option<String>,
    first_kind: Option<String>,
    pending_at: Option<i64>,
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
            amount_sats: self.amount_sats,
            fiat_amount: self.fiat_amount,
            premium: self.premium,
            is_market_price: self.first_amount_sats == Some(0),
            fiat_range: self.first_fiat_min.zip(self.first_fiat_max),
            pending_at: self.pending_at,
            origin: activity::Origin {
                fiat_code: self.first_fiat_code.unwrap_or_default(),
                payment_methods: csv::split(&self.first_payment_methods.unwrap_or_default()),
                direction: match self.first_kind {
                    Some(kind) => direction(decode("kind", Direction::parse(&kind))?),
                    None => activity::Direction::default(),
                },
            },
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
