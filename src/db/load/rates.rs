//! Rate snapshots as the lookup of `stats::rates` sees them.

use sqlx::{Executor, Sqlite};

use std::collections::{BTreeMap, BTreeSet};

use crate::db::repo;
use crate::stats::rates::feeds::Feed;
use crate::stats::rates::{MAX_AGE_SECS, RateBook, Snapshot};

use super::instance_label;

/// The snapshots that can price the orders completed in `[from, until)`,
/// from every instance: the fallback of [`RateBook::rate_at`] needs the
/// others' snapshots, so the book is never scoped to one instance — only
/// to the time it can be asked about.
///
/// A quote qualifies for up to [`MAX_AGE_SECS`] after it is published, so
/// the floor is that much below the window's; nothing published from
/// `until` onwards can price an order that completed before it.
pub async fn book<'e, E>(executor: E, from: i64, until: i64) -> Result<RateBook, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let snapshots = repo::rates::published_between(executor, from - MAX_AGE_SECS, until)
        .await?
        .into_iter()
        .map(|snapshot| Snapshot {
            pubkey: snapshot.pubkey,
            published_at: snapshot.published_at,
            rates: snapshot.rates,
        })
        .collect();

    Ok(RateBook::new(snapshots))
}

/// One [`Feed`] per known instance: its latest snapshot, or none at all.
///
/// Every instance in the bestiary is listed, including those that have
/// published no rate — §6.8 asks which feeds are dead, and one that never
/// started is the strongest case of it. `pubkey` narrows to one instance.
///
/// # Only the bestiary quotes
///
/// A rate row whose publisher has no `instances` entry is not admitted as
/// a feed. §8.1 step 4b lets a kind 30078 event in only from a pubkey the
/// bestiary already knows, and ingestion upserts that instance in the same
/// transaction as the snapshot, so an orphan row is not a state normal
/// ingestion or replay produces: it is admission bypassed or storage torn.
/// Restoring its trust at report time would let the content of an
/// unvouched event set the price §5 multiplies every converted figure by —
/// "an unvouched feed is not a feed", and the report is the wrong place to
/// decide otherwise. The rows are left out and named, so the omission is
/// visible rather than silent.
///
/// Not scoped by network: a kind 30078 event carries no network tag.
pub async fn feeds<'e, E>(executor: E, pubkey: Option<&str>) -> Result<Vec<Feed>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite> + Copy,
{
    let latest: BTreeMap<String, (i64, BTreeMap<String, f64>)> =
        repo::rates::latest_per_instance(executor, pubkey)
            .await?
            .into_iter()
            .map(|snapshot| (snapshot.pubkey, (snapshot.published_at, snapshot.rates)))
            .collect();

    let known = repo::instances::all(executor).await?;
    let bestiary: BTreeSet<&str> = known
        .iter()
        .map(|instance| instance.pubkey.as_str())
        .collect();
    let orphans: Vec<&str> = latest
        .keys()
        .map(String::as_str)
        .filter(|quoting| !bestiary.contains(quoting))
        .collect();
    if !orphans.is_empty() {
        return Err(sqlx::Error::Protocol(format!(
            "rate snapshots from {} pubkey(s) with no instance row ({}): a 30078 \
             event is admitted only from a vouched instance (SPEC §8.1 step 4b), \
             so this is a bypassed admission or a torn store — re-ingest \
             rather than report",
            orphans.len(),
            orphans.join(", ")
        )));
    }

    Ok(known
        .iter()
        .filter(|instance| pubkey.is_none_or(|wanted| instance.pubkey == wanted))
        .map(|instance| {
            let snapshot = latest.get(&instance.pubkey);
            Feed {
                instance: instance_label(&instance.pubkey, instance.name.as_deref()),
                published_at: snapshot.map(|(at, _)| *at),
                rates: snapshot.map(|(_, rates)| rates.clone()).unwrap_or_default(),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connect_and_migrate;
    use crate::ingest::pipeline::seed_fixtures;
    use crate::stats::rates::RateSource;

    #[tokio::test]
    async fn the_book_holds_the_captured_snapshots_and_answers_from_them() {
        // Arrange: both 30078 fixtures, published 2026-08-26 around 10:39Z.
        let pool = connect_and_migrate("sqlite::memory:")
            .await
            .expect("migrate");
        seed_fixtures(&pool, 1_787_800_000).await;
        let mostro = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";

        // Act
        let book = book(&pool, 1_787_740_000, 1_787_800_000)
            .await
            .expect("book");
        let own = book.rate_at(mostro, "USD", 1_787_741_000).expect("a rate");
        let other = book
            .rate_at("nobody", "USD", 1_787_741_000)
            .expect("a rate");

        // Assert
        assert!(!book.is_empty());
        assert_eq!(own.source, RateSource::Instance);
        assert_eq!(own.age_secs, 1_787_741_000 - 1_787_740_773);
        assert!(matches!(other.source, RateSource::Fallback { .. }));
        assert!(book.rate_at(mostro, "USD", 1_787_700_000).is_none());
    }
}
