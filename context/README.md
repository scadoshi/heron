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

## 2026-10-08: SSH over the tailnet only, as scadoshi

The public SSH port took about 32,000 failed logins a day. None got in (root's password is locked and only keys were accepted), but the tailnet already reached the box, so the port had no reason to be open. ufw now allows SSH on `tailscale0` only, and the box answers nothing on its public address.

Root no longer logs in. The owner logs in as `scadoshi`, which has a sudo password, and scotland-server's probe reads the journals as that user through `systemd-journal`, so a compromised scotland no longer means root here. The SSH settings are one file, `sshd_config.d/10-hardening.conf`, shared with zerver and scotland-server; `operations/deploy.md` has it.

A push that only touches Markdown, `context/` or `LICENSE` no longer deploys.

## 2026-10-07: killed at its own memory limit

UptimeRobot and scotland's probe each caught a `/health` timeout, at 07:20 and 07:39 UTC. The journal showed why: `heron.service: Failed with result 'oom-kill'`, at the unit's `MemoryMax=128M`, and systemd had it back in five seconds, so nothing else noticed. It had been happening several times a day since the counts sweep shipped. The box itself had 3 GB free.

Three things stacked up. A measurement held the whole tarball in memory twice (`bytes()` then `to_vec()`), and portfolio's is 36 MB and zwipe's 24 MB. It unpacked into `/tmp`, which `PrivateTmp=true` makes a tmpfs: RAM, charged to the unit's limit and not reclaimable. And glibc's malloc kept the freed buffers in its arenas, so the process idled at 66 to 100 MB with nothing live and every measurement started near the cap.

The tarball now streams to a file a chunk at a time, the unit's `CacheDirectory=heron` puts `MEASURE_DIR` on disk at `/var/cache/heron/measure`, and mimalloc is the global allocator. heron idles near 20 MB. A push to portfolio made the first big measurement under the fix: it succeeded with no restart, and the cgroup peaked at 96M, mostly page cache from the files on disk, which the kernel reclaims instead of killing. The limits then went to `MemoryHigh=192M` and `MemoryMax=256M`, twice that peak. scotland's probe now alerts when heron or steller logs `Failed with result`, read from the journal over SSH, because a five-second restart slips between health checks.

## 2026-10-02: a second ask for statistics GitHub is still computing

A restart of steller on the box showed what the carry-forward cannot cover: with nothing cached, a refresh got weeks for three repositories of twelve, and the next three refreshes got the same three. GitHub computes `/stats/commit_activity` and `/stats/contributors` in a few seconds once asked, then lets the result go cold again inside the fifteen minutes between refreshes for any repository nobody is pushing to, so every refresh landed on a fresh 202 for the nine quiet ones. The refresh now waits three seconds after a 202 and asks once more, for both statistics; the test fake answers 202 then 200 to prove it. Warming the nine by hand right before a refresh filled prod to all twelve, 3,294 commits for the year.

## 2026-10-01: weekly commits

Each repository's stats carry `weekly_commits`, the last 52 weeks from `GET /repos/{owner}/{name}/stats/commit_activity`, a fifth request per refresh with the same 202-while-computing handling as the contributor statistics. `totals.weekly_commits` sums them by week across the repositories that have them, which is what the portfolio draws as commits over time. The field is serde-defaulted, so payloads cached before it decode as `None` and fill in at the next refresh; no schema bump. GitHub also forgets the computed activity after a while and answers 202 again, which emptied the weeks on the next refresh: a refresh now keeps the cached weeks (and additions and deletions, same mechanism) when the new answer lacks them, so coverage only grows.

## 2026-10-01: the contribution calendar

`/stats` carries `calendar`: the account's last year of contributions, 369 days with a count and GitHub's own 0 to 4 shade, from one GraphQL query (`contributionsCollection.contributionCalendar`) with the token heron already has. Its own domain beside stats and counts: fresh for `CALENDAR_FRESH_SECS` (an hour), kept seven days, served stale when GitHub fails, `null` without a token. `GITHUB_LOGIN` names the account and defaults to the first allowlisted repository's owner. The portfolio draws it as the heatmap under the hero. The first query from the box answered 5,099 contributions.

## 2026-10-01: lines, tests and lints are measured here now

`/stats` carries `counts` per repository: lines of source, test attributes and clippy lints set to warn or deny, plus `measured_at`. They are not on GitHub's API, so heron measures them itself: a sweep downloads a tarball of the default branch (`GET /repos/{owner}/{name}/tarball`, one request, no git), unpacks it under `MEASURE_DIR`, counts it with the new `measure` workspace crate, and deletes it. The sweep runs at startup and every `COUNTS_SWEEP_SECS` (300), and measures a repository again only when its `pushed_at` from the stats cache is newer than the measurement, so a repository nobody touches is never downloaded twice. Measurements are kept `COUNTS_RETAIN_SECS` (seven days) under `heron:counts:v1:owner/name`, and a failed measurement leaves the old one standing. The first local sweep measured five repositories including zwipe in a few seconds without a token.

The portfolio used to carry these numbers in a `counts.json` refreshed by a test run by hand against local clones, which lagged whenever another repository moved. It now reads them live from here, with the `stats.json` its deploy bakes from this answer as the one fallback for every number; that file and its test are gone. For rustmas and sharpmas the measured branch is `main`, the template, which is the owner's choice.

## 2026-09-30: CORS on every response, because the edge ignores Vary

The CORS header was only sent when a request carried `Origin`. Cloudflare caches `/stats` by URL and ignores `Vary`, so a copy filled by a request without `Origin` (a curl, the portfolio's deploy workflow) carried no header, and every browser served that copy failed CORS and fell back to the baked numbers until it expired. Seen as "live on desktop, as of on the phone". With a single allowed origin the header is now constant on every response; more than one still mirrors.


scottyfermo.com asks `GET /stats` from the browser after each page loads and swaps the answer in, falling back to the numbers baked in at its build. Its hero says "live" or "as of <day>" depending on which it got. A Cloudflare cache rule holds `/stats*` for the five minutes `Cache-Control` asks for, so page loads do not reach the box.

## 2026-09-30: deploys through a runner, and the steller bug is fixed

A GitHub Actions runner on the box now deploys both services. A push to heron's `main` deploys heron after the tests and lints pass on GitHub's runners. steller is deployed from the Actions tab, which clones its `main`, runs its tests, builds, installs, restarts, and asks it for `PONG`, keeping the previous binary to put back if it does not answer. Every path ran for real: a heron push, a steller deploy, a steller deploy skipped because nothing had changed, and a steller deploy of a new commit.

The runner's user can do eight things with `sudo` and nothing else. Every public repo of the owner's now requires approval before a fork's pull request runs any workflow, since a fork's workflow can name a self-hosted runner.

steller's split-read bug is fixed at 8dfbcf1: `parse_bulk_string` answers `Incomplete` while a payload is still arriving. On the production box a `PING` sent in three pieces answers `+PONG` and a 256 KB `SET` answers `+OK`. The reproduction moved from its own file into `tests/live_steller.rs`.

## 2026-09-29: steller in production

steller runs on the box under its own unit, capped at 256M, and heron runs on it with `CACHE_BACKEND=layered`. `deploy/steller.service` had never been loaded before either, and it passed `systemd-analyze verify` and started on the first try.

Checked on the box: a repository's `fetched_at` was the same before and after a restart of heron, so the snapshot came from steller and not from GitHub. With steller stopped, `/stats` answered 200 and `/health/cache` said `unreachable`. With it started again, health returned to `healthy`. The log recorded the outage once and the recovery once.

### The bug is wider than it was first written up

The first `PING` sent to steller on the box came back `-ERR missing crlf terminator`. The second came back `+PONG`. bash's `printf` had written the command a line at a time, and steller read the first part before the rest arrived.

So the bug found earlier in the day is not a limit of 1024 bytes. steller rejects any command that reaches it in more than one read, and 1024 bytes is only the size past which that always happens. `architecture/decisions.md` is corrected. The cause is the same and so is the fix.

heron sends each command with one write over loopback, closes the connection after a rejection, and has the memory layer behind it, so it is not affected in practice. Anything that talks to steller over a real network would be.

## 2026-09-29: deployed

heron is live at `https://api.scadoshi.dev`, serving all eleven repositories on the portfolio from `CACHE_BACKEND=memory`. It runs on a Hetzner CX23 of its own, behind a Cloudflare Tunnel, with every inbound port but SSH closed.

`deploy/heron.service` was the part that could not be tested on a laptop. It passed `systemd-analyze verify` and the server started under it on the first try, hardening and all. From inside that sandbox it resolved GitHub's name and reached it over TLS with the token.

Checked from outside: the commit count for steller matches `gh api`, stats responses carry the edge cache header, a repository that is not configured and a path traversal attempt both answer 404, CORS allows `https://scottyfermo.com` and no other origin, and a request to the box's own address on port 3100 gets no answer.

Still open when this was written: steller was not on the box. Plain HTTP to the hostname answers 200 where it could redirect, which "Always Use HTTPS" in Cloudflare would close. `.github/workflows/deploy.yml` has never run, so a deploy is still done by hand.

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

It does not stop this server working. The largest snapshot measured was zwipe's at 517 bytes. But a repository with around twenty languages would cross the line, and under `CACHE_BACKEND=steller` its stats would never cache. The reproduction was `tests/live_steller_large.rs`, since folded into `tests/live_steller.rs`. Fixed the next day; see the entry above.

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

- `deploy/heron.service` had never been loaded by systemd. It has since; see the entry above.
- `.github/workflows/deploy.yml` has never run.
- The live tests have not been run against real Redis, only steller.
