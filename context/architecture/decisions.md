# Architecture Decisions

Each entry says what was decided, why, what it costs, and when to look again.

---

## steller as the cache: dogfooding, not need

**Decided: 2026-09-29.**

The cache can run on steller, the Redis-compatible server the owner wrote. For this load that is overkill. The server caches one small JSON value per repository, and a `HashMap` behind a lock does that with no network hop, no second process, and nothing else that can be down.

**Why do it anyway:** steller's best bugs have been found by running it, not by testing it. Its suite and its benchmarks against Redis were green while a TTL came back one second short and a shutdown deadlocked. A cache that something depends on finds a different class of bug than a demo does. This project is that something.

**What it proves and what it does not:** it proves steller can serve a real client for a long time, and it will keep finding bugs like the one below. It does not prove the project needed Redis, and nothing here should claim so.

**What it costs:** a second process to deploy and watch. The memory fallback is what keeps that from being a second thing that can take the server down.

**It paid off on the first day.** See the next entry.

---

## steller's 1024-byte command limit, and what the client does about it

**Found: 2026-09-29, at steller 8628070.**

steller answers `ERR missing crlf terminator` to any command longer than 1024 bytes. Its session reads 1024 bytes at a time and `Frame::parse_bulk_string` reports a payload that has not all arrived as `MissingTerminator`, a hard error, where `Incomplete` would make the session read more. The session clears its buffer, the rest of the payload arrives, and steller parses it as new commands and answers each with an error.

Reproduce it with `redis-cli`:

```sh
head -c 1000 /dev/zero | tr '\0' 'x' | redis-cli -p 3000 -x SET k
```

or with `tests/live_steller_large.rs`.

**Decided here:** the client closes its connection after any error reply. After a rejection the connection holds replies to commands that were never sent, and keeping it would hand one of them to the next command as its answer. Against Redis this costs a reconnect after an error that would have been harmless. That is cheap, and error replies are rare.

**Not decided here:** the client does not refuse large values or split them. That would write steller's bug into this codebase, and the adapter also has to work against Redis.

**Exposure:** the largest snapshot measured is 517 bytes. Each language adds about 35. A repository with around twenty languages would not cache under `CACHE_BACKEND=steller`, and every request for it would go to GitHub. Under `layered` the memory layer holds it and nothing is lost.

**Revisit when:** steller is fixed. Then `tests/live_steller_large.rs` passes and can join `tests/live_steller.rs`.

---

## Hexagonal, in one crate

**Decided: 2026-09-29.**

Ports and adapters, with `domain` importing from neither side. One crate, not a workspace: there is one deployable and no second consumer of the domain types.

**Why:** the cache being swappable is the premise of the project, and the clock and GitHub being swappable is what makes the service testable without a network or a sleep.

**Revisit when:** a second binary or a client crate wants the domain types. Then the domain moves to its own crate, as zwipe-core did.

---

## Type-erased services: `AppState` holds `Arc<dyn ErasedXService>`

**Decided: 2026-09-29, following zerver.**

Each service port is two traits: the real one with `-> impl Future + Send` methods, and an object-safe twin returning `BoxFuture`, with a blanket impl forwarding one to the other. A trait with `impl Future` in return position is not object-safe, because every implementor returns a future of a different size and a vtable needs one signature.

**Why here in particular:** `Service<S, C, K>` is generic over its cache. `memory`, `steller` and `layered` give three different concrete types, chosen at startup from an environment variable. Without erasure `AppState` and `HttpServer` would be generic over the cache, and the choice would have to be made at compile time. The composition root erases each arm to the same `Arc<dyn ErasedStatsService>`.

**Cost:** one allocation and one vtable call per service call, next to a network round trip. And one extra signature per new service method.

**Revisit when:** writing the twins by hand becomes a tax. The `dynosaur` crate generates them.

---

## A hand-written RESP client

**Decided: 2026-09-29.**

The steller adapter speaks RESP through a client written here (`outbound/cache/steller/`), not the `redis` crate.

**Why:** three commands are needed, `PING`, `GET` and `SET ... PX`. The `redis` crate opens with a handshake, `HELLO` or `CLIENT SETINFO` depending on version, and steller implements neither. A client that sends only what steller parses has no such surprises. It is also the other half of the protocol steller implements, which suits the project.

**Cost:** the codec is this repo's to maintain, and it reads five reply types and no more. Arrays are refused.

**Revisit when:** the server needs pipelining, pub/sub, or a pool.

---

## The cache never fails a request

**Decided: 2026-09-29.**

A cache read error is a miss. A cache write error is logged and the value is returned anyway. `StatsError` has no variant for a cache failure, so one cannot reach a caller by construction.

**Why:** the cache is an optimization. A server that answers 500 because its optimization is down has made itself less available by adding it.

**Cost:** a dead cache is quiet. `/health/cache` and the warn-level log are the only signs, and someone has to look.

---

## Freshness lives in the payload

**Decided: 2026-09-29.**

A `Snapshot` carries `fetched_at` and `fresh_until`. The cache's own expiry is a longer retention window.

**Why:** if the cache's TTL were the freshness window, a stale value would already be gone at the moment it is needed, which is when GitHub is down. Holding the bytes longer than they are fresh is what allows `stale: true`.

**Cost:** two windows to configure and one rule between them, which config enforces.

---

## Layered cache copies primary hits into the secondary

**Decided: 2026-09-29, after the first end-to-end run.**

`LayeredCache` writes to both layers, and a value read from the primary is copied into the secondary.

**Why:** without the copy, a server restarted while steller held the snapshots would serve them from steller and leave the memory layer empty. steller going down then leaves nothing to fall back on. The unit tests did not catch this, because each wrote through the layered cache before reading from it. The end-to-end run did.

**Cost:** the primary does not say how long its copy has left, so the secondary keeps the copy for the full retain window again. Freshness is in the payload, so a copy kept too long is served as stale and never as fresh.

---

## Private repositories are refused

**Decided: 2026-09-29.**

If GitHub reports a configured repository as private, the adapter returns `PrivateRepo` before requesting anything else, and the service never caches or serves it. This holds even when the token could read it.

**Why:** the allowlist is a line in a config file. A typo, or a repository made private after it was listed, must not publish anything.

**And:** `PrivateRepo` and `UnknownRepo` answer with the same status and the same body. A different message for a private repository would confirm that it exists.

---

## Additions and deletions are not lines written

**Decided: 2026-09-29.**

The API reports `additions` and `deletions` from GitHub's contributor statistics, and the portfolio should not present either as lines of code written.

**Why:** they count every rewrite again, along with lockfile churn, vendored code and generated files. steller's net was 7,560 on the day its `src/` held 5,901 lines.

**Also:** GitHub computes these in the background and answers 202 until it is done. The fields are `null` then, and the next refresh asks again. Nothing waits.

---

## Logs go to stdout only

**Decided: 2026-09-29. Differs from zerver, which writes rolling files.**

**Why:** journald keeps stdout already. With no log directory the systemd unit can run under `ProtectSystem=strict` with no writable path at all.

**Cost:** retention is journald's, configured on the box and not in this repo.

---

## `main` returns `ExitCode`

**Decided: 2026-09-29.**

Any startup failure exits 1.

**Why:** systemd's `Restart=on-failure` reads the exit code. A server that prints an error and exits 0 is left stopped. steller had exactly this until 165dd03.

---

## No authentication

**Decided: 2026-09-29.**

Every endpoint is public and read-only, and everything served is already public on GitHub.

**Revisit when:** an endpoint writes, or serves anything not already public.
