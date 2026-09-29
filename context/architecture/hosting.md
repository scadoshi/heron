# Hosting

**Status: not deployed.** Everything below is the plan. Nothing here has been done.

## Where it will run

On its own Hetzner box, with steller beside it. Nothing of Zwipe's runs there.

Sharing zerver's box was the first plan and was dropped. See "Its own box" in `decisions.md`.

## What runs on the box

| Service | Binds | Unit |
|---|---|---|
| heron | `127.0.0.1:3100` | `deploy/heron.service`, in this repo |
| steller | `127.0.0.1:3000`, hardcoded | `deploy/steller.service`, in steller's repo |
| cloudflared | outbound only | installed with the package |

Both servers bind loopback. Nothing listens on a public interface.

## How traffic reaches it

A Cloudflare Tunnel from a public hostname to `http://127.0.0.1:3100`. Use `127.0.0.1`, not `localhost`, as zwipe's Cloudflare notes explain.

The firewall denies all inbound traffic except SSH. The tunnel is an outbound connection from the box, so it needs no open port.

**This is not only tidiness.** The rate limiter keys on `CF-Connecting-IP`, and that header is only trustworthy when every request comes through Cloudflare. If heron were ever bound to a public interface, a client could send any value it liked and have a rate-limit bucket to itself. Keep heron on loopback behind the tunnel, or change `CfConnectingIpKeyExtractor` first.

Stats responses carry `Cache-Control: public, max-age=300`, so the edge holds each for five minutes.

## What the portfolio does with it

It reads `GET /stats` when it builds, in CI, and bakes the numbers into static HTML. The site does not call this server from the browser and keeps working when this server is down.

That change is in the portfolio repo and has not been made.
