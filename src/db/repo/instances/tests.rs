//! The instance projection and its name history, against a real migrated
//! SQLite database.

use sqlx::SqlitePool;

use super::*;
use crate::db::connect_and_migrate;

const MEMORY: &str = "sqlite::memory:";
const PUBKEY: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const OTHER: &str = "00037abd1b8a1a6c1c3f4b9e2d5a7c8e0f1234567890abcdef1234567890abcd";
/// Two publications, an hour apart, in the instance's own clock.
const EARLIER: i64 = 1_787_700_000;
const LATER: i64 = 1_787_703_600;

async fn migrated() -> SqlitePool {
    connect_and_migrate(MEMORY).await.expect("migrate")
}

/// What the pipeline does for one event: refresh the projection, and add the
/// name to the history when there is one.
async fn seen(pool: &SqlitePool, pubkey: &str, name: Option<&str>, at: i64) {
    upsert(pool, pubkey, name, at).await.expect("upsert");
    if let Some(name) = name {
        record_name(pool, pubkey, name, at).await.expect("history");
    }
}

#[tokio::test]
async fn a_first_sighting_creates_the_instance() {
    // Arrange
    let pool = migrated().await;

    // Act
    seen(&pool, PUBKEY, Some("Mostro"), EARLIER).await;

    // Assert
    let instance = find(&pool, PUBKEY).await.expect("find").expect("stored");
    assert_eq!(instance.name.as_deref(), Some("Mostro"));
    assert_eq!(instance.name_seen_at, Some(EARLIER));
    assert_eq!(instance.first_seen_at, EARLIER);
    assert_eq!(instance.last_seen_at, EARLIER);
}

#[tokio::test]
async fn a_nameless_instance_is_stored_with_no_name() {
    // A third of the network publishes `y = ["mostro"]` and nothing else. It
    // is still an instance, and still has to appear in the bestiary.
    let pool = migrated().await;

    seen(&pool, PUBKEY, None, EARLIER).await;

    let instance = find(&pool, PUBKEY).await.expect("find").expect("stored");
    assert_eq!(instance.name, None);
    assert_eq!(instance.name_seen_at, None);
    assert!(names(&pool, PUBKEY).await.expect("names").is_empty());
}

#[tokio::test]
async fn a_rename_wins_and_both_names_stay_in_the_history() {
    let pool = migrated().await;

    seen(&pool, PUBKEY, Some("Mostro"), EARLIER).await;
    seen(&pool, PUBKEY, Some("Mostro Brasil"), LATER).await;

    let instance = find(&pool, PUBKEY).await.expect("find").expect("stored");
    assert_eq!(instance.name.as_deref(), Some("Mostro Brasil"));
    assert_eq!(instance.name_seen_at, Some(LATER));
    assert_eq!(
        names(&pool, PUBKEY).await.expect("names"),
        vec![
            ("Mostro Brasil".to_string(), LATER),
            ("Mostro".to_string(), EARLIER),
        ]
    );
}

#[tokio::test]
async fn an_older_event_does_not_overwrite_a_newer_name() {
    // Backfill walks backwards, so the older event arrives second. Ordering
    // by arrival would let it overwrite the current name with a stale one.
    let pool = migrated().await;

    seen(&pool, PUBKEY, Some("Mostro Brasil"), LATER).await;
    seen(&pool, PUBKEY, Some("Mostro"), EARLIER).await;

    let instance = find(&pool, PUBKEY).await.expect("find").expect("stored");
    assert_eq!(instance.name.as_deref(), Some("Mostro Brasil"));
    assert_eq!(instance.name_seen_at, Some(LATER));
}

#[tokio::test]
async fn a_nameless_event_never_clears_a_known_name() {
    // The same instance names itself on its orders and not on its disputes
    // (SPEC 3). If the dispute cleared the name, the bestiary would flicker
    // between named and anonymous depending on which kind arrived last.
    let pool = migrated().await;

    seen(&pool, PUBKEY, Some("Mostro"), EARLIER).await;
    seen(&pool, PUBKEY, None, LATER).await;

    let instance = find(&pool, PUBKEY).await.expect("find").expect("stored");
    assert_eq!(instance.name.as_deref(), Some("Mostro"));
    assert_eq!(instance.name_seen_at, Some(EARLIER));
    assert_eq!(instance.last_seen_at, LATER, "the sighting still counts");
}

#[tokio::test]
async fn a_name_arriving_after_a_nameless_sighting_is_taken() {
    let pool = migrated().await;

    seen(&pool, PUBKEY, None, EARLIER).await;
    seen(&pool, PUBKEY, Some("Mostro"), LATER).await;

    let instance = find(&pool, PUBKEY).await.expect("find").expect("stored");
    assert_eq!(instance.name.as_deref(), Some("Mostro"));
}

#[tokio::test]
async fn the_window_widens_in_both_directions() {
    // first_seen_at and last_seen_at bound every sighting, whichever order
    // the events arrive in.
    let pool = migrated().await;

    seen(&pool, PUBKEY, None, LATER).await;
    seen(&pool, PUBKEY, None, EARLIER).await;

    let instance = find(&pool, PUBKEY).await.expect("find").expect("stored");
    assert_eq!(instance.first_seen_at, EARLIER);
    assert_eq!(instance.last_seen_at, LATER);
}

#[tokio::test]
async fn replaying_the_same_event_changes_nothing() {
    let pool = migrated().await;

    seen(&pool, PUBKEY, Some("Mostro"), EARLIER).await;
    let once = find(&pool, PUBKEY).await.expect("find").expect("stored");
    seen(&pool, PUBKEY, Some("Mostro"), EARLIER).await;
    let twice = find(&pool, PUBKEY).await.expect("find").expect("stored");

    assert_eq!(once, twice);
    assert_eq!(names(&pool, PUBKEY).await.expect("names").len(), 1);
}

#[tokio::test]
async fn instances_do_not_bleed_into_each_other() {
    let pool = migrated().await;

    seen(&pool, PUBKEY, Some("Mostro"), EARLIER).await;
    seen(&pool, OTHER, Some("Mostro Brasil"), LATER).await;

    let all = all(&pool).await.expect("all");
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].pubkey, PUBKEY, "oldest first");
    assert_eq!(all[1].name.as_deref(), Some("Mostro Brasil"));
    assert_eq!(names(&pool, OTHER).await.expect("names").len(), 1);
}

#[tokio::test]
async fn an_unknown_pubkey_is_not_found() {
    let pool = migrated().await;

    assert_eq!(find(&pool, PUBKEY).await.expect("find"), None);
    assert!(all(&pool).await.expect("all").is_empty());
}
