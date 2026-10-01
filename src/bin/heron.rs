//! Composition root. Wires adapters into services and starts the HTTP server.

use heron::{
    config::{CacheBackend, Config},
    domain::{
        counts::{self, ports::ErasedCountsService},
        health::{self, ports::ErasedHealthService},
        stats::{
            self,
            ports::{ErasedStatsService, StatsCache},
            services::Settings,
        },
    },
    inbound::http::{HttpServer, HttpServerConfig},
    outbound::{
        cache::{layered::LayeredCache, memory::MemoryCache, steller::StellerCache},
        clock::SystemClock,
        github::GitHub,
        tarball::Tarball,
    },
};
use std::{process::ExitCode, sync::Arc, time::Duration};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Tracing may not be up yet, and systemd reads the exit code to decide
            // whether to restart.
            #[allow(clippy::print_stderr)]
            {
                eprintln!("heron failed: {error:#}");
            }
            ExitCode::FAILURE
        }
    }
}

async fn run() -> anyhow::Result<()> {
    let config = Config::from_env()?;

    // `RUST_LOG` from the process wins. The value from `.env` covers when it is unset.
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
    let settings = Settings {
        repos: config.github_repos.clone(),
        fresh: config.stats_fresh,
        retain: config.stats_retain,
    };
    let counts_settings = counts::services::Settings {
        repos: config.github_repos,
        retain: config.counts_retain,
    };

    // Each arm builds services over a different cache type. Erasing them here is
    // what lets everything past this point hold one type.
    let (stats_service, health_service, counts_service) = match config.cache_backend {
        CacheBackend::Memory => services(
            github,
            tarball,
            MemoryCache::new(),
            settings,
            counts_settings,
        ),
        CacheBackend::Steller(address) => services(
            github,
            tarball,
            StellerCache::new(address)?,
            settings,
            counts_settings,
        ),
        CacheBackend::Layered(address) => services(
            github,
            tarball,
            LayeredCache::new(StellerCache::new(address)?, MemoryCache::new(), retain),
            settings,
            counts_settings,
        ),
    };

    tokio::spawn(sweeper(Arc::clone(&counts_service), config.counts_sweep));

    let server = HttpServer::new(
        stats_service,
        health_service,
        counts_service,
        HttpServerConfig {
            bind_address: &config.bind_address,
            allowed_origins: config.allowed_origins,
        },
    )
    .await?;
    server.run().await
}

type Services = (
    Arc<dyn ErasedStatsService>,
    Arc<dyn ErasedHealthService>,
    Arc<dyn ErasedCountsService>,
);

fn services<C: StatsCache>(
    github: GitHub,
    tarball: Tarball,
    cache: C,
    settings: Settings,
    counts_settings: counts::services::Settings,
) -> Services {
    let stats = stats::services::Service::new(github, cache.clone(), SystemClock, settings);
    let counts = counts::services::Service::new(
        tarball,
        cache.clone(),
        stats.clone(),
        SystemClock,
        counts_settings,
    );
    (
        Arc::new(stats),
        Arc::new(health::services::Service::new(cache)),
        Arc::new(counts),
    )
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
