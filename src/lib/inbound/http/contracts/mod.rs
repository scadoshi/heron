//! The JSON the API answers with. Domain types convert into these and carry nothing
//! HTTP-shaped themselves.

/// Health responses.
pub mod health;

/// Stats responses.
pub mod stats;

/// Serializes a timestamp as RFC 3339 in UTC to the second: `2026-09-29T16:04:41Z`.
mod rfc3339 {
    use chrono::{DateTime, SecondsFormat, Utc};
    use serde::Serializer;

    pub fn serialize<S: Serializer>(at: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&at.to_rfc3339_opts(SecondsFormat::Secs, true))
    }

    pub mod option {
        use super::{DateTime, SecondsFormat, Serializer, Utc};

        // serde's `serialize_with` passes a reference to the field.
        #[allow(clippy::ref_option)]
        pub fn serialize<S: Serializer>(
            at: &Option<DateTime<Utc>>,
            s: S,
        ) -> Result<S::Ok, S::Error> {
            match at {
                Some(at) => s.serialize_some(&at.to_rfc3339_opts(SecondsFormat::Secs, true)),
                None => s.serialize_none(),
            }
        }
    }
}
