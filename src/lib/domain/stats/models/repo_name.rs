use serde::{Deserialize, Serialize};
use std::ops::Deref;
use thiserror::Error;

/// GitHub's limit on a user or organization name.
const OWNER_MAX: usize = 39;

/// GitHub's limit on a repository name.
const NAME_MAX: usize = 100;

/// Why a [`RepoName`] could not be built.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum InvalidRepoName {
    /// No `/` between owner and name.
    #[error("repository must be written owner/name")]
    MissingSlash,
    /// More than one `/`.
    #[error("repository must contain exactly one slash")]
    TooManySlashes,
    /// Nothing before the slash.
    #[error("repository owner cannot be empty")]
    EmptyOwner,
    /// Nothing after the slash.
    #[error("repository name cannot be empty")]
    EmptyName,
    /// Owner longer than GitHub allows.
    #[error("repository owner cannot exceed {OWNER_MAX} characters")]
    OwnerTooLong,
    /// Name longer than GitHub allows.
    #[error("repository name cannot exceed {NAME_MAX} characters")]
    NameTooLong,
    /// A character outside ASCII letters, digits, `-`, `_`, and `.`.
    #[error("repository contains an invalid character: {0:?}")]
    InvalidCharacter(char),
    /// `.` or `..` as either half.
    #[error("repository owner and name cannot be . or ..")]
    Reserved,
}

/// A GitHub repository written `owner/name`.
///
/// Both halves hold only ASCII letters, digits, `-`, `_`, and `.`, and neither is `.`
/// or `..`. A value of this type is therefore safe to place in a URL path or a cache
/// key as is.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RepoName {
    full: String,
    owner_len: usize,
}

impl RepoName {
    /// Validates `raw`, trimmed.
    pub fn new(raw: impl AsRef<str>) -> Result<Self, InvalidRepoName> {
        let trimmed = raw.as_ref().trim();
        let (owner, name) = trimmed
            .split_once('/')
            .ok_or(InvalidRepoName::MissingSlash)?;
        if name.contains('/') {
            return Err(InvalidRepoName::TooManySlashes);
        }
        if owner.is_empty() {
            return Err(InvalidRepoName::EmptyOwner);
        }
        if name.is_empty() {
            return Err(InvalidRepoName::EmptyName);
        }
        if owner.len() > OWNER_MAX {
            return Err(InvalidRepoName::OwnerTooLong);
        }
        if name.len() > NAME_MAX {
            return Err(InvalidRepoName::NameTooLong);
        }
        if let Some(c) = trimmed
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')))
        {
            return Err(InvalidRepoName::InvalidCharacter(c));
        }
        if matches!(owner, "." | "..") || matches!(name, "." | "..") {
            return Err(InvalidRepoName::Reserved);
        }
        Ok(Self {
            full: trimmed.to_string(),
            owner_len: owner.len(),
        })
    }

    /// The half before the slash.
    pub fn owner(&self) -> &str {
        self.full.get(..self.owner_len).unwrap_or_default()
    }

    /// The half after the slash.
    pub fn name(&self) -> &str {
        self.owner_len
            .checked_add(1)
            .and_then(|start| self.full.get(start..))
            .unwrap_or_default()
    }
}

impl Deref for RepoName {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.full
    }
}

impl std::fmt::Display for RepoName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.full)
    }
}

impl Serialize for RepoName {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.full)
    }
}

/// Routes through [`RepoName::new`]. A derive would take the fields off the wire and
/// skip validation.
impl<'de> Deserialize<'de> for RepoName {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        RepoName::new(raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_owner_and_name() {
        let repo = RepoName::new("scadoshi/steller").unwrap();
        assert_eq!(repo.owner(), "scadoshi");
        assert_eq!(repo.name(), "steller");
        assert_eq!(&*repo, "scadoshi/steller");
    }

    #[test]
    fn accepts_dots_dashes_and_underscores() {
        for raw in ["scadoshi/.claude", "a-b/c_d.e", "A1/b2"] {
            assert!(RepoName::new(raw).is_ok(), "{raw}");
        }
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(&*RepoName::new("  a/b \n").unwrap(), "a/b");
    }

    #[test]
    fn rejects_path_traversal() {
        assert_eq!(RepoName::new("../x"), Err(InvalidRepoName::Reserved));
        assert_eq!(RepoName::new("x/.."), Err(InvalidRepoName::Reserved));
        assert_eq!(RepoName::new("./x"), Err(InvalidRepoName::Reserved));
    }

    #[test]
    fn rejects_extra_slashes() {
        assert_eq!(RepoName::new("a/b/c"), Err(InvalidRepoName::TooManySlashes));
    }

    #[test]
    fn rejects_a_missing_slash() {
        assert_eq!(RepoName::new("steller"), Err(InvalidRepoName::MissingSlash));
    }

    #[test]
    fn rejects_an_empty_half() {
        assert_eq!(RepoName::new("/b"), Err(InvalidRepoName::EmptyOwner));
        assert_eq!(RepoName::new("a/"), Err(InvalidRepoName::EmptyName));
    }

    #[test]
    fn rejects_whitespace_and_control_characters() {
        assert_eq!(
            RepoName::new("a b/c"),
            Err(InvalidRepoName::InvalidCharacter(' '))
        );
        assert_eq!(
            RepoName::new("a/b\r\nHost: evil"),
            Err(InvalidRepoName::InvalidCharacter('\r'))
        );
    }

    #[test]
    fn rejects_url_metacharacters() {
        for raw in ["a/b?x=1", "a/b#frag", "a/b%2e", "a/b:80", "a/b@c", "ä/b"] {
            assert!(
                matches!(
                    RepoName::new(raw),
                    Err(InvalidRepoName::InvalidCharacter(_))
                ),
                "{raw}"
            );
        }
    }

    #[test]
    fn rejects_halves_past_the_length_limits() {
        let owner = "a".repeat(OWNER_MAX + 1);
        let name = "b".repeat(NAME_MAX + 1);
        assert_eq!(
            RepoName::new(format!("{owner}/b")),
            Err(InvalidRepoName::OwnerTooLong)
        );
        assert_eq!(
            RepoName::new(format!("a/{name}")),
            Err(InvalidRepoName::NameTooLong)
        );
    }

    #[test]
    fn serializes_as_a_bare_string() {
        let repo = RepoName::new("a/b").unwrap();
        assert_eq!(serde_json::to_string(&repo).unwrap(), "\"a/b\"");
    }

    #[test]
    fn deserialize_rejects_what_new_rejects() {
        assert!(serde_json::from_str::<RepoName>("\"a/b\"").is_ok());
        for raw in ["\"../x\"", "\"a/b/c\"", "\"a b/c\"", "\"\""] {
            assert!(serde_json::from_str::<RepoName>(raw).is_err(), "{raw}");
        }
    }
}
