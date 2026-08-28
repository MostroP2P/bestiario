//! The signing key, and the signed event a document becomes —
//! `docs/NOSTR-PUBLICATION.md` §12 and §11.
//!
//! Responsibility: turn a key an operator configured into [`Keys`], and a
//! [`Document`] the stats crate computed into an event a relay will accept.
//! Nothing here talks to a relay; that is [`super::client`].
//!
//! # Why the key is in the environment
//!
//! §12 keeps the key out of the command line: a flag is readable in `ps`
//! by every user on the machine and lands in the shell history of the one
//! who typed it. It is kept out of `settings.toml` for the same kind of
//! reason — that file is copied between machines, committed, and pasted
//! into issues. What the file holds is the *name* of an environment
//! variable, and this module is where that name becomes a key.
//!
//! # Why the event's clock is the run's
//!
//! `created_at` is the snapshot's `generated_at`, not the moment the
//! signature was produced. Every document of a run then carries the same
//! timestamp, which is what §7 means by a snapshot being one reading of
//! the archive — and it keeps a run that takes a minute to sign from
//! looking like a minute of separate publications.

use nostr_sdk::prelude::*;

use crate::config::SecretRef;

use crate::stats::publish::document::{self, KIND, Run};
use crate::stats::publish::index::Index;
use crate::stats::publish::snapshot::Document;

#[cfg(test)]
mod tests;

/// Anything that can go wrong between a configured key and [`Keys`].
#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    /// Quoted rather than described: the operator wrote the reference,
    /// and what they wrote is the only thing that tells them which
    /// variable to export or which file to mount.
    #[error("`{reference}`, named by [publish].nsec, holds no signing key: {reason}")]
    Unresolved { reference: String, reason: String },

    /// The reason is the library's, which names the checksum or the
    /// length; repeating it in our own words would say less.
    #[error("{setting} is not a signing key: expected `nsec1…` or 64 hexadecimal characters")]
    Malformed { setting: String },
}

/// A key as written in the file: `nsec1…` or 64 hexadecimal characters.
///
/// Both spellings are accepted because both are what a key-management tool
/// hands an operator, and refusing one would be a rule with no reason
/// behind it. `setting` names where the value came from, so the error can
/// say `[publish].nsec` or the path of the file.
pub fn parse(raw: &str, setting: &str) -> Result<Keys, KeyError> {
    let value = raw.trim();
    let malformed = || KeyError::Malformed {
        setting: setting.to_string(),
    };

    let secret = if value.starts_with("nsec1") {
        SecretKey::from_bech32(value).map_err(|_| malformed())?
    } else {
        SecretKey::from_hex(value).map_err(|_| malformed())?
    };
    Ok(Keys::new(secret))
}

/// The key the reference names.
///
/// Read here rather than when the configuration loads, and only by a run
/// that is going to sign: `stats` on a machine that publishes nothing
/// neither needs the variable nor fails without it, and neither does
/// `publish --dry-run`, whose whole purpose is to review a snapshot
/// without a key being involved. `publish` resolves before it reads the
/// archive, so an unexported variable still fails in the first second.
pub fn resolve(reference: &SecretRef) -> Result<Keys, KeyError> {
    let secret = reference.resolve().map_err(|reason| KeyError::Unresolved {
        reference: reference.describe(),
        reason: reason.to_string(),
    })?;
    parse(secret.expose(), &format!("`{}`", reference.describe()))
}

/// One document, signed: kind 30666, the tag set of §11, and the
/// envelope as `content`.
///
/// The tags are built from the document's own address and revision rather
/// than passed in, so a caller cannot sign a document under an address it
/// does not have — which is the one error in publication that no reader
/// could detect.
pub fn sign(document: &Document, run: &Run, keys: &Keys) -> Event {
    let tags = document::tags(&document.address, run, Some(document.envelope.revision()))
        .into_iter()
        .map(|tag| Tag::custom(tag.name, tag.values))
        .collect::<Vec<_>>();

    EventBuilder::new(Kind::Custom(KIND), document.content())
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(run.generated_at.max(0) as u64))
        .finalize(keys)
        // Signing a well-formed event with a key that already parsed has
        // no failure mode of its own; treating it as one would put a
        // branch in every caller that nothing can reach.
        .expect("a document signs with a key that parsed")
}

/// The index, signed. Separate from [`sign`] because the index is not a
/// [`Document`]: §6 exempts it from the envelope the rest carry, so there
/// is no revision to tag and its `content` is the whole document.
pub fn sign_index(index: &Index, run: &Run, keys: &Keys) -> Event {
    let tags = document::tags(&index.address(), run, None)
        .into_iter()
        .map(|tag| Tag::custom(tag.name, tag.values))
        .collect::<Vec<_>>();

    EventBuilder::new(Kind::Custom(KIND), index.content())
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(run.generated_at.max(0) as u64))
        .finalize(keys)
        .expect("an index signs with a key that parsed")
}
