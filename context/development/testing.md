# Testing

## Two rules

**A test that has never failed proves nothing.** For every test that guards a behavior, break the behavior once, watch the test fail, and restore it. This is how a weak test is found. It is also how the redundant check for GitHub's 202 was found: removing it changed no test, because the parse failure behind it lands on the same result.

**Prove readiness by a served request, never by a successful connect.** A listener accepts connections into its backlog as soon as it is bound, including while the process is shutting down. `wait_until_serving` in `tests/binary.rs` asks for `/health` and waits for a 200. The CI job waits for steller to answer `PING`.

## Where tests live

| Kind | Where | Runs with |
|---|---|---|
| Unit | `#[cfg(test)] mod tests` beside the code | `cargo test` |
| GitHub adapter | `src/lib/outbound/github/tests.rs` | `cargo test` |
| Router | `tests/health.rs`, `tests/stats.rs` | `cargo test` |
| Binary | `tests/binary.rs` | `cargo test` |
| Live | `tests/live_steller.rs` | a running steller, and `-- --ignored` |
| Reproduction | `tests/live_steller_large.rs` | a running steller, and `-- --ignored` |

## Names

A test name is a sentence about behavior: `a_source_failure_serves_the_stale_snapshot_and_marks_it`. When it fails, the name alone should say what broke.

## Fakes

Unit tests use the fakes in `src/lib/test_support.rs`:

- `FakeClock` moves only when told to, with `advance(secs)`.
- `FakeSource` counts its calls, fails on request, and can take time to answer so that concurrent callers pile up behind it.
- `FakeCache` fails reads, writes or pings on request, and records the retain window it was last given.

Tests under `tests/` cannot see `#[cfg(test)]` items in the library, so `tests/common/mod.rs` has its own: `TestClock`, `StubSource`, `DeadCache`, and `TestApp`, which builds the real router over the real service.

The GitHub adapter is tested against a fake of GitHub's API served by axum on `127.0.0.1:0`. The steller adapter is tested against a small RESP server in its own test module. Both use real sockets, since timeouts, partial reads and reconnection are the point.

## Running the live tests

steller binds `127.0.0.1:3000`, which is not configurable, and writes `cache/aof` and `cache/snapshot` relative to where it is started. Make sure nothing else holds the port.

```bash
cd ~/Developer/steller && cargo build --release
mkdir -p /tmp/steller-live/cache && cd /tmp/steller-live
~/Developer/steller/target/release/steller < /dev/null &

cd ~/Developer/scotland-server
STELLER_ADDRESS=127.0.0.1:3000 cargo test --test live_steller -- --ignored
```

Stop steller with `kill -TERM`. It takes a final snapshot and exits.

Every live test uses a repository name no other run has used, so runs do not read each other's keys.

`tests/live_steller_large.rs` is kept apart and is not in CI. It fails against steller at 8628070 and says why at the top of the file. Run it after steller is fixed, and move its test into `live_steller.rs` when it passes.

## Tests that bind a fixed port

None of the tests run by `cargo test` do. Every listener binds port 0 and reads back what it was given, so the suite runs in parallel and beside a running server.

The live tests use whatever `STELLER_ADDRESS` names.

## What is not tested

- SIGINT. It goes through the same `shutdown_signal` as SIGTERM, which is tested.
- The systemd unit and the deploy workflow. Neither can run on a laptop.
- Real GitHub, in CI. The adapter is tested against a fake. Agreement with the real API was checked by hand, and the log in `../README.md` records what was compared.
