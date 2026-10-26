//! [`Login`], a validated GitHub account name.

use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::stats::models::repo_name::RepoName;

/// GitHub's limit on a user or organization name.
const LOGIN_MAX: usize = 39;

/// Why a [`Login`] could not be built.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum InvalidLogin {
    /// Empty, or whitespace only.
    #[error("login cannot be empty")]
    Empty,
    /// Longer than GitHub allows.
    #[error("login cannot exceed {LOGIN_MAX} characters")]
    TooLong,
    /// A character outside ASCII letters, digits, `-`, `_`, and `.`.
    #[error("login contains an invalid character: {0:?}")]
    InvalidCharacter(char),
    /// `.` or `..`.
    #[error("login cannot be . or ..")]
    Reserved,
}

/// A GitHub account name, such as the owner half of a [`RepoName`].
///
/// Holds the same characters a [`RepoName`] owner may: ASCII letters, digits, `-`,
/// `_`, and `.`, at most 39 of them, and never `.` or `..`. That is looser than
/// GitHub's own rule, so every repository owner is a valid `Login`, and it is still
/// safe to place in a cache key or a query as is.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Login(String);

impl Login {
    /// Validates `raw`, trimmed.
    ///
    /// # Errors
    ///
    /// The first [`InvalidLogin`] rule `raw` breaks, checked in the order the
    /// variants are declared.
    pub fn new(raw: impl AsRef<str>) -> Result<Self, InvalidLogin> {
        let trimmed = raw.as_ref().trim();
        if trimmed.is_empty() {
            return Err(InvalidLogin::Empty);
        }
        if trimmed.len() > LOGIN_MAX {
            return Err(InvalidLogin::TooLong);
        }
        if let Some(c) = trimmed
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
        {
            return Err(InvalidLogin::InvalidCharacter(c));
        }
        if matches!(trimmed, "." | "..") {
            return Err(InvalidLogin::Reserved);
        }
        Ok(Self(trimmed.to_string()))
    }

    /// The login as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The repository's owner. Infallible: [`RepoName`] already holds its owner to the
/// rules a [`Login`] checks.
impl From<&RepoName> for Login {
    fn from(repo: &RepoName) -> Self {
        Self(repo.owner().to_string())
    }
}

impl Deref for Login {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for Login {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for Login {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

/// Routes through [`Login::new`]. A derive would take the value off the wire and
/// skip validation.
impl<'de> Deserialize<'de> for Login {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Login::new(raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_login_is_trimmed() {
        assert_eq!(Login::new("  scadoshi \n").unwrap().as_str(), "scadoshi");
    }

    #[test]
    fn a_repository_owner_is_a_login() {
        for raw in ["scadoshi/steller", "a-b/c", "a_b.c/d", "A1/b2"] {
            let repo = RepoName::new(raw).unwrap();
            let login = Login::from(&repo);
            assert_eq!(Login::new(repo.owner()), Ok(login), "{raw}");
        }
    }

    #[test]
    fn rejects_empty_long_reserved_and_hostile_logins() {
        assert_eq!(Login::new("  "), Err(InvalidLogin::Empty));
        assert_eq!(
            Login::new("a".repeat(LOGIN_MAX + 1)),
            Err(InvalidLogin::TooLong)
        );
        assert_eq!(Login::new(".."), Err(InvalidLogin::Reserved));
        for raw in ["a/b", "a b", "a\r\nb", "a:b", "ä"] {
            assert!(
                matches!(Login::new(raw), Err(InvalidLogin::InvalidCharacter(_))),
                "{raw}"
            );
        }
    }

    #[test]
    fn a_login_serializes_as_a_bare_string_and_deserializes_through_new() {
        let login = Login::new("scadoshi").unwrap();
        assert_eq!(serde_json::to_string(&login).unwrap(), "\"scadoshi\"");
        assert_eq!(
            serde_json::from_str::<Login>("\"scadoshi\"").unwrap(),
            login
        );
        assert!(serde_json::from_str::<Login>("\"a/b\"").is_err());
    }
}
