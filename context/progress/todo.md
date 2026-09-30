# Todo

## Finish the deploy

- [ ] Turn on "Always Use HTTPS" for `scadoshi.dev` in Cloudflare. Plain HTTP to `api.scadoshi.dev` answers 200 today.
- [x] The token expires 2026-12-28 and the reminder is set. When it lapses heron drops to 60 requests an hour and starts serving `stale: true`. Rotation is in `../operations/runbook.md`.
- [x] The box's hostname is `heron`.

## Fix in steller

This is in steller, which the owner writes by hand.

- [ ] **steller rejects a command that arrives in more than one read.** `Frame::parse_bulk_string` returns `MissingTerminator` for a payload that has not all arrived, where `Incomplete` would make the session read more. Reproduction: `tests/live_steller_large.rs`. Details: `../architecture/decisions.md`.
- [ ] After the fix, deploy steller from the Actions tab and move the test in `tests/live_steller_large.rs` into `tests/live_steller.rs`.

## Use it

- [x] The portfolio reads `https://api.scadoshi.dev/stats` at build time and every morning. A repository added to the portfolio has to be added to `GITHUB_REPOS` in `/etc/heron/heron.env` as well.
- [ ] Run `tests/live_steller.rs` against real Redis. It should pass unchanged.
