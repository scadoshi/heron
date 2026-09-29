# Todo

## Deploy heron on `memory`

Nothing blocks this.

- [ ] Create the GitHub token: fine-grained, read-only, public repositories.
- [ ] Choose the public hostname.
- [ ] Create the Hetzner box, deny inbound traffic, install `cloudflared`, and route the hostname to `http://127.0.0.1:3100`.
- [ ] Install heron following `../operations/deploy.md`.
- [ ] Run `systemd-analyze verify` on `deploy/heron.service`. It has never been loaded, and it is hardened hard enough that a directive or two may need loosening.
- [ ] Correct `../operations/deploy.md` against what actually happened.

## Turn on steller

- [ ] **Fix steller's 1024-byte command limit.** `Frame::parse_bulk_string` returns `MissingTerminator` for a payload that has not all arrived, where `Incomplete` would make the session read more. Reproduction: `tests/live_steller_large.rs`. Details: `../architecture/decisions.md`. This is in steller, which the owner writes by hand. It does not block `layered`.
- [ ] Install steller under its own unit with `MemoryMax=` set.
- [ ] Switch heron to `CACHE_BACKEND=layered`.
- [ ] After the fix, move the test in `tests/live_steller_large.rs` into `tests/live_steller.rs`.

## After it is up

- [ ] Have the portfolio read `GET /stats` at build time. That change is in the portfolio repo.
- [ ] Run `.github/workflows/deploy.yml` once by hand, then decide whether to deploy on push.
- [ ] Run `tests/live_steller.rs` against real Redis. It should pass unchanged.
