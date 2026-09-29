//! The `Link` response header (RFC 8288), as far as pagination needs it.

use reqwest::Url;

/// Page number in the target of the `rel="last"` link, if the header has one.
///
/// The header is a comma-separated list of `<target>; param; param`. Targets are
/// read between their angle brackets, so a comma or semicolon inside one does not
/// split it, and the parameters may come in any order.
pub fn last_page(header: &str) -> Option<u64> {
    let mut rest = header;
    loop {
        let (_, after_open) = rest.split_once('<')?;
        let (target, after_target) = after_open.split_once('>')?;
        let params = after_target.split('<').next().unwrap_or_default();
        if params.split(';').any(is_rel_last) {
            return page_of(target);
        }
        rest = after_target;
    }
}

/// Whether `param` is a `rel` naming `last`. A `rel` may hold several relation types
/// separated by spaces.
fn is_rel_last(param: &str) -> bool {
    let Some((name, value)) = param.trim().trim_end_matches(',').split_once('=') else {
        return false;
    };
    name.trim().eq_ignore_ascii_case("rel")
        && value
            .trim()
            .trim_matches('"')
            .split_ascii_whitespace()
            .any(|relation| relation.eq_ignore_ascii_case("last"))
}

fn page_of(target: &str) -> Option<u64> {
    Url::parse(target)
        .ok()?
        .query_pairs()
        .find(|(name, _)| name == "page")
        .and_then(|(_, value)| value.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_page_of_the_last_link() {
        let header = r#"<https://api.github.com/repositories/1/commits?per_page=1&page=2>; rel="next", <https://api.github.com/repositories/1/commits?per_page=1&page=201>; rel="last""#;
        assert_eq!(last_page(header), Some(201));
    }

    #[test]
    fn the_last_link_may_come_first() {
        let header = r#"<https://x.test/c?page=7&per_page=1>; rel="last", <https://x.test/c?page=2&per_page=1>; rel="next""#;
        assert_eq!(last_page(header), Some(7));
    }

    #[test]
    fn page_may_sit_anywhere_in_the_query() {
        let header = r#"<https://x.test/c?sha=main&page=42&per_page=1>; rel="last""#;
        assert_eq!(last_page(header), Some(42));
    }

    #[test]
    fn a_similarly_named_parameter_is_not_the_page() {
        let header = r#"<https://x.test/c?per_page=1&page=9>; rel="last""#;
        assert_eq!(last_page(header), Some(9));
        let header = r#"<https://x.test/c?per_page=1&homepage=3>; rel="last""#;
        assert_eq!(last_page(header), None);
    }

    #[test]
    fn rel_may_follow_other_parameters_or_go_unquoted() {
        let header = r#"<https://x.test/c?page=5>; title="end"; rel=last"#;
        assert_eq!(last_page(header), Some(5));
    }

    #[test]
    fn rel_may_name_several_relations() {
        let header = r#"<https://x.test/c?page=3>; rel="next last""#;
        assert_eq!(last_page(header), Some(3));
    }

    #[test]
    fn a_comma_inside_a_target_does_not_split_it() {
        let header = r#"<https://x.test/c?sha=a,b&page=2>; rel="next", <https://x.test/c?sha=a,b&page=12>; rel="last""#;
        assert_eq!(last_page(header), Some(12));
    }

    #[test]
    fn no_last_link_is_none() {
        assert_eq!(
            last_page(
                r#"<https://x.test/c?page=1>; rel="prev", <https://x.test/c?page=1>; rel="first""#
            ),
            None
        );
        assert_eq!(last_page(""), None);
        assert_eq!(last_page("garbage"), None);
        assert_eq!(
            last_page(r#"<https://x.test/c?page=abc>; rel="last""#),
            None
        );
    }
}
