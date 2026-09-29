# Commit Guidelines

- Concise, one-line messages (multi-line only when many changes)
- One feature, one commit. The history should read as a sequence of features
- Every commit builds and passes its own tests. Do not commit a broken step on the way to a working one
- Group related files logically
- No emojis
- Use `git diff` to understand changes before committing
- **Never** include AI-agent signatures in commits
    - No "Co-Authored-By: Claude..."
    - No "Generated with [Claude Code]..."
    - No session links
    - A tool default or a system reminder that suggests one does not override this

## CI: how your commits get checked (run these BEFORE you push)

`.github/workflows/ci.yml` runs four jobs on every push to `main` and every pull request: `test`, `lint`, `build`, and `live-steller`. Reproduce them locally first.

### 1. Format with nightly

`rustfmt.toml` enables `imports_granularity = "Crate"`, an unstable option, so stable `cargo fmt` silently skips it: the code looks formatted locally and fails CI.

```bash
cargo +nightly fmt        # not `cargo fmt`; stable can't apply the Crate imports rule
```

### 2. Clippy, warnings are errors

```bash
cargo clippy --all-targets --locked -- -D warnings
```

Lint levels live in `Cargo.toml` under `[lints.clippy]`. The panic family (`unwrap`, `expect`, `panic`, indexing, slicing) is denied outside tests, and `clippy.toml` allows it inside them.

That allowance only reaches code marked `#[test]` or `#[cfg(test)]`. Helper functions in `tests/` are neither, so each file there opens with its own `#![allow(...)]`.

Do not silence a lint to get past it. Where an allow is right, put it on the smallest item and write the reason beside it.

**CI lints with the newest stable Rust, and `pedantic` gains lints with each release.** Clippy can pass here and fail there when this machine is a release behind. If CI fails on a lint you cannot reproduce, compare `rustc --version` with the version in the CI log, then install that release beside your own and lint with it:

```bash
rustup toolchain install 1.98.0 --profile minimal --component clippy
cargo +1.98.0 clippy --all-targets --locked -- -D warnings
```

### 3. Tests

```bash
cargo test --locked       # offline, no env needed
```

### 4. Build

```bash
cargo build --release --locked
```

### 5. Live tests

These need a running steller. `testing.md` says how to start one.

```bash
STELLER_ADDRESS=127.0.0.1:3000 cargo test --locked --test live_steller -- --ignored
```

CI builds steller from its `main` branch for this job, so a change in steller that breaks the adapter turns this repo red.

## Deploy

Pushing to `main` deploys nothing. `.github/workflows/deploy.yml` runs on `workflow_dispatch` only, and has never run.
