//! The signing key, and the signed event a document becomes —
//! `docs/NOSTR-PUBLICATION.md` §12 and §11.
//!
//! Responsibility: turn a key an operator configured into [`Keys`], and a
//! [`Document`] the stats crate computed into an event a relay will accept.
//! Nothing here talks to a relay; that is [`super::client`].
//!
//! # Why the key is never a flag
//!
//! §12 says the key comes from `[publish].nsec` or a file, and never from
//! a command-line flag. A flag is readable in `ps` by every user on the
//! machine and lands in the shell history of the one who typed it; a
//! configuration file has permissions, and a file path has them without
//! the key ever being pasted anywhere at all.
//!
//! # Why the event's clock is the run's
//!
//! `created_at` is the snapshot's `generated_at`, not the moment the
//! signature was produced. Every document of a run then carries the same
//! timestamp, which is what §7 means by a snapshot being one reading of
//! the archive — and it keeps a run that takes a minute to sign from
//! looking like a minute of separate publications.

use std::path::Path;

use nostr_sdk::prelude::*;

use crate::stats::publish::document::{self, KIND, Run};
use crate::stats::publish::snapshot::Document;

#[cfg(test)]
mod tests;

/// Anything that can go wrong between a configured key and [`Keys`].
#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("could not read the signing key from `{path}`")]
    Unreadable {
        path: String,
        #[source]
        source: std::io::Error,
    },

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

/// The key an operator configured, if they configured one.
///
/// The file is read here rather than at startup: a `stats` invocation that
/// never publishes should not open the secret, and should not fail because
/// the machine it runs on does not hold one. The two arguments are already
/// known to be mutually exclusive — the configuration refuses a file
/// alongside an inline key — so the order they are tried in is not a
/// precedence.
pub fn resolve(nsec: Option<&str>, file: Option<&Path>) -> Result<Option<Keys>, KeyError> {
    if let Some(raw) = nsec {
        return parse(raw, "[publish].nsec").map(Some);
    }

    let Some(path) = file else {
        return Ok(None);
    };
    let raw = std::fs::read_to_string(path).map_err(|source| KeyError::Unreadable {
        path: path.display().to_string(),
        source,
    })?;
    parse(&raw, &format!("the key file `{}`", path.display())).map(Some)
}

/// One document, signed: kind 30666, the tag set of §11, and the
/// envelope as `content`.
///
/// The tags are built from the document's own address and revision rather
/// than passed in, so a caller cannot sign a document under an address it
/// does not have — which is the one error in publication that no reader
/// could detect.
pub fn sign(document: &Document, run: &Run, keys: &Keys) -> Event {
    let tags = document::tags(&document.address, run, document.envelope.revision())
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
