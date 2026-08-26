//! The `y` tag: which platform published an event, and under what name.
//!
//! [NIP-69](https://nips.nostr.com/69) defines `y` as the name of the platform
//! publishing the order. Mostro extends it with a second value naming the
//! *node*, because Mostro is not one node but a network of many running the
//! same software (`docs/SPEC.md` §2.1):
//!
//! ```text
//! y = ["mostro"]                    an instance that publishes no name
//! y = ["mostro", "Mostro Brasil"]   the usual shape
//! y = ["hodlhodl"]                  not a Mostro event at all
//! ```
//!
//! Both halves are read here, in one place, because both are answers to
//! questions the rest of the pipeline asks of every kind that carries the tag:
//! *should this event be indexed at all* (§8.1 step 4) and *what is this
//! instance called* (§3).

use nostr_sdk::prelude::Event;

use super::tag_values;

/// The platform value bestiario indexes.
pub const MOSTRO: &str = "mostro";

/// The values of the single `y` tag, or `None` when the event carries no `y`
/// or carries more than one.
///
/// A repeated `y` is an error to the tag readers of the parent module, and it
/// is no more readable here: two `y` tags name two platforms, and picking
/// either would be a guess. Reported as absence, the event simply falls out of
/// the Mostro filter of `docs/SPEC.md` §8.1 instead of being indexed under a
/// platform it may not belong to.
fn y(event: &Event) -> Option<Vec<String>> {
    tag_values(event, "y").ok().flatten()
}

/// The first value of `y` — the platform that published the event.
///
/// `None` for the kinds that carry no `y` at all (30078 rates and 10002 relay
/// lists), which is why the platform filter of `docs/SPEC.md` §8.1 is scoped
/// to the kinds that do, and `None` too when the event repeats the tag.
pub fn platform(event: &Event) -> Option<String> {
    y(event)?.first().cloned()
}

/// Whether the event was published by a Mostro node.
pub fn is_mostro(event: &Event) -> bool {
    platform(event).as_deref() == Some(MOSTRO)
}

/// The second value of `y` — the name of the publishing node.
///
/// `None` when the instance publishes no name, which is the normal case for a
/// third of the network (`docs/SPEC.md` §3) and not an anomaly worth an error.
/// An empty or whitespace-only name is treated as no name: it would otherwise
/// be stored, win the most-recent-name-wins rule, and blank out a name the
/// instance had published a minute earlier.
pub fn instance_name(event: &Event) -> Option<String> {
    let name = y(event)?.get(1)?.trim().to_string();

    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests;
