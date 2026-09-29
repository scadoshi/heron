# Context: Start Here

Orientation for AI assistants and returning contributors. Each subdirectory owns one concern.

## Directory map

| Directory | What's in it |
|-----------|--------------|
| [`architecture/`](architecture/) | Why it is built this way: structure, decisions, hosting |
| [`development/`](development/) | How to write code here: commits, comments, documentation, newtypes, testing |
| [`operations/`](operations/) | How to deploy it and what to do when it misbehaves |
| [`plans/`](plans/) | Specs for work in flight |
| [`progress/`](progress/) | What is open: `todo.md`, `backlog.md` |
| [`archive/`](archive/) | No longer active; kept for history |

Plus [`CLAUDE.md`](CLAUDE.md), the rules for working in the repo.

## Where things stand

The running log, newest first. Update it when something ships. [`progress/todo.md`](progress/todo.md) holds what is still open.

## 2026-09-29: built, and steller's first production bug

The whole server went up in one day: domain, three cache adapters, the GitHub adapter, the HTTP layer, config, the binary, CI.

### What runs

`GET /stats` and `GET /stats/{owner}/{name}` serve commit counts, stars, languages and line churn for an allowlist of repositories, cached behind a port. Three adapters sit behind that port: an in-process map, steller over a hand-written RESP client, and the two layered. Nothing is deployed. The server has only run on a laptop.

### What was verified, and how

Run against real GitHub with the release binary, commit counts matched `gh api` for all three repositories tried: steller, chickadee and zwipe. A second request for the same repository reached GitHub zero times.

Run with `CACHE_BACKEND=layered` against a real steller, snapshots survived a restart of the server: the restarted process served them from steller with GitHub pointed at a dead port. With steller killed mid-run, requests kept answering 200 from the memory layer, `steller cache is down` was logged once, and `/health/cache` reported `unreachable`. When steller came back the log said so once and health returned to `healthy`.

SIGTERM exits 0. A missing variable, a steller address off loopback, and an address already in use each exit 1 and name the problem.

Every test that guards a behavior was checked by breaking the behavior and watching the test fail. One mutation survived, and it is an equivalent one: removing the explicit check for GitHub's 202 changes nothing, because an empty body fails to parse and lands on the same `None`.

### steller rejects any command over 1024 bytes

This is the first bug found by running steller as something other than a demo. A `SET` whose whole command is longer than 1024 bytes is answered `ERR missing crlf terminator`. With `redis-cli` it starts at a value of exactly 1000 bytes.

steller's session reads 1024 bytes at a time, and `Frame::parse_bulk_string` returns `MissingTerminator` when the payload has not all arrived. That is a hard error, where `Incomplete` would have made the session read more. The session then clears its buffer, and the rest of the payload arrives and is parsed as new commands, each answered with another error.

It does not stop this server working. The largest snapshot measured was zwipe's at 517 bytes. But a repository with around twenty languages would cross the line, and under `CACHE_BACKEND=steller` its stats would never cache. The reproduction is `tests/live_steller_large.rs`. The fix belongs to steller and is on the list in `progress/todo.md`.

### What the bug changed here

The second half of it mattered more than the first. After a rejected command the connection holds error replies nobody asked for, and the next command would read one of them as its own answer. So the client now closes the connection after any error reply. With that defense removed, `the_command_after_a_large_value_is_answered_correctly` fails against the real steller, which is how it is known to be doing something.

### What the end-to-end run caught that the tests had not

`LayeredCache` wrote to both layers and read from the first that answered. After a restart the memory layer was empty, values came from steller, and nothing copied them across. Killing steller then left nothing to fall back on. Every unit test passed, because each one wrote through the layered cache before reading from it. A value read from the primary is now copied into the secondary.

### The first CI run failed on a lint this machine could not see

CI had Rust 1.98 and the laptop had 1.97. Between them clippy gained `unused_async_trait_impl`, which is in `pedantic`, and it fired on every adapter method that is `async` to satisfy a port and awaits nothing. Those methods now do their work when called and return `std::future::ready`. The other three jobs passed on the first run, the live tests against steller among them. `development/commit_guidelines.md` says how to reproduce a lint from a newer release.

### Renamed to heron

The server was scotland-server until the end of its first day. The repository, the crate, the library, the binary, the systemd unit and the cache key prefix all changed with it, so a key written under `scotland:stats:v1:` is not read. Nothing was deployed, so nothing was lost.

### It gets its own box

The first plan was to share zerver's box. The owner changed it to a Hetzner box of its own, which removes the port 3000 collision between steller and zerver and keeps an unbounded store away from Zwipe's Postgres. The unit now runs as a `heron` system user from `/usr/local/bin/heron` with its environment in `/etc/heron/heron.env`.

### Not verified

- `deploy/heron.service` has never been loaded by systemd.
- `.github/workflows/deploy.yml` has never run, and there is no server for it to run on.
- The live tests have not been run against real Redis, only steller.
