//! One parser per Nostr kind: tags in, typed struct out.
//!
//! Responsibility: the tag-level knowledge of `docs/SPEC.md` §2. A parser
//! never touches the database and never decides whether an event is wanted —
//! it only decides whether an event is *well formed*. Unknown or missing
//! required tags are hard errors, never silent defaults.

pub mod dev_fee;
pub mod dispute;
pub mod info;
pub mod order;
pub mod rates;
pub mod relay_list;
