# Backlog

Not scheduled. Each has a reason it is not done yet.

## Refresh in the background

A snapshot is refreshed by the first request after it goes stale, and that request waits for GitHub. A task that refreshes ahead of expiry would keep every request fast. Not done because the portfolio reads at build time, where a slow first request costs nothing.

## Conditional requests to GitHub

GitHub answers `304 Not Modified` to a request carrying the `ETag` of the last response, and a 304 does not count against the rate limit. Not done because four requests per repository every fifteen minutes (the production `STATS_FRESH_SECS`; the default is six hours) is nowhere near the limit with a token.

## A status page

Uptime of the server and of steller, served by the server. This is the claim the project exists to be able to make, so it should be measured and not asserted.

## Make steller's address configurable

steller hardcodes `127.0.0.1:3000` and its cache paths. On heron's own box nothing else wants that port, so this blocks nothing. It would let the live tests run beside a steller already in use. The work is in steller.

## `TtlMap` in place of per-repository locks

The refresh locks are a map built once from the allowlist. If the allowlist ever becomes dynamic, the map needs eviction and this design stops fitting.

## Retry a 202 on commit activity inside the refresh

GitHub computes `/stats/commit_activity` in the background and answers `202` with no body until it is done, then forgets the result again after a while. The refresh asks once, keeps the last answer it had for any repository that comes back `202`, and moves on. That is enough in steady state: once a repository has answered `200` the weeks never go away. After a cache wipe (a steller restart) there is nothing to keep, and the first refresh gets whatever GitHub has warm, which on 2026-10-02 was three repositories of twelve. The chart on the portfolio read 2,822 commits for the year instead of 3,294 until the next refresh, fifteen minutes later.

A second request a few seconds after a `202`, inside the same refresh, would close that gap: asked by hand with an eight second pause, all three repositories that had been `202` for an hour answered `200` on the first retry, because the first ask is what starts the job. One retry per repository per refresh, bounded by a short sleep, in `activity()` in `outbound/github`. Not done because the gap only opens after a wipe, it closes on its own, and the retry makes every refresh a few seconds slower for the repositories GitHub has let go cold.
