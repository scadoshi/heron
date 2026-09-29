# Deploy

**Not verified.** Nothing here has been run on a server. Treat each step as a draft and correct this file as you go.

## Before the first deploy

1. Decide the cache backend. `memory` needs nothing else. `steller` and `layered` need steller running on the box, which is blocked: see `../architecture/hosting.md`.
2. Create a fine-grained GitHub token with read-only access to public repositories. Without one the limit is 60 requests an hour for the whole box.
3. Pick the public hostname and add the Cloudflare Tunnel route to `http://127.0.0.1:3100`.

## First-time setup on the server

```bash
mkdir -p ~/heron
cd ~/heron
```

Write `~/heron/.env` from `.env.example`. Set `BIND_ADDRESS=127.0.0.1:3100`, and put the token in `GITHUB_TOKEN`. Then:

```bash
chmod 600 ~/heron/.env
```

Copy the unit into place and check it before enabling it:

```bash
sudo cp deploy/heron.service /etc/systemd/system/
systemd-analyze verify /etc/systemd/system/heron.service
sudo systemctl daemon-reload
sudo systemctl enable heron
```

The unit hardcodes `User=scadoshi` and paths under `/home/scadoshi/heron`, following zerver's layout. Change both if the box differs.

`ProtectHome=read-only` lets the service read its binary and `.env` under `/home` and write nothing there. If `systemd-analyze verify` or the first start objects, that line is the first suspect.

## Deploying

Build on the server, or build elsewhere for `x86_64-unknown-linux-gnu` and copy the binary over.

```bash
cargo build --release --locked
sudo systemctl stop heron
cp target/release/heron ~/heron/
sudo systemctl start heron
```

Then ask it, since the unit starting is not the server serving:

```bash
curl -fsS http://127.0.0.1:3100/health
curl -fsS http://127.0.0.1:3100/health/cache
curl -fsS http://127.0.0.1:3100/stats | head -c 300
```

## From GitHub Actions

`.github/workflows/deploy.yml` does the same on a self-hosted runner, gated on tests and lints. It runs on `workflow_dispatch` only. The comment at the top of the file says which lines to add to deploy on every push.

The runner needs passwordless `sudo` for `systemctl stop` and `systemctl start` on this one unit.

## With steller

Once steller can bind an address other than `127.0.0.1:3000`:

1. Install steller's own unit from its repo, `deploy/steller.service`. It carries `MemoryMax=`, which matters here: steller has no memory bound and no eviction, and the box also runs Postgres.
2. In `heron.service`, uncomment `Wants=steller.service` and `After=steller.service`.
3. In `.env`, set `CACHE_BACKEND=layered` and `STELLER_ADDRESS` to steller's address.
4. Restart, and check `/health/cache` reports `layered` and `healthy`.
