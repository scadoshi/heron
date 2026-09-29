# Comments

**A comment says what the code does, in the present tense, and only when the code does not say it itself.**

Comments are where generated code is easiest to spot, and the owner reads every line. Long ones get skimmed, and then the one that mattered gets skimmed with them.

## Do not write

**The history.** No "previously", "used to", "now", "no longer", "was changed to", and no dates of past incidents. The code has no past tense. Git has the history.

**The situation.** No "the reason this exists", no "this is needed because we ran into".

**The sermon.** No "without this, X would happen, and then Y". Say what the line does. If the consequence is the part that is not obvious, give it one short clause.

**A restatement.** `/// Returns the name.` above `fn name()` is noise. Delete it. When every item in a container explains itself, put `#[allow(missing_docs)]` on the container.

**Advice.** Unless it is an instruction the reader must follow, such as "loopback only, do not widen".

## Do write

- What a caller can rely on: invariants, guarantees, ordering.
- The surprise: GitHub answering 202, why freshness lives in the payload, why a connection is closed after a rejection.
- Units, ranges and formats the type does not carry.

## Before and after

```rust
// Bad: a past the reader does not need, and a sermon.
// Previously we kept the connection open after an error reply, but this caused
// a desync bug where the next command would read a stale error. Without this
// fix the cache would return wrong values.

// Good: what it does, and the one fact that is not obvious.
/// Puts `connection` back for the next command, unless the reply is an error.
///
/// A server that rejects a command partway through reading it goes on to read the
/// rest as new commands and answers each. Those answers would be taken for the
/// replies to whatever is sent next, so the connection is closed.
```

```rust
// Bad: restates the name.
/// The half before the slash in the repo name.
/// Returns the owner of the repository as a string slice.
pub fn owner(&self) -> &str

// Good.
/// The half before the slash.
pub fn owner(&self) -> &str
```

```rust
// Bad: says nothing the field does not.
/// The additions.
pub additions: Option<u64>,

// Good: says what the number is and when it is absent.
/// Lines added across all commits, every rewrite counted again. `None` while
/// GitHub is still computing contributor statistics.
pub additions: Option<u64>,
```

## Form

- Every module opens with a `//!` header.
- Every public item has a doc comment, unless it explains itself.
- Section banners are one line: `// == errors ==`.
- No em dashes, in code or in prose. No spaced hyphen joining two fragments either. Use a sentence, a colon, or a comma.
- American spelling.
- Doc comments wrap at the line width rustfmt uses for code. Markdown files under `context/` do not wrap at all.

## Let the code breathe

A function with a comment on every line has the wrong names. Fix the names.

Before you finish, reread every comment you wrote and delete the ones a competent Rust developer would not miss.

## Checking

```bash
grep -rnE 'previously|used to|no longer|without this|in order to|—' src tests
```

A hit is not always wrong. "used to" also means "used for". Read each one.
