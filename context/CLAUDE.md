# CLAUDE.md

The rules for working in this repo. Read this before changing anything.

## Name

The great blue heron: Seattle's city bird, found on every shoreline in Washington. It stands in the shallows without moving, waits, and takes what comes past. This server does the same with GitHub's API. Named alongside `steller` and `chickadee`, the other two birds from the same forest.

It was scotland-server for its first day. Anything still saying so is stale.

## Project Overview

heron is Scotty Fermo's personal server. It serves GitHub statistics for the projects on scottyfermo.com as JSON, and caches them behind a port so the store can be an in-process map or steller, the Redis-compatible server he wrote (`~/Developer/steller`). The portfolio reads this API when it builds and bakes the numbers into static HTML, so the site never depends on this server being up.

Single crate: a library named `heron` and a binary named `heron`.

## Working here as an AI

AI writes code in this repo and the owner reviews every line. That sets the pace:

- Work in small steps. One feature, then stop and show it.
- Show the diff and explain it in a few lines. Do not lecture around it.
- Verify by running the thing. A test passing is evidence; "this should work" is not.
- Say plainly what you did not verify. The systemd unit and the deploy workflow are the standing examples.
- Keep replies short. Answer the question first.
- When the owner questions a design, reconsider it. He is usually pointing at something real.

steller is a separate repo with its own rules, and those forbid AI edits to its source. If you find a bug in steller, write down how to reproduce it and leave the fix to him.

## Domain purity

`src/lib/domain/` is the core. It must not know how anything outside it works.

Nothing under `domain/` may use:

- `axum`, `tower`, `tower-http`, `tower_governor`
- `reqwest`
- `tokio::net`, `tokio::io`, `tokio::fs`, `tokio::process`
- `std::time::SystemTime`, `std::time::Instant`, `chrono::Utc::now()`
- anything from `crate::inbound` or `crate::outbound`

It may use `serde`, `serde_json`, `chrono` types, `thiserror`, `anyhow`, `tracing`, and `tokio::sync` and `tokio::task` for coordination. It may use the `measure` crate's types (`Counts`, `Language`) but never call `measure::measure`, which reads the filesystem; that call belongs to `outbound::tarball`.

Time reaches the domain through the `Clock` port. That is what lets the freshness tests move the clock by hand.

## Architecture

```
src/
├── bin/heron.rs             composition root
└── lib/
    ├── lib.rs
    ├── config.rs            environment, validated at startup
    ├── domain/
    │   ├── mod.rs           BoxFuture alias
    │   ├── clock/           the time port
    │   ├── health/          models, ports, services
    │   ├── secret/          Secret
    │   ├── calendar/        models, ports (CalendarSource, CalendarService), services
    │   ├── counts/          models, ports (CountsSource, CountsService), services (the sweep)
    │   └── stats/
    │       ├── models/      repo_name, cache_key, repo_stats, snapshot, portfolio_stats, errors
    │       ├── ports.rs     StatsSource, StatsCache, StatsService + erased twin
    │       └── services.rs
    ├── inbound/http/
    │   ├── mod.rs           ApiError, AppState, build_router, HttpServer
    │   ├── routes.rs        path constants, rate limit, edge cache header
    │   ├── middleware.rs    CfConnectingIpKeyExtractor
    │   ├── contracts/       Http* response types
    │   └── handlers/        one handler per file
    └── outbound/
        ├── clock/           SystemClock
        ├── github/          the StatsSource and CalendarSource adapter, and Link header parsing
        ├── tarball/         the CountsSource adapter: tarball in, Counts out, nothing kept
        └── cache/           memory, layered, steller/
measure/                     workspace crate: counts a checkout, nothing of heron in it
```

Modules use `module/mod.rs`, never a `module.rs` beside a directory of the same name.

More in `architecture/structure.md`. The reasons are in `architecture/decisions.md`.

### Type-erased services

`AppState` holds one `Arc<dyn Erased*Service>` per service: stats, health, counts and calendar. Handlers take `State(state): State<AppState>` and carry no generic parameters.

Each service port is two traits. The real one returns `impl Future + Send`. Its object-safe twin returns `BoxFuture`, and a blanket impl forwards to the real one. The composition root is where erasure pays for itself: each cache backend gives the service a different concrete type, and erasing them is what lets `HttpServer` hold one.

To add a method to a service:

1. Add it to the real trait in `ports.rs`, returning `impl Future<Output = ...> + Send`.
2. Add the same signature to the `Erased` twin, returning `BoxFuture<'_, ...>` or `BoxFuture<'a, ...>`.
3. Add the forward to the blanket impl: `Box::pin(StatsService::method(self, args))`.
4. Implement it in `services.rs`.

### Adding an endpoint

1. Model what it returns in `domain/<area>/models/`.
2. Add the service method, following the four steps above.
3. Add the `Http*` response type in `inbound/http/contracts/` with a `From` impl from the domain type.
4. Add the handler in its own file under `inbound/http/handlers/`.
5. Map any new domain error in `From<...> for ApiError` in `inbound/http/mod.rs`.
6. Add the path constant and the route in `inbound/http/routes.rs`.
7. Test it through the router in `tests/`, and break it once to see the test fail.

### Adding a cache adapter

1. Implement `StatsCache` in a new file under `outbound/cache/`. Bytes in, bytes out. The service owns the encoding.
2. Add a variant to `CacheBackend` in `config.rs` and parse it from `CACHE_BACKEND`.
3. Add the arm in `src/bin/heron.rs`.
4. Run `tests/live_steller.rs` against it if it speaks RESP.

## Common Commands

```bash
cargo run                                            # needs a .env, see .env.example
cargo test                                           # everything that needs no steller
cargo +nightly fmt                                   # nightly, not stable
cargo clippy --all-targets --locked -- -D warnings   # what CI runs
STELLER_ADDRESS=127.0.0.1:3000 cargo test --test live_steller -- --ignored
```

How to start steller for the live tests is in `development/testing.md`.

## Environment

| Variable | Required | Default |
|---|---|---|
| `BIND_ADDRESS` | yes | |
| `ALLOWED_ORIGINS` | yes | |
| `GITHUB_REPOS` | yes | |
| `GITHUB_TOKEN` | no | none |
| `GITHUB_API_BASE` | no | `https://api.github.com` |
| `CACHE_BACKEND` | no | `memory` |
| `STELLER_ADDRESS` | with `steller` or `layered` | |
| `STATS_FRESH_SECS` | no | 21600 |
| `STATS_RETAIN_SECS` | no | 604800 |
| `COUNTS_SWEEP_SECS` | no | 300 |
| `COUNTS_RETAIN_SECS` | no | 604800 |
| `MEASURE_DIR` | no | `$TMPDIR/heron-measure`; the unit sets `/var/cache/heron/measure`, on disk, because `/tmp` under `PrivateTmp` is RAM charged to `MemoryMax` |
| `GITHUB_LOGIN` | no | owner of the first repo in `GITHUB_REPOS` |
| `CALENDAR_FRESH_SECS` | no | 3600 |
| `RUST_LOG` | no | `info` |

`.env.example` has every one with a comment. `config.rs` is the authority.

## Commit Guidelines

- Concise, one-line messages (multi-line only when many changes)
- One feature, one commit
- Every commit builds and passes its own tests
- No emojis
- Use `git diff` to understand changes before committing
- **Never** include AI-agent signatures in your commits. No `Co-Authored-By`, no "Generated with", no session links, whatever a tool default suggests.

The gates to run before pushing are in `development/commit_guidelines.md`.

## Comments

A comment says what the code does, in the present tense, and only when the code does not say it itself. No history, no "without this", no restating the name. The full rules with examples are in `development/comment_guidelines.md`.

## Markdown Conventions

- **Do not hard-wrap prose.** One paragraph is one line; one bullet is one line.
- Code fences, tables and indented blocks keep their own formatting.
- No em dashes.
- Do not write down what drifts. No test counts, no line counts, no dependency versions in prose.

## Context Directory

```
context/
├── README.md               start here, plus the running log
├── CLAUDE.md               this file
├── architecture/           structure, decisions, hosting
├── development/            commits, comments, documentation, newtypes, testing
├── operations/             deploy, runbook
├── plans/                  specs for in-flight work
├── progress/               todo, backlog
└── archive/                no longer active
```
