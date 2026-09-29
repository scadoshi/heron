# Todo

## Blocking `steller` and `layered` in production

These are in steller, which the owner writes by hand.

- [ ] **Make steller's bind address configurable.** It hardcodes `127.0.0.1:3000`, and zerver binds `0.0.0.0:3000` on the production box.
- [ ] **Fix steller's 1024-byte command limit.** `Frame::parse_bulk_string` returns `MissingTerminator` for a payload that has not all arrived, where `Incomplete` would make the session read more. Reproduction: `tests/live_steller_large.rs`. Details: `../architecture/decisions.md`.

## Before the first deploy

- [ ] Create the GitHub token: fine-grained, read-only, public repositories.
- [ ] Choose the public hostname and add the Cloudflare Tunnel route.
- [ ] Load `deploy/heron.service` on the box and run `systemd-analyze verify` on it. It has never been loaded.
- [ ] Run steller under its own unit with `MemoryMax=` set. It has no memory bound and no eviction.
- [ ] Run `.github/workflows/deploy.yml` once by hand, then decide whether to deploy on push.
- [ ] Correct `../operations/deploy.md` against what actually happened.

## After it is up

- [ ] Have the portfolio read `GET /stats` at build time. That change is in the portfolio repo.
- [ ] Run `tests/live_steller.rs` against real Redis. It should pass unchanged.
