//! [`CacheKey`], the versioned key a snapshot is stored under.

use std::fmt;
use std::ops::Deref;

use crate::domain::login::Login;
use crate::domain::stats::models::repo_name::RepoName;

/// Version of the cached payloads' shape, shared by every kind of key. Bump it when
/// [`Snapshot`], [`RepoStats`], [`Counts`] or [`Calendar`] changes, so a payload
/// written by an older build is a miss and not a parse error.
///
/// [`Snapshot`]: crate::domain::stats::models::snapshot::Snapshot
/// [`RepoStats`]: crate::domain::stats::models::repo_stats::RepoStats
/// [`Counts`]: crate::domain::counts::models::Counts
/// [`Calendar`]: crate::domain::calendar::models::Calendar
const SCHEMA_VERSION: u32 = 1;

/// What a cached snapshot holds, which decides how long it is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyKind {
    /// A repository's stats.
    Stats,
    /// A repository's source counts.
    Counts,
    /// An account's contribution calendar.
    Calendar,
}

/// Key a snapshot is stored under: `heron:<kind>:v<SCHEMA_VERSION>:<id>`, where the
/// kind is `stats`, `counts` or `calendar` and the id is a repository or a login.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    kind: KeyKind,
    key: String,
}

impl CacheKey {
    /// The key for `repo` at the current schema version.
    pub fn for_repo(repo: &RepoName) -> Self {
        Self {
            kind: KeyKind::Stats,
            key: format!("heron:stats:v{SCHEMA_VERSION}:{repo}"),
        }
    }

    /// The key for `repo`'s source counts: `heron:counts:v1:owner/name`.
    pub fn for_counts(repo: &RepoName) -> Self {
        Self {
            kind: KeyKind::Counts,
            key: format!("heron:counts:v{SCHEMA_VERSION}:{repo}"),
        }
    }

    /// The key for an account's contribution calendar: `heron:calendar:v1:login`.
    pub fn for_calendar(login: &Login) -> Self {
        Self {
            kind: KeyKind::Calendar,
            key: format!("heron:calendar:v{SCHEMA_VERSION}:{login}"),
        }
    }

    /// What the snapshot under this key holds.
    pub fn kind(&self) -> KeyKind {
        self.kind
    }
}

impl Deref for CacheKey {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.key
    }
}

impl fmt::Display for CacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_prefix_version_and_repo() {
        let repo = RepoName::new("scadoshi/steller").unwrap();
        assert_eq!(
            &*CacheKey::for_repo(&repo),
            "heron:stats:v1:scadoshi/steller"
        );
        assert_eq!(
            &*CacheKey::for_counts(&repo),
            "heron:counts:v1:scadoshi/steller"
        );
    }
}
