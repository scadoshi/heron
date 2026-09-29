# heron

My personal server. Right now it does one thing: serves GitHub stats for the projects on [scottyfermo.com](https://scottyfermo.com) as JSON. It is live at [api.scadoshi.dev](https://api.scadoshi.dev/stats).

The stats are cached, and the cache can run on [steller](https://github.com/scadoshi/steller), the Redis-compatible server I wrote. For this load that is overkill. One small JSON value per repo fits in a `HashMap`, and the server runs on exactly that if you ask it to. I run it on steller because benchmarks never found steller's best bugs and running it did. On the first day it found another: steller rejects any command over 1024 bytes. The write-up is in [`context/architecture/decisions.md`](context/architecture/decisions.md).

## Endpoints

| | |
|---|---|
| `GET /stats` | every configured repo, with totals |
| `GET /stats/{owner}/{name}` | one repo |
| `GET /health` | is the server up |
| `GET /health/cache` | which cache backend, and does it answer |

```json
{
  "repo": "scadoshi/steller",
  "commits": 71,
  "stars": 2,
  "default_branch": "main",
  "pushed_at": "2026-09-29T16:04:14Z",
  "languages": [{ "name": "Rust", "bytes": 207124 }],
  "additions": null,
  "deletions": null,
  "fetched_at": "2026-09-29T17:18:43Z",
  "stale": false
}
```

`additions` and `deletions` are `null` while GitHub is still computing them. `stale` is `true` when GitHub could not be reached and you are getting the last numbers it gave.

Only repos on the allowlist are served, and a private one is refused even if it is listed.

## Run it

```sh
cp .env.example .env
cargo run
curl localhost:3100/stats
```

Without a token GitHub allows 60 requests an hour, which is plenty to try it.

## Cache backends

Set `CACHE_BACKEND`:

- `memory` is an in-process map. The default.
- `steller` is steller over a RESP client written here, about 300 lines with no handshake. It works against Redis too.
- `layered` is steller with the map behind it. Kill steller and requests keep answering.

steller is reached on loopback only. It has no AUTH and no TLS, so the server refuses to start if you point it anywhere else.

## Tests

```sh
cargo test
```

The tests against a real steller are ignored by default. Start one, then:

```sh
STELLER_ADDRESS=127.0.0.1:3000 cargo test --test live_steller -- --ignored
```

## Layout

Ports and adapters in one crate. `domain/` knows nothing about HTTP, GitHub, RESP or the clock. How it fits together, why, and how to work in it are under [`context/`](context/), starting at [`context/README.md`](context/README.md).

## Name

The great blue heron, Seattle's city bird. It stands in the shallows, waits, and takes what comes past. [steller](https://github.com/scadoshi/steller) and [chickadee](https://github.com/scadoshi/chickadee) are the other two birds.

What is left is in [`context/progress/todo.md`](context/progress/todo.md).
