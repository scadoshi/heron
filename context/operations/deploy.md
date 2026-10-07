# Deploy

These steps were run on 2026-09-29, and both heron and steller came up on the first start. "From GitHub Actions" has not been run.

heron runs on its own Hetzner box. `../architecture/hosting.md` says what else is on it and how traffic arrives.

## Before the first deploy

1. Create the GitHub token. Without one the limit is 60 requests an hour. Under Settings, Developer settings, Personal access tokens, Fine-grained tokens:
    - **Repository access:** "Public repositories". This is read-only by definition and cannot see a private repository at all.
    - **Permissions:** none. Leave every repository and account permission unset.
    - **Expiration:** set one, and put the date in your calendar. `runbook.md` has the rotation steps.
2. Pick the public hostname.
3. Create the box. Ubuntu Server on the smallest plan is enough: heron's unit and steller's each cap it at 256M.

## Setting up the box

Lock it down before anything listens:

```bash
sudo ufw default deny incoming
sudo ufw default allow outgoing
sudo ufw allow OpenSSH
sudo ufw enable
```

The box is also on the owner's tailnet as `heron`, and scotland-server's key is in root's `authorized_keys`: its probe reads heron's and steller's journals over SSH for crashes.

Then the tunnel, from the Cloudflare dashboard under Networking, Tunnels:

1. Create a tunnel of type Cloudflared. Choose Debian, 64-bit.
2. Run both commands it shows on the box. The first installs `cloudflared`. The second, `cloudflared service install <token>`, connects it, and the token in it is a credential.
3. Add a route of the kind called a published application. A private hostname is reachable only through Cloudflare's client and is the wrong kind.
4. Subdomain `api`, domain `scadoshi.dev`, no path, service URL `http://127.0.0.1:3100`. The scheme is required.

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

`verify` prints nothing for a unit it accepts. On Ubuntu 26.04 it does print two lines about `CPUAccounting=` in `xfs_scrub_all.service` and `system-xfs_scrub.slice`. Those are Ubuntu's units and not this one.

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
curl -fsS https://api.scadoshi.dev/health
```

A machine that looked the hostname up before the route existed will have cached the miss and say it cannot resolve the host. Ask a resolver directly with `dig @1.1.1.1 api.scadoshi.dev`.

## From GitHub Actions

`.github/workflows/deploy.yml` deploys through a self-hosted runner on the box, registered to this repo. A push to `main` deploys heron after the tests and lints pass on GitHub's runners. steller is deployed from the Actions tab, "Run workflow", choosing `steller` or `both`, since it is written by hand and it is the cache under a live service.

Each deploy keeps the old binary at `/home/runner/deploy/<name>.previous` and puts it back if the new one does not answer within ten seconds. steller is skipped when its `main` is already the installed commit, recorded in `/home/runner/deploy/steller.commit`.

The runner is the `runner` user, installed the way GitHub's "New self-hosted runner" page shows and run as a service with `svc.sh`. It has its own Rust toolchain under `/home/runner/.cargo`. Its `sudo` is `/etc/sudoers.d/runner`: stop, start and restart of the two services, `install` of the two binaries from `/home/runner/deploy/<name>` and nowhere else, and the last 50 lines of each service's log. `sudo` refuses wildcards in arguments, which is why the paths are fixed.

Both repos are public. A pull request from a fork runs the fork's copy of a workflow, so the repos are set to require approval for every external contributor before any workflow runs. That setting is what keeps a stranger's pull request off this box.

## Adding steller

After heron is up on `memory`:

1. Build steller and install the binary:

    ```bash
    git clone https://github.com/scadoshi/steller.git && cd steller
    cargo build --release --locked
    sudo install -m 755 target/release/steller /usr/local/bin/steller
    ```

2. Give it a user and a place to write. steller writes `cache/aof` and `cache/snapshot` under its working directory and does not create `cache` itself:

    ```bash
    sudo useradd --system --no-create-home --shell /usr/sbin/nologin steller
    sudo mkdir -p /var/lib/steller/cache
    sudo chown -R steller:steller /var/lib/steller
    sudo chmod 750 /var/lib/steller
    ```

3. Install its unit, `deploy/steller.service` in steller's repo. It carries `MemoryMax=`, which matters: steller has no memory bound and no eviction.

    ```bash
    sudo cp deploy/steller.service /etc/systemd/system/
    systemd-analyze verify /etc/systemd/system/steller.service
    sudo systemctl daemon-reload
    sudo systemctl enable --now steller
    ```

4. Check it answers. This speaks RESP over a socket, so there is no client to install:

    ```bash
    exec 3<>/dev/tcp/127.0.0.1/3000
    printf '*1\r\n$4\r\nPING\r\n' >&3
    head -c 7 <&3; echo
    exec 3>&-
    ```

    `+PONG` is the answer wanted.

Then point heron at it:

1. In `/etc/systemd/system/heron.service`, uncomment `Wants=steller.service` and `After=steller.service`, then `sudo systemctl daemon-reload`.
2. In `/etc/heron/heron.env`, set `CACHE_BACKEND=layered` and `STELLER_ADDRESS=127.0.0.1:3000`.
3. `sudo systemctl restart heron`, and check `/health/cache` reports `layered` and `healthy`.

`layered` is what runs, so a steller restart costs nothing. `steller` alone would also work now.

To see that it holds, read a repository's `fetched_at`, restart heron, and read it again. A restart empties the memory layer, so the same time means the snapshot came from steller. Then stop steller: `/stats` still answers 200 and `/health/cache` says `unreachable`.
