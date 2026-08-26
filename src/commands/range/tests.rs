//! Window and instance-filter resolution.

use super::*;

/// 2026-01-01T00:00:00Z, used as "now" so that defaulting is deterministic.
const NOW: i64 = 1_767_225_600;
const DAY: i64 = 86_400;

fn instances() -> Vec<KnownInstance> {
    vec![
        (
            "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390".to_string(),
            Some("lnp2pbot".to_string()),
        ),
        (
            "82fa0000000000000000000000000000000000000000000000000000000000ff".to_string(),
            Some("mostro-dev".to_string()),
        ),
        (
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
            None,
        ),
    ]
}

#[test]
fn both_bounds_given_are_used_as_given() {
    let range = Range::resolve(Some(NOW - DAY), Some(NOW), NOW).expect("valid range");

    assert_eq!(range.from(), NOW - DAY);
    assert_eq!(range.until(), NOW);
}

#[test]
fn a_missing_upper_bound_means_now() {
    // Not "the last event stored": a report covering a quiet week has to say
    // zero, rather than silently shrinking to the last thing that happened.
    let range = Range::resolve(Some(NOW - DAY), None, NOW).expect("valid range");

    assert_eq!(range.until(), NOW);
}

#[test]
fn a_missing_lower_bound_means_thirty_days_before_the_upper_one() {
    let range = Range::resolve(None, None, NOW).expect("valid range");

    assert_eq!(range.from(), NOW - 30 * DAY);
    assert_eq!(range.until(), NOW);
}

#[test]
fn the_default_window_is_measured_from_the_upper_bound_not_from_now() {
    // `--until 2025-01-01` with no `--from` should give December 2024, not the
    // last thirty days ending today, which would be an empty window.
    let until = NOW - 365 * DAY;
    let range = Range::resolve(None, Some(until), NOW).expect("valid range");

    assert_eq!(range.from(), until - 30 * DAY);
    assert_eq!(range.until(), until);
}

#[test]
fn the_window_is_half_open() {
    let range = Range::resolve(Some(100), Some(200), NOW).expect("valid range");

    assert!(range.contains(100), "the lower bound is included");
    assert!(range.contains(199));
    assert!(!range.contains(200), "the upper bound is excluded");
    assert!(!range.contains(99));
}

#[test]
fn consecutive_windows_tile_without_double_counting() {
    // The reason for half-open bounds: a monthly series must count every event
    // exactly once. With inclusive bounds, an event at a boundary lands in two
    // buckets.
    let first = Range::resolve(Some(0), Some(100), NOW).expect("valid range");
    let second = Range::resolve(Some(100), Some(200), NOW).expect("valid range");

    let boundary = 100;
    assert!(!first.contains(boundary));
    assert!(second.contains(boundary));
}

#[test]
fn an_empty_window_is_rejected() {
    let error = Range::resolve(Some(200), Some(100), NOW).expect_err("reversed bounds");

    assert_eq!(
        error,
        RangeError::Empty {
            from: 200,
            until: 100
        }
    );
}

#[test]
fn a_zero_length_window_is_rejected() {
    // `--from X --until X` contains nothing, and would report zeros that look
    // like an answer.
    let error = Range::resolve(Some(100), Some(100), NOW).expect_err("zero length");

    assert!(matches!(error, RangeError::Empty { .. }));
}

#[test]
fn the_previous_window_has_the_same_length_and_ends_where_this_one_starts() {
    // What the "Δ vs. the previous period" metrics of SPEC §6.1 compare with.
    let range = Range::resolve(Some(NOW - 30 * DAY), Some(NOW), NOW).expect("valid range");
    let previous = range.previous();

    assert_eq!(previous.until(), range.from());
    assert_eq!(
        previous.until() - previous.from(),
        range.until() - range.from()
    );
}

#[test]
fn the_unbounded_window_contains_every_plausible_timestamp() {
    let range = Range::unbounded();

    assert!(range.contains(0));
    assert!(range.contains(NOW));
    assert!(range.contains(i64::MAX - 1));
}

#[test]
fn the_window_renders_as_rfc3339_for_the_json_envelope() {
    let range = Range::resolve(Some(1_735_689_600), Some(1_767_225_600), NOW).expect("valid");
    let (from, until) = range.to_rfc3339();

    assert_eq!(from, "2025-01-01T00:00:00+00:00");
    assert_eq!(until, "2026-01-01T00:00:00+00:00");
}

#[test]
fn no_instance_flag_means_every_instance() {
    let filter = InstanceFilter::resolve(None, &instances()).expect("resolves");

    assert_eq!(filter, InstanceFilter::All);
}

#[test]
fn a_full_pubkey_resolves_to_itself() {
    let pubkey = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
    let filter = InstanceFilter::resolve(Some(pubkey), &instances()).expect("resolves");

    assert_eq!(
        filter,
        InstanceFilter::One {
            pubkey: pubkey.to_string()
        }
    );
}

#[test]
fn a_name_resolves_to_its_pubkey() {
    let filter = InstanceFilter::resolve(Some("lnp2pbot"), &instances()).expect("resolves");

    assert_eq!(
        filter,
        InstanceFilter::One {
            pubkey: "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390".to_string()
        }
    );
}

#[test]
fn a_name_is_matched_case_insensitively() {
    // Names come from a free-text tag; nobody types them back exactly.
    let filter = InstanceFilter::resolve(Some("LNP2PBot"), &instances()).expect("resolves");

    assert!(matches!(filter, InstanceFilter::One { .. }));
}

#[test]
fn a_unique_pubkey_prefix_resolves() {
    let filter = InstanceFilter::resolve(Some("0123456"), &instances()).expect("resolves");

    assert_eq!(
        filter,
        InstanceFilter::One {
            pubkey: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string()
        }
    );
}

#[test]
fn an_ambiguous_prefix_is_an_error_naming_the_candidates() {
    // Reporting on the wrong instance is worse than being asked for another
    // character, so this must not silently pick one.
    let error = InstanceFilter::resolve(Some("82fa"), &instances()).expect_err("ambiguous");

    match error {
        InstanceError::Ambiguous { needle, pubkeys } => {
            assert_eq!(needle, "82fa");
            assert_eq!(pubkeys.len(), 2);
        }
        other => panic!("expected an ambiguity error, got {other:?}"),
    }
}

#[test]
fn an_exact_pubkey_wins_over_being_a_prefix_of_another() {
    // A pubkey that happens to be a prefix of nothing else is trivial; this
    // covers the case where an exact match co-exists with prefix matches.
    let known = vec![("abcd".to_string(), None), ("abcdef".to_string(), None)];

    let filter = InstanceFilter::resolve(Some("abcd"), &known).expect("resolves");

    assert_eq!(
        filter,
        InstanceFilter::One {
            pubkey: "abcd".to_string()
        }
    );
}

#[test]
fn an_unknown_instance_is_an_error_rather_than_an_empty_report() {
    let error = InstanceFilter::resolve(Some("nosuch"), &instances()).expect_err("unknown");

    assert_eq!(
        error,
        InstanceError::NotFound {
            needle: "nosuch".to_string()
        }
    );
}

#[test]
fn surrounding_whitespace_is_ignored() {
    let filter = InstanceFilter::resolve(Some("  lnp2pbot  "), &instances()).expect("resolves");

    assert!(matches!(filter, InstanceFilter::One { .. }));
}
