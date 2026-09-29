# Documentation

**Document intent, not what the name already says.** Every doc comment should tell the reader something they could not get from the name and the type.

`comments.md` covers how a comment is worded. This file covers when to write one at all.

## Always document

**What is not obvious from the type.**

```rust
/// Last push to any branch. `None` for a repository that has never been pushed to.
pub pushed_at: Option<DateTime<Utc>>,
```

The `Option` is strange until the reason is given.

**What an outside system means by a field.**

```rust
/// One week of one contributor: `a` lines added, `d` lines deleted.
struct RawWeek { a: u64, d: u64 }
```

GitHub chose those names. The comment is the only place their meaning is written down.

**What a caller can rely on.**

```rust
/// The repositories that resolved, in allowlist order.
pub repos: Vec<RepoReport>,
```

**What a type guarantees.**

```rust
/// Both halves hold only ASCII letters, digits, `-`, `_`, and `.`, and neither is `.`
/// or `..`. A value of this type is therefore safe to place in a URL path or a cache
/// key as is.
pub struct RepoName
```

**Every port method.** A port is a contract between two halves of the program that do not see each other.

## Skip it

**Fields that restate their names.** Use `#[allow(missing_docs)]` on the container, with a doc comment on the container itself:

```rust
/// Sums across the repositories that resolved.
#[allow(missing_docs)]
pub struct Totals {
    pub repos: u32,
    pub commits: u64,
    pub stars: u64,
}
```

**Variants that are their own explanation.** `ApiError::NotFound` needs nothing.

**Private helpers with good names.** `fn page_of(target: &str) -> Option<u64>` is documented by its signature.

## Response types

Every field of an `Http*` contract is part of the public API, and someone will read the JSON without the source beside it. Document what each value means, what `null` means where it can occur, and the format of anything that is a string.

## The test

If a doc comment makes you think "well, obviously", delete it. If it makes you think "I didn't know that", keep it.
