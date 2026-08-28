//! Secrets, and the `env:NAME` and `file:PATH` references that stand in
//! for them.
//!
//! # Why a reference and not the value
//!
//! `settings.toml` is a file operators paste into issues, copy between
//! machines and commit to a private repository. A signing key written in
//! it is a signing key in every one of those places, and no amount of
//! care at the point of use undoes that. So the file never holds the key:
//! it holds a *reference* to where the key lives, and [`SecretRef`] is
//! the only shape `[publish].nsec` will deserialize into.
//!
//! That is a type-level guarantee rather than a rule in the
//! documentation. `Settings` cannot carry a secret, because the field
//! that would carry one cannot be built from a literal.
//!
//! # The two places a key lives
//!
//! §12 names both: an environment variable and a file path. Neither is a
//! secret, so both keep the guarantee above, and each is the natural one
//! somewhere. `env:NAME` is what a systemd unit or a shell profile hands
//! a process. `file:/run/secrets/nsec` is what Docker and Kubernetes
//! mount, and it is the better of the two there: an environment is
//! readable in `/proc/<pid>/environ` and comes back out of
//! `docker inspect`, while a mounted file has permissions of its own.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer};

/// Prefix marking a configured string as an environment-variable
/// reference.
const ENV_PREFIX: &str = "env:";

/// Marker printed instead of a secret.
const REDACTED: &str = "[redacted]";

/// Prefix marking a configured string as a path to a file holding a
/// secret.
const FILE_PREFIX: &str = "file:";

/// Where a secret lives: never the secret, always the way to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretRef {
    /// The name of an environment variable, written in the file as
    /// `"env:BESTIARIO_PUBLISH_NSEC"`.
    Env(String),
    /// The path of a file whose contents are the secret, written as
    /// `"file:/run/secrets/bestiario-nsec"`.
    File(PathBuf),
}

impl SecretRef {
    /// The reference as it was written — safe to print, and the only
    /// useful thing to say when it turns out to lead nowhere. A path is
    /// not a secret; the file it names is.
    pub fn describe(&self) -> String {
        match self {
            Self::Env(name) => format!("{ENV_PREFIX}{name}"),
            Self::File(path) => format!("{FILE_PREFIX}{}", path.display()),
        }
    }

    /// The secret the reference leads to, through readers of the caller's
    /// choosing.
    ///
    /// Taking the lookups rather than reading the process environment and
    /// the filesystem directly is what makes the resolution testable
    /// without a test mutating the environment of every other test
    /// running beside it.
    pub fn read<E, F>(&self, from_env: E, from_file: F) -> Result<Secret, Unresolved>
    where
        E: FnOnce(&str) -> Option<String>,
        F: FnOnce(&Path) -> std::io::Result<String>,
    {
        match self {
            Self::Env(name) => from_env(name).map(Secret).ok_or(Unresolved::NotSet),
            // Trimmed because a file written with `echo` ends in a
            // newline, and a key that differs from the operator's by one
            // invisible byte is the least debuggable failure there is.
            Self::File(path) => from_file(path)
                .map(|raw| Secret(raw.trim().to_string()))
                .map_err(|source| Unresolved::Unreadable {
                    reason: source.to_string(),
                }),
        }
    }

    /// The secret the reference leads to in this process.
    pub fn resolve(&self) -> Result<Secret, Unresolved> {
        self.read(
            |name| std::env::var(name).ok(),
            |path| std::fs::read_to_string(path),
        )
    }
}

/// Why a reference led to no secret. Carries no value and no path
/// contents — only what an operator needs in order to fix it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Unresolved {
    #[error("it is not set")]
    NotSet,
    #[error("it could not be read: {reason}")]
    Unreadable { reason: String },
}

/// Refuses anything that is neither an `env:NAME` nor a `file:PATH`
/// reference, so a key pasted into the file is a configuration error and
/// never a working setup that happens to leak.
impl<'de> Deserialize<'de> for SecretRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;

        if let Some(name) = raw.strip_prefix(ENV_PREFIX) {
            let name = name.trim();
            if name.is_empty() {
                return Err(serde::de::Error::custom(format!(
                    "`{ENV_PREFIX}` names no variable"
                )));
            }
            return Ok(Self::Env(name.to_string()));
        }

        if let Some(path) = raw.strip_prefix(FILE_PREFIX) {
            let path = path.trim();
            if path.is_empty() {
                return Err(serde::de::Error::custom(format!(
                    "`{FILE_PREFIX}` names no file"
                )));
            }
            return Ok(Self::File(PathBuf::from(path)));
        }

        Err(serde::de::Error::custom(format!(
            "expected a reference of the form `{ENV_PREFIX}NAME` or \
             `{FILE_PREFIX}PATH`, so that the secret itself is not written \
             into the configuration file"
        )))
    }
}

/// A secret value, read from wherever its reference pointed.
///
/// `Debug` prints a marker: a secret that reaches a log line reaches
/// every place that log line is pasted. [`Secret::expose`] is the one way
/// to the value, so every exposure is greppable at its call site.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

#[cfg(test)]
mod tests;
