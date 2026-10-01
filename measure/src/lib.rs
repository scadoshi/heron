//! Lines, tests and clippy lints counted from a checkout.
//!
//! The measure is what the source says, not what a build prints: every line of every
//! source file outside build directories, every test attribute, and every clippy
//! entry a `Cargo.toml` sets to `warn` or `deny`.

#![warn(missing_docs)]

use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

/// The language a repository is written in, decided by what sits at its root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    /// A `Cargo.toml` at the root.
    Rust,
    /// A `.csproj`, `.sln` or `.slnx` at the root.
    CSharp,
}

impl Language {
    /// The language's name as the site prints it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::CSharp => "C#",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Rust => "rs",
            Self::CSharp => "cs",
        }
    }

    /// True when a trimmed source line is a test attribute.
    fn is_test_attribute(self, line: &str) -> bool {
        match self {
            Self::Rust => {
                line == "#[test]"
                    || [
                        "#[tokio::test",
                        "#[rstest",
                        "#[wasm_bindgen_test",
                        "#[sqlx::test",
                    ]
                    .iter()
                    .any(|prefix| line.starts_with(prefix))
            }
            Self::CSharp => ["[Fact", "[Theory", "[Test]", "[TestMethod"]
                .iter()
                .any(|prefix| line.starts_with(prefix)),
        }
    }
}

/// What one repository's source contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    /// The language the files were counted as.
    pub language: Language,
    /// Lines in every source file, blanks and comments included.
    pub lines: u64,
    /// Test attributes, one per test function.
    pub tests: u64,
    /// Clippy lints set to `warn` or `deny`. `None` for a language clippy does not cover.
    pub clippy_lints: Option<u64>,
}

/// Directories that hold build output or dependencies, never source.
const SKIPPED: &[&str] = &["target", "bin", "obj", "node_modules"];

/// Measures a checkout. `None` when it is neither a Rust nor a C# repository.
pub fn measure(repo: &Path) -> io::Result<Option<Counts>> {
    let Some(language) = language_of(repo)? else {
        return Ok(None);
    };
    let mut lines: u64 = 0;
    let mut tests: u64 = 0;
    walk(repo, language, &mut |source| {
        for line in source.lines() {
            lines = lines.saturating_add(1);
            if language.is_test_attribute(line.trim()) {
                tests = tests.saturating_add(1);
            }
        }
    })?;
    let clippy_lints = match language {
        Language::Rust => Some(clippy_lints(&fs::read_to_string(repo.join("Cargo.toml"))?)),
        Language::CSharp => None,
    };
    Ok(Some(Counts {
        language,
        lines,
        tests,
        clippy_lints,
    }))
}

/// Rust when there is a `Cargo.toml` at the root, C# when there is a project or
/// solution file.
fn language_of(repo: &Path) -> io::Result<Option<Language>> {
    if repo.join("Cargo.toml").is_file() {
        return Ok(Some(Language::Rust));
    }
    for entry in fs::read_dir(repo)? {
        let path = entry?.path();
        if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("csproj" | "sln" | "slnx")
        ) {
            return Ok(Some(Language::CSharp));
        }
    }
    Ok(None)
}

/// Calls `visit` with the text of every source file under `dir`, skipping hidden
/// directories and the ones in `SKIPPED`. A file that is not UTF-8 is skipped.
fn walk(dir: &Path, language: Language, visit: &mut dyn FnMut(&str)) -> io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if !name.starts_with('.') && !SKIPPED.contains(&name.as_ref()) {
                walk(&path, language, visit)?;
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some(language.extension())
            && let Ok(source) = fs::read_to_string(&path)
        {
            visit(&source);
        }
    }
    Ok(())
}

/// Entries under `[lints.clippy]` or `[workspace.lints.clippy]` whose level is `warn`
/// or `deny`, in either the `"deny"` or the `{ level = "deny" }` form.
fn clippy_lints(cargo_toml: &str) -> u64 {
    let mut in_clippy = false;
    let mut lints: u64 = 0;
    for line in cargo_toml.lines().map(str::trim) {
        if line.starts_with('[') {
            in_clippy = matches!(line, "[lints.clippy]" | "[workspace.lints.clippy]");
        } else if in_clippy && !line.starts_with('#') {
            let Some((_, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            if ["deny", "warn"].iter().any(|level| {
                value == format!("\"{level}\"") || value.contains(&format!("level = \"{level}\""))
            }) {
                lints = lints.saturating_add(1);
            }
        }
    }
    lints
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A throwaway directory under the system temp dir, removed on drop.
    struct Checkout(PathBuf);

    impl Checkout {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("measure_{}_{name}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, path: &str, text: &str) -> &Self {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
            self
        }
    }

    impl Drop for Checkout {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_rust_checkout_counts_lines_tests_and_lints_outside_build_directories() {
        let checkout = Checkout::new("rust");
        checkout
            .write("Cargo.toml", "[package]\nname = \"x\"\n\n[lints.clippy]\ntodo = \"deny\"\n")
            .write("src/lib.rs", "pub fn a() {}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {}\n    #[tokio::test]\n    async fn u() {}\n}\n")
            .write("target/debug/build.rs", "#[test]\nfn not_counted() {}\n")
            .write(".git/hooks/pre-commit.rs", "#[test]\nfn not_counted() {}\n")
            .write("README.md", "#[test] in prose is not source\n");
        assert_eq!(
            measure(&checkout.0).unwrap(),
            Some(Counts {
                language: Language::Rust,
                lines: 9,
                tests: 2,
                clippy_lints: Some(1),
            })
        );
    }

    #[test]
    fn a_csharp_checkout_counts_cs_files_and_has_no_lint_count() {
        let checkout = Checkout::new("csharp");
        checkout
            .write("Thing.slnx", "")
            .write("src/A.cs", "class A {}\n")
            .write(
                "tests/ATests.cs",
                "[Fact]\npublic void T() {}\n[Theory]\npublic void U() {}\n",
            )
            .write(
                "src/bin/Debug/A.cs",
                "[Fact]\npublic void NotCounted() {}\n",
            );
        assert_eq!(
            measure(&checkout.0).unwrap(),
            Some(Counts {
                language: Language::CSharp,
                lines: 5,
                tests: 2,
                clippy_lints: None,
            })
        );
    }

    #[test]
    fn a_checkout_in_neither_language_is_none() {
        let checkout = Checkout::new("other");
        checkout.write("index.html", "<p>hi</p>\n");
        assert_eq!(measure(&checkout.0).unwrap(), None);
    }

    #[test]
    fn clippy_lints_counts_warn_and_deny_in_a_clippy_section() {
        let toml = r#"
[lints.rust]
unsafe_code = "deny"

[lints.clippy]
pedantic = { level = "warn", priority = -1 }
# a comment with "deny" in it
todo = "deny"
unwrap_used = { level = "deny", priority = 1 }
as_conversions = "warn"
must_use_candidate = "allow"

[dependencies]
serde = "deny"
"#;
        assert_eq!(clippy_lints(toml), 4);
    }

    #[test]
    fn clippy_lints_reads_a_workspace_section() {
        assert_eq!(
            clippy_lints("[workspace.lints.clippy]\npanic = \"deny\"\nexit = \"warn\"\n"),
            2
        );
    }
}
