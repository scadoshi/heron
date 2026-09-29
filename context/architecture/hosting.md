# Hosting

**Status: live since 2026-09-29 at `https://api.scadoshi.dev`, on `CACHE_BACKEND=layered`.** steller runs beside it.

## Where it runs

On its own Hetzner box: a CX23 in Nuremberg, x86_64, 4 GB, Ubuntu 26.04. Nothing of Zwipe's runs there.

Sharing zerver's box was the first plan and was dropped. See "Its own box" in `decisions.md`.

## What runs on the box

| Service | Binds | Unit |
|---|---|---|
| heron | `127.0.0.1:3100` | `deploy/heron.service`, in this repo |
| steller | `127.0.0.1:3000`, hardcoded | `deploy/steller.service`, in steller's repo |
| cloudflared | outbound only | installed with the package |

Both servers bind loopback. Nothing listens on a public interface.

## How traffic reaches it

A Cloudflare Tunnel named `heron` routes `api.scadoshi.dev` to `http://127.0.0.1:3100`. The tunnel and its route are managed in the Cloudflare dashboard, not in a file on the box. The address is `127.0.0.1`, not `localhost`, as zwipe's Cloudflare notes explain.

The hostname is on `scadoshi.dev` and not on the portfolio's domain because heron serves `github.com/scadoshi` and is more than the portfolio's backend. `.dev` is on the HSTS preload list, so browsers refuse plain HTTP to it.

`ufw` denies all inbound traffic except SSH. The tunnel is an outbound connection from the box, so it needs no open port. A request to the box's own address on port 3100 gets no answer.

**This is not only tidiness.** The rate limiter keys on `CF-Connecting-IP`, and that header is only trustworthy when every request comes through Cloudflare. If heron were ever bound to a public interface, a client could send any value it liked and have a rate-limit bucket to itself. Keep heron on loopback behind the tunnel, or change `CfConnectingIpKeyExtractor` first.

Stats responses carry `Cache-Control: public, max-age=300`, so the edge holds each for five minutes.

## What the portfolio does with it

It reads `GET /stats` when it builds, in CI, and bakes the numbers into static HTML. The site does not call this server from the browser and keeps working when this server is down.

That change is in the portfolio repo and has not been made.
