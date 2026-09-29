# Todo

## Finish the deploy

- [ ] Turn on "Always Use HTTPS" for `scadoshi.dev` in Cloudflare. Plain HTTP to `api.scadoshi.dev` answers 200 today.
- [ ] Put the token's expiry, 2026-12-28, in a calendar. When it lapses heron drops to 60 requests an hour and starts serving `stale: true`. Rotation is in `../operations/runbook.md`.
- [ ] Set the box's hostname to `heron`. It is still `ubuntu-4gb-nbg1-1`.
- [ ] Run `.github/workflows/deploy.yml` once by hand, then decide whether to deploy on push. Until then a deploy is `git pull`, build, `install`, restart, by hand on the box.

## Turn on steller

- [ ] **Fix steller's 1024-byte command limit.** `Frame::parse_bulk_string` returns `MissingTerminator` for a payload that has not all arrived, where `Incomplete` would make the session read more. Reproduction: `tests/live_steller_large.rs`. Details: `../architecture/decisions.md`. This is in steller, which the owner writes by hand. It does not block `layered`.
- [ ] Install steller on the box under its own unit with `MemoryMax=` set.
- [ ] Switch heron to `CACHE_BACKEND=layered`.
- [ ] After the fix, move the test in `tests/live_steller_large.rs` into `tests/live_steller.rs`.

## Use it

- [ ] Have the portfolio read `https://api.scadoshi.dev/stats` at build time. That change is in the portfolio repo.
- [ ] Run `tests/live_steller.rs` against real Redis. It should pass unchanged.
