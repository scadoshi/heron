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
