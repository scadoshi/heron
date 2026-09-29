# Deploy

**Not verified.** Nothing here has been run on a server. Treat each step as a draft and correct this file as you go.

heron runs on its own Hetzner box. `../architecture/hosting.md` says what else is on it and how traffic arrives.

## Before the first deploy

1. Create the GitHub token. Without one the limit is 60 requests an hour. Under Settings, Developer settings, Personal access tokens, Fine-grained tokens:
    - **Repository access:** "Public repositories". This is read-only by definition and cannot see a private repository at all.
    - **Permissions:** none. Leave every repository and account permission unset.
    - **Expiration:** set one, and put the date in your calendar. `runbook.md` has the rotation steps.
2. Pick the public hostname.
3. Create the box. Ubuntu Server on the smallest plan is enough: heron's unit caps it at 128M and steller's at 256M.

## Setting up the box

Lock it down before anything listens:

```bash
sudo ufw default deny incoming
sudo ufw default allow outgoing
sudo ufw allow OpenSSH
sudo ufw enable
```

Install `cloudflared`, create the tunnel, and route the hostname to `http://127.0.0.1:3100`. zwipe's `context/operations/infrastructure/cloudflare.md` has the steps.

## Installing heron

A user that owns nothing and can log in nowhere:

```bash
sudo useradd --system --no-create-home --shell /usr/sbin/nologin heron
```

The environment file, readable by root and the service only:

```bash
sudo mkdir -p /etc/heron
sudo cp .env.example /etc/heron/heron.env
sudo chown root:heron /etc/heron/heron.env
sudo chmod 640 /etc/heron/heron.env
sudoedit /etc/heron/heron.env
```

Set `BIND_ADDRESS=127.0.0.1:3100`, `ALLOWED_ORIGINS`, `GITHUB_REPOS`, and `GITHUB_TOKEN`. Start with `CACHE_BACKEND=memory`.

systemd reads this file itself and does not run it through a shell. Write `KEY=value` with no `export` and no quotes around values.

The unit, checked before it is enabled:

```bash
sudo cp deploy/heron.service /etc/systemd/system/
systemd-analyze verify /etc/systemd/system/heron.service
sudo systemctl daemon-reload
sudo systemctl enable heron
```

## Deploying

Build on the box, or build elsewhere for its architecture and copy the binary over. Hetzner's cheapest plans are ARM, so check `uname -m` before cross-compiling.

```bash
cargo build --release --locked
sudo systemctl stop heron
sudo install -m 755 target/release/heron /usr/local/bin/heron
sudo systemctl start heron
```

Then ask it, since the unit starting is not the server serving:

```bash
curl -fsS http://127.0.0.1:3100/health
curl -fsS http://127.0.0.1:3100/health/cache
curl -fsS http://127.0.0.1:3100/stats | head -c 300
```

And from outside, through the tunnel:

```bash
curl -fsS https://<hostname>/health
```

## From GitHub Actions

`.github/workflows/deploy.yml` does the same on a self-hosted runner, gated on tests and lints. It runs on `workflow_dispatch` only. The comment at the top of the file says which lines to add to deploy on every push.

The runner needs passwordless `sudo` for `systemctl stop heron`, `systemctl start heron`, and the `install` into `/usr/local/bin`.

A self-hosted runner compiles Rust on the box, which takes more memory than heron and steller together. If the box is small, build in a GitHub-hosted job and copy the binary over instead.

## Adding steller

After heron is up on `memory`:

1. Build steller and install it with its own unit, `deploy/steller.service` in steller's repo. It carries `MemoryMax=`, which matters: steller has no memory bound and no eviction.
2. Check it answers: `redis-cli -p 3000 PING`.
3. In `/etc/systemd/system/heron.service`, uncomment `Wants=steller.service` and `After=steller.service`, then `sudo systemctl daemon-reload`.
4. In `/etc/heron/heron.env`, set `CACHE_BACKEND=layered` and `STELLER_ADDRESS=127.0.0.1:3000`.
5. `sudo systemctl restart heron`, and check `/health/cache` reports `layered` and `healthy`.

Use `layered`, not `steller`, until steller reads commands past 1024 bytes. See `../architecture/decisions.md`.
