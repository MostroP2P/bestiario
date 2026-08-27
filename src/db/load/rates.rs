//! Rate snapshots as the lookup of `stats::rates` sees them.

use sqlx::{Executor, Sqlite};

use crate::db::repo;
use crate::stats::rates::{RateBook, Snapshot};

/// Every snapshot stored, from every instance: the fallback of
/// [`RateBook::rate_at`] needs the others' snapshots, so the book is never
/// scoped to one instance.
pub async fn book<'e, E>(executor: E) -> Result<RateBook, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let snapshots = repo::rates::all(executor)
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
        let book = book(&pool).await.expect("book");
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
