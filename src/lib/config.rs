//! Configuration from environment variables, validated at startup.
//!
//! Everything is checked before the server binds. A bad value stops the process with
//! an error naming the variable.

use crate::domain::{secret::Secret, stats::models::repo_name::RepoName};
use anyhow::{Context, anyhow, bail};
use axum::http::HeaderValue;
use std::{net::SocketAddr, path::PathBuf, time::Duration};

/// Address to bind the HTTP server to.
const BIND_ADDRESS_KEY: &str = "BIND_ADDRESS";

/// Origins allowed by CORS, comma-separated.
const ALLOWED_ORIGINS_KEY: &str = "ALLOWED_ORIGINS";

/// Repositories to serve, comma-separated `owner/name`.
const GITHUB_REPOS_KEY: &str = "GITHUB_REPOS";

/// GitHub token. Optional.
const GITHUB_TOKEN_KEY: &str = "GITHUB_TOKEN";

/// Base URL of GitHub's API.
const GITHUB_API_BASE_KEY: &str = "GITHUB_API_BASE";
const GITHUB_API_BASE_DEFAULT: &str = "https://api.github.com";

/// Which cache adapter to run: `memory`, `steller`, or `layered`.
const CACHE_BACKEND_KEY: &str = "CACHE_BACKEND";
const CACHE_BACKEND_DEFAULT: &str = "memory";

/// Address of steller. Required when the backend is `steller` or `layered`.
const STELLER_ADDRESS_KEY: &str = "STELLER_ADDRESS";

/// Seconds a snapshot is served before GitHub is asked again.
const STATS_FRESH_SECS_KEY: &str = "STATS_FRESH_SECS";
const STATS_FRESH_SECS_DEFAULT: u64 = 6 * 60 * 60;

/// Seconds the cache keeps a snapshot.
const STATS_RETAIN_SECS_KEY: &str = "STATS_RETAIN_SECS";
const STATS_RETAIN_SECS_DEFAULT: u64 = 7 * 24 * 60 * 60;

/// Seconds between sweeps that re-measure repositories pushed to since last time.
const COUNTS_SWEEP_SECS_KEY: &str = "COUNTS_SWEEP_SECS";
const COUNTS_SWEEP_SECS_DEFAULT: u64 = 5 * 60;

/// Seconds the cache keeps a measurement.
const COUNTS_RETAIN_SECS_KEY: &str = "COUNTS_RETAIN_SECS";
const COUNTS_RETAIN_SECS_DEFAULT: u64 = 7 * 24 * 60 * 60;

/// Directory tarballs are unpacked under while being measured.
const MEASURE_DIR_KEY: &str = "MEASURE_DIR";

/// Tracing filter directives.
const RUST_LOG_KEY: &str = "RUST_LOG";
const RUST_LOG_DEFAULT: &str = "info";

/// The cache adapter to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheBackend {
    /// An in-process map.
    Memory,
    /// steller at this address.
    Steller(SocketAddr),
    /// steller at this address, with an in-process map behind it.
    Layered(SocketAddr),
}

/// Application configuration.
#[derive(Debug)]
pub struct Config {
    /// Address to bind the HTTP server to (e.g. `127.0.0.1:3100`).
    pub bind_address: String,

    /// Origins allowed by CORS.
    pub allowed_origins: Vec<HeaderValue>,

    /// The allowlist: the only repositories the server will read or serve. Never
    /// empty, and holds no repository twice.
    pub github_repos: Vec<RepoName>,

    /// GitHub token. Without one GitHub allows 60 requests an hour per IP.
    pub github_token: Option<Secret>,

    /// Base URL of GitHub's API. Overridden in tests to reach a local fake.
    pub github_api_base: String,

    /// The cache adapter to run.
    pub cache_backend: CacheBackend,

    /// How long a snapshot is served before GitHub is asked again.
    pub stats_fresh: Duration,

    /// How long the cache keeps a snapshot. Always longer than `stats_fresh`.
    pub stats_retain: Duration,

    /// How often the sweep looks for repositories to measure again.
    pub counts_sweep: Duration,

    /// How long the cache keeps a measurement.
    pub counts_retain: Duration,

    /// Where tarballs are unpacked while being measured. Emptied as it goes.
    pub measure_dir: PathBuf,

    /// Tracing filter. A bare level (`info`) or per-target directives
    /// (`info,heron=debug`).
    pub rust_log: String,
}

impl Config {
    /// Loads configuration from the environment, reading a `.env` file first when
    /// one exists.
    ///
    /// # Errors
    ///
    /// Returns an error if a required variable is missing or any value is invalid.
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok();
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Loads configuration from `lookup`, which answers a variable's value by name.
    /// A variable that is set and blank counts as unset.
    ///
    /// # Errors
    ///
    /// Returns an error if a required variable is missing or any value is invalid.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let optional = |key: &str| lookup(key).filter(|value| !value.trim().is_empty());
        let required = |key: &str| {
            optional(key).with_context(|| format!("failed to get variable from env: {key}"))
        };

        let bind_address = required(BIND_ADDRESS_KEY)?;

        let allowed_origins = required(ALLOWED_ORIGINS_KEY)?
            .split(',')
            .map(|origin| {
                origin
                    .trim()
                    .parse::<HeaderValue>()
                    .with_context(|| format!("invalid origin in {ALLOWED_ORIGINS_KEY}: {origin:?}"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        let mut github_repos = Vec::new();
        for raw in required(GITHUB_REPOS_KEY)?.split(',') {
            let repo = RepoName::new(raw)
                .with_context(|| format!("invalid repository in {GITHUB_REPOS_KEY}: {raw:?}"))?;
            if github_repos.contains(&repo) {
                bail!("{GITHUB_REPOS_KEY} lists {repo} twice");
            }
            github_repos.push(repo);
        }

        let github_token = optional(GITHUB_TOKEN_KEY)
            .map(Secret::new)
            .transpose()
            .with_context(|| format!("invalid {GITHUB_TOKEN_KEY}"))?;

        let github_api_base =
            optional(GITHUB_API_BASE_KEY).unwrap_or_else(|| GITHUB_API_BASE_DEFAULT.to_string());

        let backend = optional(CACHE_BACKEND_KEY)
            .unwrap_or_else(|| CACHE_BACKEND_DEFAULT.to_string())
            .trim()
            .to_ascii_lowercase();
        let cache_backend = match backend.as_str() {
            "memory" => CacheBackend::Memory,
            "steller" => CacheBackend::Steller(steller_address(&required)?),
            "layered" => CacheBackend::Layered(steller_address(&required)?),
            other => bail!(
                "invalid {CACHE_BACKEND_KEY}: {other:?} (expected memory, steller, or layered)"
            ),
        };

        let stats_fresh = seconds(&optional, STATS_FRESH_SECS_KEY, STATS_FRESH_SECS_DEFAULT)?;
        let stats_retain = seconds(&optional, STATS_RETAIN_SECS_KEY, STATS_RETAIN_SECS_DEFAULT)?;
        if stats_retain <= stats_fresh {
            bail!(
                "{STATS_RETAIN_SECS_KEY} ({}) must exceed {STATS_FRESH_SECS_KEY} ({}), or a snapshot is dropped the moment it goes stale and nothing can cover for GitHub",
                stats_retain.as_secs(),
                stats_fresh.as_secs()
            );
        }

        let counts_sweep = seconds(&optional, COUNTS_SWEEP_SECS_KEY, COUNTS_SWEEP_SECS_DEFAULT)?;
        let counts_retain = seconds(
            &optional,
            COUNTS_RETAIN_SECS_KEY,
            COUNTS_RETAIN_SECS_DEFAULT,
        )?;
        let measure_dir = optional(MEASURE_DIR_KEY).map_or_else(
            || std::env::temp_dir().join("heron-measure"),
            |dir| PathBuf::from(dir.trim()),
        );

        let rust_log = optional(RUST_LOG_KEY).unwrap_or_else(|| RUST_LOG_DEFAULT.to_string());

        Ok(Self {
            bind_address,
            allowed_origins,
            github_repos,
            github_token,
            github_api_base,
            cache_backend,
            stats_fresh,
            stats_retain,
            counts_sweep,
            counts_retain,
            measure_dir,
            rust_log,
        })
    }
}

/// Reads and checks `STELLER_ADDRESS`.
fn steller_address(
    required: &impl Fn(&str) -> anyhow::Result<String>,
) -> anyhow::Result<SocketAddr> {
    let raw = required(STELLER_ADDRESS_KEY)?;
    let address: SocketAddr = raw.trim().parse().map_err(|_| {
        anyhow!("invalid {STELLER_ADDRESS_KEY}: {raw:?} (expected ip:port, such as 127.0.0.1:3000)")
    })?;
    if !address.ip().is_loopback() {
        bail!(
            "invalid {STELLER_ADDRESS_KEY}: {address} is not loopback. steller has no AUTH, no TLS and no ACL, so it is only safe to reach on 127.0.0.1 or ::1"
        );
    }
    Ok(address)
}

/// Reads a positive number of seconds, or the default when unset.
fn seconds(
    optional: &impl Fn(&str) -> Option<String>,
    key: &str,
    default: u64,
) -> anyhow::Result<Duration> {
    let Some(raw) = optional(key) else {
        return Ok(Duration::from_secs(default));
    };
    match raw.trim().parse::<u64>() {
        Ok(secs) if secs > 0 => Ok(Duration::from_secs(secs)),
        _ => bail!("invalid {key}: {raw:?} (expected a positive number of seconds)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const REQUIRED: [(&str, &str); 3] = [
        ("BIND_ADDRESS", "127.0.0.1:3100"),
        (
            "ALLOWED_ORIGINS",
            "https://scottyfermo.com, http://localhost:8080",
        ),
        ("GITHUB_REPOS", "scadoshi/steller, scadoshi/chickadee"),
    ];

    /// The required variables, then `extra` laid over them. An empty value unsets.
    fn load(extra: &[(&str, &str)]) -> anyhow::Result<Config> {
        let vars: HashMap<&str, &str> = REQUIRED.iter().chain(extra).copied().collect();
        Config::from_lookup(|key| vars.get(key).map(ToString::to_string))
    }

    fn error(extra: &[(&str, &str)]) -> String {
        format!("{:#}", load(extra).unwrap_err())
    }

    #[test]
    fn the_required_variables_alone_are_enough() {
        let config = load(&[]).unwrap();
        assert_eq!(config.bind_address, "127.0.0.1:3100");
        assert_eq!(config.allowed_origins.len(), 2);
        assert_eq!(config.allowed_origins[1], "http://localhost:8080");
        assert_eq!(&*config.github_repos[1], "scadoshi/chickadee");
        assert!(config.github_token.is_none());
        assert_eq!(config.github_api_base, "https://api.github.com");
        assert_eq!(config.cache_backend, CacheBackend::Memory);
        assert_eq!(config.stats_fresh, Duration::from_hours(6));
        assert_eq!(config.stats_retain, Duration::from_hours(7 * 24));
        assert_eq!(config.rust_log, "info");
    }

    #[test]
    fn each_required_variable_is_named_when_missing() {
        for (key, _) in REQUIRED {
            let message = error(&[(key, "")]);
            assert!(message.contains(key), "{message}");
        }
    }

    #[test]
    fn a_repository_that_does_not_parse_stops_startup() {
        let message = error(&[("GITHUB_REPOS", "scadoshi/steller,../etc")]);
        assert!(message.contains("GITHUB_REPOS"), "{message}");
        assert!(message.contains("../etc"), "{message}");
    }

    #[test]
    fn a_repository_listed_twice_stops_startup() {
        let message = error(&[("GITHUB_REPOS", "a/b,c/d, a/b")]);
        assert!(message.contains("twice"), "{message}");
    }

    #[test]
    fn the_token_is_held_redacted_and_blank_means_none() {
        let config = load(&[("GITHUB_TOKEN", "ghp_hunter2")]).unwrap();
        assert_eq!(config.github_token.as_ref().unwrap().read(), "ghp_hunter2");
        assert!(!format!("{config:?}").contains("hunter2"));
        assert!(
            load(&[("GITHUB_TOKEN", "  ")])
                .unwrap()
                .github_token
                .is_none()
        );
    }

    #[test]
    fn steller_backends_take_a_loopback_address() {
        let address = "127.0.0.1:3000".parse().unwrap();
        for (backend, expected) in [
            ("steller", CacheBackend::Steller(address)),
            ("layered", CacheBackend::Layered(address)),
            (" Layered ", CacheBackend::Layered(address)),
        ] {
            let config = load(&[
                ("CACHE_BACKEND", backend),
                ("STELLER_ADDRESS", "127.0.0.1:3000"),
            ])
            .unwrap();
            assert_eq!(config.cache_backend, expected);
        }
    }

    #[test]
    fn steller_backends_need_the_address() {
        let message = error(&[("CACHE_BACKEND", "steller")]);
        assert!(message.contains("STELLER_ADDRESS"), "{message}");
    }

    #[test]
    fn a_steller_address_off_loopback_is_refused_with_the_reason() {
        for address in ["10.0.0.5:3000", "0.0.0.0:3000"] {
            let message = error(&[("CACHE_BACKEND", "layered"), ("STELLER_ADDRESS", address)]);
            assert!(message.contains("not loopback"), "{message}");
            assert!(message.contains("no AUTH"), "{message}");
        }
    }

    #[test]
    fn a_steller_address_must_be_an_ip_and_port() {
        let message = error(&[
            ("CACHE_BACKEND", "steller"),
            ("STELLER_ADDRESS", "localhost:3000"),
        ]);
        assert!(message.contains("expected ip:port"), "{message}");
    }

    #[test]
    fn the_memory_backend_ignores_the_steller_address() {
        let config = load(&[("STELLER_ADDRESS", "10.0.0.5:3000")]).unwrap();
        assert_eq!(config.cache_backend, CacheBackend::Memory);
    }

    #[test]
    fn an_unknown_backend_is_refused() {
        let message = error(&[("CACHE_BACKEND", "redis")]);
        assert!(message.contains("CACHE_BACKEND"), "{message}");
        assert!(message.contains("redis"), "{message}");
    }

    #[test]
    fn windows_must_be_positive_numbers() {
        for value in ["0", "-5", "six hours", "1.5"] {
            let message = error(&[("STATS_FRESH_SECS", value)]);
            assert!(message.contains("STATS_FRESH_SECS"), "{value}: {message}");
        }
    }

    #[test]
    fn the_retain_window_must_exceed_the_fresh_window() {
        for (fresh, retain) in [("600", "600"), ("600", "60")] {
            let message = error(&[("STATS_FRESH_SECS", fresh), ("STATS_RETAIN_SECS", retain)]);
            assert!(message.contains("must exceed"), "{message}");
        }
        assert!(load(&[("STATS_FRESH_SECS", "600"), ("STATS_RETAIN_SECS", "601")]).is_ok());
    }

    #[test]
    fn an_origin_that_is_not_a_header_value_is_refused() {
        let message = error(&[("ALLOWED_ORIGINS", "https://ok.test,bad\norigin")]);
        assert!(message.contains("ALLOWED_ORIGINS"), "{message}");
    }

    #[test]
    fn counts_settings_default_and_parse() {
        let config = load(&[]).unwrap();
        assert_eq!(config.counts_sweep, Duration::from_mins(5));
        assert_eq!(config.counts_retain, Duration::from_hours(168));
        assert!(config.measure_dir.ends_with("heron-measure"));

        let config = load(&[
            ("COUNTS_SWEEP_SECS", "60"),
            ("COUNTS_RETAIN_SECS", "3600"),
            ("MEASURE_DIR", " /var/tmp/measure "),
        ])
        .unwrap();
        assert_eq!(config.counts_sweep, Duration::from_mins(1));
        assert_eq!(config.counts_retain, Duration::from_hours(1));
        assert_eq!(config.measure_dir, PathBuf::from("/var/tmp/measure"));

        let error = load(&[("COUNTS_SWEEP_SECS", "soon")])
            .unwrap_err()
            .to_string();
        assert!(error.contains("COUNTS_SWEEP_SECS"), "{error}");
    }
}
