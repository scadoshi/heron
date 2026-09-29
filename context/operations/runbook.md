# Runbook

Written before the first deploy. The commands are right for a systemd box. The symptoms are what the server does on a laptop.

## Is it up

```bash
curl -fsS http://127.0.0.1:3100/health          # the server
curl -fsS http://127.0.0.1:3100/health/cache    # the cache behind it
systemctl status scotland-server
```

`/health/cache` always answers 200. Read the body: `status` is `healthy` or `unreachable`.

## Logs

```bash
journalctl -u scotland-server -f                 # follow
journalctl -u scotland-server -n 200 --no-pager  # the last 200 lines
journalctl -u scotland-server -p warning         # warnings and errors only
```

Every request line carries an `x-request-id`, and the same id is in the response headers.

To log more, set `RUST_LOG=info,scotland=debug` in `.env` and restart.

## What each failure looks like

| From outside | In the log | What it is |
|---|---|---|
| 200 with `"stale": true` | `refresh failed for ..., serving the stale value` | GitHub is unreachable or the rate limit is spent. The old numbers are being served. Nothing to do unless it lasts past the retain window. |
| 503 `stats are temporarily unavailable` | `503 github rate limit exhausted` or `503 github request failed` | GitHub could not be reached and there is no snapshot to fall back on. Usually just after a restart with a cold cache. |
| 200, and `/health/cache` says `unreachable` | `steller cache is down, serving from memory`, once | steller is down under `layered`. Requests are fine. Restart steller. |
| 200, and `/health/cache` says `healthy` again | `steller cache is answering again`, once | steller came back. |
| Every request for one repository reaches GitHub | `cache write failed for ...: steller rejected the command: ERR missing crlf terminator` | The snapshot is over steller's 1024-byte limit. See `../architecture/decisions.md`. Use `layered` until steller is fixed. |
| 404 `repository not found` | `404 repository not found` | The repository is not in `GITHUB_REPOS`, is private, was deleted, or the name does not parse. The response does not say which, on purpose. The log does not either. Check the allowlist. |
| 429 | nothing | One client passed 30 requests in a burst. |
| The service will not start | `scotland-server failed: ...` | A bad or missing variable. The message names it. |

## Switch the cache backend

Edit `CACHE_BACKEND` in `~/scotland-server/.env`, and `STELLER_ADDRESS` if needed, then:

```bash
sudo systemctl restart scotland-server
curl -fsS http://127.0.0.1:3100/health/cache
```

Going to `memory` loses nothing that matters. The first request for each repository asks GitHub again.

## Rotate the GitHub token

1. Create the new token: fine-grained, read-only, public repositories.
2. Replace `GITHUB_TOKEN` in `~/scotland-server/.env`.
3. `sudo systemctl restart scotland-server`
4. Check the startup line in the log says `github token set`.
5. Revoke the old token.

The token is never logged. The startup line says only whether one is set.

## Add or remove a repository

Edit `GITHUB_REPOS` and restart. A name that does not parse, or a name listed twice, stops the server from starting and says which.

A private repository can be listed without harm. It is refused when first read and answers 404.

## Clear one repository's cache

Under `memory`, restart the server.

Under `steller` or `layered`, the key is `scotland:stats:v1:owner/name`. steller has `DEL`:

```bash
redis-cli -p <steller port> DEL scotland:stats:v1:scadoshi/steller
```

Under `layered` the memory layer holds a copy, so restart the server as well.
