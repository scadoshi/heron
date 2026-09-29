# Hosting

**Status: not deployed.** Everything below is the plan. Nothing here has been done.

## Where it will run

On the box that runs zerver: Ubuntu Server, headless, x86_64, behind a Cloudflare Tunnel. That box is described in zwipe's `context/architecture/hosting.md`.

The owner chose one box over two. The cost of that choice is that scotland-server and steller share a failure domain with Zwipe's production API and its Postgres, so both run under systemd with memory and task caps.

## Ports on that box

| Port | Held by |
|---|---|
| 3000 | zerver, bound `0.0.0.0:3000` |
| 3000 | steller, hardcoded `127.0.0.1:3000` |
| 3100 | scotland-server, by convention in `.env.example` |
| 5432 | Postgres |

**The first two collide.** steller cannot start on that box until its bind address is configurable. That is work in steller, and it blocks `CACHE_BACKEND=steller` and `layered` in production. `CACHE_BACKEND=memory` is not blocked.

## How traffic will reach it

A Cloudflare Tunnel route from a public hostname to `http://127.0.0.1:3100`. Use `127.0.0.1`, not `localhost`, as zwipe's Cloudflare notes explain.

The rate limiter keys on `CF-Connecting-IP`. That header is trusted because the origin is reachable only through the tunnel.

Stats responses carry `Cache-Control: public, max-age=300`, so the edge holds each for five minutes.

## What the portfolio does with it

It reads `GET /stats` when it builds, in CI, and bakes the numbers into static HTML. The site does not call this server from the browser and keeps working when this server is down.

That change is in the portfolio repo and has not been made.
