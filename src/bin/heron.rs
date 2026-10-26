//! Composition root. Wires adapters into services and starts the HTTP server.

use std::sync::Arc;
use std::time::Duration;

use heron::config::{CacheBackend, Config};
use heron::domain::calendar::ports::ErasedCalendarService;
use heron::domain::counts::ports::ErasedCountsService;
use heron::domain::health::ports::ErasedHealthService;
use heron::domain::stats::ports::{Cache, ErasedStatsService};
use heron::domain::{calendar, counts, health, stats};
use heron::inbound::http::{HttpServer, HttpServerConfig};
use heron::outbound::cache::layered::LayeredCache;
use heron::outbound::cache::memory::MemoryCache;
use heron::outbound::cache::steller::StellerCache;
use heron::outbound::clock::SystemClock;
use heron::outbound::github::GitHub;
use heron::outbound::tarball::Tarball;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Hands freed memory back to the OS, which glibc's malloc holds in per-thread
/// arenas after a counts measurement, keeping the process near `MemoryMax`.
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// An error ends the process with its chain on stderr, since tracing may not be up
/// yet, and a non-zero exit code, which systemd reads to decide whether to restart.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_env()?;

    // `RUST_LOG` comes from the environment, which dotenvy has already filled from
    // `.env`. When it is unset or does not parse, the filter is built from
    // `config.rust_log`, which defaults to `info`.
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.rust_log));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!(
        "heron v{} serving {} repositories, cache {:?}, github token {}",
        env!("CARGO_PKG_VERSION"),
        config.github_repos.len(),
        config.cache_backend,
        if config.github_token.is_some() {
            "set"
        } else {
            "not set"
        },
    );

    let github = GitHub::new(&config.github_api_base, config.github_token.clone())?;
    let tarball = Tarball::new(
        &config.github_api_base,
        config.github_token,
        config.measure_dir,
    )?;
    let retain = config.stats_retain;
    let settings = stats::services::Settings {
        repos: config.github_repos.clone(),
        fresh: config.stats_fresh,
        retain: config.stats_retain,
    };
    let counts_settings = counts::services::Settings {
        repos: config.github_repos,
        retain: config.counts_retain,
    };
    let calendar_settings = calendar::services::Settings {
        login: config.github_login,
        fresh: config.calendar_fresh,
        retain: config.stats_retain,
    };

    // Each arm builds services over a different cache type. Erasing them here is
    // what lets everything past this point hold one type.
    let Services {
        stats: stats_service,
        health: health_service,
        counts: counts_service,
        calendar: calendar_service,
    } = match config.cache_backend {
        CacheBackend::Memory => services(
            github,
            tarball,
            MemoryCache::new(),
            settings,
            counts_settings,
            calendar_settings,
        ),
        CacheBackend::Steller(address) => services(
            github,
            tarball,
            StellerCache::new(address)?,
            settings,
            counts_settings,
            calendar_settings,
        ),
        CacheBackend::Layered(address) => services(
            github,
            tarball,
            LayeredCache::new(StellerCache::new(address)?, MemoryCache::new(), retain),
            settings,
            counts_settings,
            calendar_settings,
        ),
    };

    tokio::spawn(sweeper(Arc::clone(&counts_service), config.counts_sweep));

    let server = HttpServer::new(
        stats_service,
        health_service,
        counts_service,
        calendar_service,
        HttpServerConfig {
            bind_address: &config.bind_address,
            allowed_origins: config.allowed_origins,
        },
    )
    .await?;
    server.run().await
}

/// The four services over one cache, erased.
struct Services {
    stats: Arc<dyn ErasedStatsService>,
    health: Arc<dyn ErasedHealthService>,
    counts: Arc<dyn ErasedCountsService>,
    calendar: Arc<dyn ErasedCalendarService>,
}

fn services<C: Cache>(
    github: GitHub,
    tarball: Tarball,
    cache: C,
    settings: stats::services::Settings,
    counts_settings: counts::services::Settings,
    calendar_settings: calendar::services::Settings,
) -> Services {
    let calendar = calendar::services::Service::new(
        github.clone(),
        cache.clone(),
        SystemClock,
        calendar_settings,
    );
    let stats = stats::services::Service::new(github, cache.clone(), SystemClock, settings);
    let counts = counts::services::Service::new(
        tarball,
        cache.clone(),
        stats.clone(),
        SystemClock,
        counts_settings,
    );
    Services {
        stats: Arc::new(stats),
        health: Arc::new(health::services::Service::new(cache)),
        counts: Arc::new(counts),
        calendar: Arc::new(calendar),
    }
}

/// Sweeps once at startup, then every `interval`. A pass that measured nothing is
/// logged at debug; one that did, at info.
async fn sweeper(counts: Arc<dyn ErasedCountsService>, interval: Duration) {
    loop {
        let sweep = counts.sweep().await;
        if sweep.measured.is_empty() && sweep.failed.is_empty() {
            tracing::debug!(
                "sweep: nothing to measure ({} up to date)",
                sweep.skipped.len()
            );
        } else {
            tracing::info!(
                "sweep: measured {}, failed {}, up to date {}",
                sweep.measured.len(),
                sweep.failed.len(),
                sweep.skipped.len()
            );
        }
        tokio::time::sleep(interval).await;
    }
}
