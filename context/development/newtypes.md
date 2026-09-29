# Newtypes

A few primitives here are wrapped in types that guarantee something about what they hold. This is the inventory and the rules for adding to it.

## What exists

| Type | Wraps | Guarantee | Defined in |
|------|-------|-----------|------------|
| `RepoName` | `String` | `owner/name` with exactly one slash, both halves non-empty and within GitHub's length limits, only ASCII letters, digits, `-`, `_` and `.`, and neither half `.` or `..` | `domain/stats/models/repo_name.rs` |
| `Secret` | `String` | Never printed by `Debug` or `Display`; not empty | `domain/secret/mod.rs` |
| `CacheKey` | `String` | Built only from a `RepoName` and the schema version | `domain/stats/models/cache_key.rs` |

Counts stay plain integers. `commits: u64` is the intended shape. Do not add `CommitCount`.

## Why `RepoName` matters

A repository name arrives from two untrusted places: the URL path of a request, and an environment variable someone typed. It then goes into a URL sent to GitHub and into a cache key sent to steller.

Because the type refuses anything outside its character set, path traversal (`../x`), a header smuggled through a newline, and a query string smuggled through `?` are not things the adapters have to defend against. They cannot be built. The tests in `repo_name.rs` hold the hostile inputs.

## The validating pattern

```rust
pub struct RepoName { full: String, owner_len: usize }

impl RepoName {
    pub fn new(raw: impl AsRef<str>) -> Result<Self, InvalidRepoName> {
        let trimmed = raw.as_ref().trim();
        // one check per error variant, cheapest first
    }
}
```

Four things travel with a validating newtype:

1. `new` returning `Result<Self, InvalidX>`, where `InvalidX` is a `thiserror` enum with one variant per rule.
2. Private fields, so `new` is the only way in.
3. `Deref` to the inner value for reading.
4. A `Deserialize` written by hand that goes through `new`:

```rust
impl<'de> Deserialize<'de> for RepoName {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        RepoName::new(raw).map_err(serde::de::Error::custom)
    }
}
```

The fourth is the one people skip. A derived `Deserialize` takes the fields straight off the wire and never calls `new`, so the type compiles and lies. It matters here: a `RepoName` is inside every `Snapshot`, and snapshots are read back from a cache that another process can write to.

## Secret-bearing types

`Secret` writes `Debug` and `Display` by hand to print a placeholder:

```rust
/// Never derive this: the derive prints the plaintext, and every struct holding a
/// `Secret` inherits that through its own `Debug`.
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(REDACTED)")
    }
}
```

`read()` is the only way to the value, so every use is a call site you can grep for. There is one: `GitHub::get`, where the token becomes a header.

Two tests fail if someone derives either trait again. One of them holds the secret inside another struct, because that is how a derived `Debug` leaks in practice.

## Adding one

Add a newtype when a primitive crosses a trust boundary, or when mixing two values of the same primitive type would be a bug that compiles. Do not add one to make a signature look more careful.

Then add a row to the table above.
