//! The relays bestiario knows about — the configured ones and those an
//! instance's NIP-65 list named (`docs/SPEC.md` §2.6).
//!
//! One row per URL, with the *first* source that named it. First rather
//! than latest on purpose: a relay the operator configured stays configured
//! even after an instance advertises it, and a relay discovered from two
//! instances is credited to the one that named it first. The column answers
//! "how did bestiario come to dial this?", and that has one answer.

use sqlx::{Executor, Sqlite};

/// Where a relay URL came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Listed in `[nostr].relays`.
    Config,
    /// Named by this instance's kind 10002.
    Nip65 { pubkey: String },
}

impl Source {
    /// As stored: `config`, or `nip65:<pubkey>`.
    pub fn as_stored(&self) -> String {
        match self {
            Self::Config => "config".to_string(),
            Self::Nip65 { pubkey } => format!("nip65:{pubkey}"),
        }
    }

    fn parse(stored: &str) -> Self {
        match stored.strip_prefix("nip65:") {
            Some(pubkey) => Self::Nip65 {
                pubkey: pubkey.to_string(),
            },
            None => Self::Config,
        }
    }
}

/// One known relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relay {
    pub url: String,
    pub source: Source,
    pub first_seen_at: i64,
}

/// Records `url`, keeping the source that named it first.
pub async fn upsert<'e, E>(
    executor: E,
    url: &str,
    source: &Source,
    first_seen_at: i64,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO relays (url, source, first_seen_at) VALUES (?, ?, ?)
         ON CONFLICT(url) DO NOTHING",
    )
    .bind(url)
    .bind(source.as_stored())
    .bind(first_seen_at)
    .execute(executor)
    .await?;

    Ok(())
}

/// Every relay known, first seen first.
pub async fn all<'e, E>(executor: E) -> Result<Vec<Relay>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    Ok(sqlx::query_as::<_, Row>(
        "SELECT url, source, first_seen_at FROM relays ORDER BY first_seen_at, url",
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(Row::into_relay)
    .collect())
}

/// The relays an instance's NIP-65 list named, first seen first.
pub async fn discovered<'e, E>(executor: E) -> Result<Vec<Relay>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    Ok(all(executor)
        .await?
        .into_iter()
        .filter(|relay| relay.source != Source::Config)
        .collect())
}

#[derive(sqlx::FromRow)]
struct Row {
    url: String,
    source: String,
    first_seen_at: i64,
}

impl Row {
    fn into_relay(self) -> Relay {
        Relay {
            source: Source::parse(&self.source),
            url: self.url,
            first_seen_at: self.first_seen_at,
        }
    }
}

#[cfg(test)]
mod tests;
