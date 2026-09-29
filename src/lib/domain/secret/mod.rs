//! [`Secret`], a string that never prints its contents.

use thiserror::Error;

/// Why a [`Secret`] could not be built.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum InvalidSecret {
    /// Empty, or whitespace only.
    #[error("secret cannot be empty")]
    Empty,
}

/// A credential. `Debug` and `Display` print a placeholder, and [`Secret::read`] is the
/// only way to the value, so every access is a visible call site.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wraps `raw`, trimmed.
    pub fn new(raw: impl AsRef<str>) -> Result<Self, InvalidSecret> {
        let trimmed = raw.as_ref().trim();
        if trimmed.is_empty() {
            return Err(InvalidSecret::Empty);
        }
        Ok(Self(trimmed.to_string()))
    }

    /// The secret value.
    pub fn read(&self) -> &str {
        &self.0
    }
}

/// Never derive this: the derive prints the plaintext, and every struct holding a
/// `Secret` inherits that through its own `Debug`.
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(REDACTED)")
    }
}

impl std::fmt::Display for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("REDACTED")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_never_contain_the_value() {
        let secret = Secret::new("ghp_hunter2").unwrap();
        assert!(!format!("{secret:?}").contains("hunter2"));
        assert!(!format!("{secret}").contains("hunter2"));
    }

    #[test]
    fn a_struct_holding_a_secret_stays_redacted() {
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Holder {
            token: Secret,
        }
        let holder = Holder {
            token: Secret::new("ghp_hunter2").unwrap(),
        };
        assert!(!format!("{holder:?}").contains("hunter2"));
    }

    #[test]
    fn read_returns_the_trimmed_value() {
        assert_eq!(Secret::new("  abc \n").unwrap().read(), "abc");
    }

    #[test]
    fn empty_is_rejected() {
        assert_eq!(Secret::new("   "), Err(InvalidSecret::Empty));
    }
}
