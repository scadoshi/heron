use crate::domain::stats::models::repo_name::RepoName;
use std::ops::Deref;

/// Version of the cached payload's shape. Bump it when [`Snapshot`] or [`RepoStats`]
/// changes, so a payload written by an older build is a miss and not a parse error.
///
/// [`Snapshot`]: crate::domain::stats::models::snapshot::Snapshot
/// [`RepoStats`]: crate::domain::stats::models::repo_stats::RepoStats
const SCHEMA_VERSION: u32 = 1;

/// Key a repository's snapshot is stored under: `scotland:stats:v1:owner/name`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey(String);

impl CacheKey {
    /// The key for `repo` at the current schema version.
    pub fn for_repo(repo: &RepoName) -> Self {
        Self(format!("scotland:stats:v{SCHEMA_VERSION}:{repo}"))
    }

    /// The key as bytes, the form a cache adapter sends.
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl Deref for CacheKey {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::fmt::Display for CacheKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
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
            "scotland:stats:v1:scadoshi/steller"
        );
    }
}
