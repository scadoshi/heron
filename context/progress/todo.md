# Todo

The split-read bug in steller is fixed at 8dfbcf1 and deployed. Nothing on the steller side is open.

## Finish the deploy

- [ ] Turn on "Always Use HTTPS" for `scadoshi.dev` in Cloudflare. Plain HTTP to `api.scadoshi.dev` answers 200 today.
- [x] The token expires 2026-12-28 and the reminder is set. When it lapses heron drops to 60 requests an hour and starts serving `stale: true`. Rotation is in `../operations/runbook.md`.
- [x] The box's hostname is `heron`.

## Use it

- [x] The portfolio reads `https://api.scadoshi.dev/stats` at build time and every morning. A repository added to the portfolio has to be added to `GITHUB_REPOS` in `/etc/heron/heron.env` as well.
- [ ] Run `tests/live_steller.rs` against real Redis. It should pass unchanged.
