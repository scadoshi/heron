# heron

My personal server. Right now it does one thing: serves GitHub stats for the projects on [scottyfermo.com](https://scottyfermo.com) as JSON. It is live at [api.scadoshi.dev](https://api.scadoshi.dev/stats).

The stats are cached, and the cache can run on [steller](https://github.com/scadoshi/steller), the Redis-compatible server I wrote. For this load that is overkill. One small JSON value per repo fits in a `HashMap`, and the server runs on exactly that if you ask it to. I run it on steller because benchmarks never found steller's best bugs and running it did. On the first day it found another: steller rejects any command that does not arrive in a single read. The write-up is in [`context/architecture/decisions.md`](context/architecture/decisions.md).

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
  "stale": false,
  "counts": {
    "language": "Rust",
    "lines": 6102,
    "tests": 242,
    "clippy_lints": 14,
    "measured_at": "2026-10-01T14:02:11Z"
  }
}
```

`additions` and `deletions` are `null` while GitHub is still computing them. `stale` is `true` when GitHub could not be reached and you are getting the last numbers it gave.

`GET /stats` also carries `calendar`: the account's last year of contributions as the GitHub profile draws it, `{ login, total, days: [{ date, count, level }], fetched_at, stale }`, with `level` from 0 to 4. It comes from GitHub's GraphQL API, needs a token, is fresh for `CALENDAR_FRESH_SECS` (an hour) and is `null` without a token or when it could not be read and nothing is cached. `GITHUB_LOGIN` names the account; it defaults to the owner of the first allowlisted repository.

`counts` is not from GitHub. A sweep downloads a tarball of each repository's default branch, counts every line of source, every test attribute and every clippy lint set to warn or deny, and throws the files away. It runs at startup and then every `COUNTS_SWEEP_SECS`, measuring a repository again only when GitHub's `pushed_at` is newer than its last measurement. `counts` is `null` until the first sweep reaches the repository. The counting lives in the `measure` crate in this workspace.

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
