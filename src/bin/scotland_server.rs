//! Composition root. Wires adapters into services and starts the HTTP server.

use scotland::{
    config::{CacheBackend, Config},
    domain::{
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
    },
};
use std::{process::ExitCode, sync::Arc};
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
                eprintln!("scotland-server failed: {error:#}");
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
        "scotland-server v{} serving {} repositories, cache {:?}, github token {}",
        env!("CARGO_PKG_VERSION"),
        config.github_repos.len(),
        config.cache_backend,
        if config.github_token.is_some() {
            "set"
        } else {
            "not set"
        },
    );

    let github = GitHub::new(&config.github_api_base, config.github_token)?;
    let settings = Settings {
        repos: config.github_repos,
        fresh: config.stats_fresh,
        retain: config.stats_retain,
    };

    // Each arm builds services over a different cache type. Erasing them here is
    // what lets everything past this point hold one type.
    let (stats_service, health_service) = match config.cache_backend {
        CacheBackend::Memory => services(github, MemoryCache::new(), settings),
        CacheBackend::Steller(address) => services(github, StellerCache::new(address)?, settings),
        CacheBackend::Layered(address) => services(
            github,
            LayeredCache::new(StellerCache::new(address)?, MemoryCache::new()),
            settings,
        ),
    };

    let server = HttpServer::new(
        stats_service,
        health_service,
        HttpServerConfig {
            bind_address: &config.bind_address,
            allowed_origins: config.allowed_origins,
        },
    )
    .await?;
    server.run().await
}

fn services<C: StatsCache>(
    github: GitHub,
    cache: C,
    settings: Settings,
) -> (Arc<dyn ErasedStatsService>, Arc<dyn ErasedHealthService>) {
    (
        Arc::new(stats::services::Service::new(
            github,
            cache.clone(),
            SystemClock,
            settings,
        )),
        Arc::new(health::services::Service::new(cache)),
    )
}
