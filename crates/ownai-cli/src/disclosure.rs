//! Privacy controls for semantic analysis.
//!
//! This module decides what a review would disclose, where it would go, and
//! whether the operator has acknowledged it. It never builds or calls a
//! provider: planning is pure, so a dry run cannot transmit anything. The
//! engine stays free of provider and privacy concepts; the CLI owns this UX.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use ownai_core::RepoPath;
use ownai_engine::{FileDiff, ReviewLens};

use crate::decision::ProviderRoute;

/// A pre-flight summary of one planned review.
///
/// It is computed entirely from the deterministic item diffs and the configured
/// state limit, before any provider exists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewPlan {
    /// The provider's command-line label.
    pub provider: String,
    /// Where the provider sends state.
    pub route: ProviderRoute,
    /// The remote host, when one was configured or parsed. Remote-only.
    pub host: Option<String>,
    /// The requested model alias, when one was configured.
    pub model: Option<String>,
    /// The number of size-eligible review units.
    pub units: usize,
    /// The total serialized state of the size-eligible units.
    pub state_bytes: usize,
    /// The number of units over the local state limit.
    pub skipped: usize,
    /// The eligible paths, ascending and deduplicated.
    pub paths: Vec<RepoPath>,
}

/// Summarizes a planned review without calling any provider.
///
/// Units come from [`FileDiff::item_diffs`], exactly as [`ReviewLens::review_file`]
/// uses them, so the plan counts the same units the review would evaluate. A
/// unit over [`ReviewLens::max_state_bytes`] is counted as skipped and its path
/// is excluded.
pub fn review_plan(
    provider: &str,
    route: ProviderRoute,
    host: Option<&str>,
    model: Option<&str>,
    lens: &ReviewLens,
    diffs: &[FileDiff],
) -> ReviewPlan {
    let limit = lens.max_state_bytes();
    let mut units = 0;
    let mut state_bytes = 0;
    let mut skipped = 0;
    let mut paths = BTreeSet::new();

    for diff in diffs {
        for item in diff.item_diffs() {
            let bytes = ReviewLens::state_bytes(&item);
            if bytes > limit {
                skipped += 1;
            } else {
                units += 1;
                state_bytes += bytes;
                paths.insert(item.path);
            }
        }
    }

    ReviewPlan {
        provider: provider.to_owned(),
        route,
        host: host.map(str::to_owned),
        model: model.map(str::to_owned),
        units,
        state_bytes,
        skipped,
        paths: paths.into_iter().collect(),
    }
}

/// A malformed endpoint URL.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum EndpointError {
    /// The URL does not use a supported scheme.
    #[error("endpoint `{url}` must start with `http://` or `https://`")]
    UnsupportedScheme { url: String },

    /// The URL embeds credentials, which must never be sent or stored.
    #[error("endpoint `{url}` embeds credentials; pass secrets in the environment instead")]
    EmbeddedCredentials { url: String },

    /// The URL has no host component.
    #[error("endpoint `{url}` does not name a host")]
    MissingHost { url: String },
}

/// Validates an endpoint URL and returns its host.
///
/// Only `http://` and `https://` are accepted. The authority is the text after
/// the scheme up to the first `/`, `?`, or `#`. Credentials (`@`) are rejected
/// outright, and an empty host is an error. Bracketed IPv6 literals and
/// `host:port` forms are both understood.
pub fn endpoint_host(url: &str) -> Result<String, EndpointError> {
    let unsupported = || EndpointError::UnsupportedScheme {
        url: url.to_owned(),
    };
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or_else(unsupported)?;

    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];

    if authority.contains('@') {
        return Err(EndpointError::EmbeddedCredentials {
            url: url.to_owned(),
        });
    }

    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        match bracketed.find(']') {
            Some(close) => &bracketed[..close],
            None => {
                return Err(EndpointError::MissingHost {
                    url: url.to_owned(),
                });
            }
        }
    } else {
        authority.split(':').next().unwrap_or("")
    };

    if host.is_empty() {
        return Err(EndpointError::MissingHost {
            url: url.to_owned(),
        });
    }
    Ok(host.to_owned())
}

/// A remote disclosure the operator has not acknowledged.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DisclosureError {
    /// A non-interactive remote review was not acknowledged.
    #[error(
        "remote semantic analysis for `{provider}` requires acknowledgement; \
         re-run with `--accept-disclosure` or set `OWNAI_ACCEPT_DISCLOSURE=1`"
    )]
    NotAcknowledged { provider: String },
}

/// Enforces the remote acknowledgement gate.
///
/// Only a remote route with neither an interactive stderr nor an explicit
/// acknowledgement fails. Local and offline routes never require one.
pub fn check_disclosure(
    provider: &str,
    route: ProviderRoute,
    interactive: bool,
    accepted: bool,
) -> Result<(), DisclosureError> {
    if route == ProviderRoute::Remote && !interactive && !accepted {
        return Err(DisclosureError::NotAcknowledged {
            provider: provider.to_owned(),
        });
    }
    Ok(())
}

/// Renders the disclosure for a plan.
///
/// A dry run appends the skipped count, the affected paths, and a closing
/// "no request was sent" line. The disclosure form ends with the route-specific
/// notice where one applies. The result is deterministic plain text ending in
/// exactly one newline.
pub fn render_plan(plan: &ReviewPlan, dry_run: bool) -> String {
    let mut out = String::new();

    if dry_run {
        let _ = writeln!(out, "ownai review dry run");
    } else {
        let _ = writeln!(out, "ownai review disclosure");
    }
    let _ = writeln!(out, "provider: {}", plan.provider);
    let _ = writeln!(out, "route: {}", route_key(plan.route));
    if plan.route == ProviderRoute::Remote {
        let _ = writeln!(
            out,
            "host: {}",
            plan.host.as_deref().unwrap_or("(provider default)")
        );
    }
    let _ = writeln!(
        out,
        "model: {}",
        plan.model.as_deref().unwrap_or("(provider default)")
    );
    let _ = writeln!(out, "units: {}", plan.units);
    let _ = writeln!(out, "serialized-state-bytes: {}", plan.state_bytes);

    if dry_run {
        let _ = writeln!(out, "skipped: {}", plan.skipped);
        let _ = writeln!(out, "paths:");
        if plan.paths.is_empty() {
            let _ = writeln!(out, "  (none)");
        } else {
            for path in &plan.paths {
                let _ = writeln!(out, "  {path}");
            }
        }
        let _ = writeln!(out, "no request was sent");
    } else {
        match plan.route {
            ProviderRoute::Remote => {}
            ProviderRoute::Local => {
                let _ = writeln!(out, "state stays on this machine");
            }
            ProviderRoute::LocalTest => {
                let _ = writeln!(out, "no network access");
            }
        }
    }

    out
}

/// The stable lowercase key for a route, as rendered in a disclosure.
fn route_key(route: ProviderRoute) -> &'static str {
    match route {
        ProviderRoute::Remote => "remote",
        ProviderRoute::Local => "local",
        ProviderRoute::LocalTest => "local-test",
    }
}

#[cfg(test)]
mod tests {
    use ownai_core::{
        ItemKind, Language, ProjectedFile, ProjectedItem, ProjectionMode, SourceSpan,
    };

    use super::*;

    fn path(value: &str) -> RepoPath {
        RepoPath::new(value).expect("valid path")
    }

    fn projected_item(stable_key: &str, text: &str) -> ProjectedItem {
        ProjectedItem {
            stable_key: stable_key.to_owned(),
            parent_key: None,
            kind: ItemKind::Function,
            name: stable_key.to_owned(),
            span: SourceSpan {
                start_byte: 0,
                end_byte: 0,
                start_line: 0,
                start_column: 0,
                end_line: 0,
                end_column: 0,
            },
            canonical_text: text.to_owned(),
        }
    }

    fn file(items: &[(&str, &str)]) -> Option<ProjectedFile> {
        (!items.is_empty()).then(|| {
            ProjectedFile::new(
                path("src/lib.rs"),
                Language::Rust,
                items
                    .iter()
                    .map(|(key, text)| projected_item(key, text))
                    .collect(),
            )
        })
    }

    fn file_diff(path_value: &str, old: &[(&str, &str)], new: &[(&str, &str)]) -> FileDiff {
        FileDiff {
            path: path(path_value),
            mode: ProjectionMode::Signatures,
            old: file(old),
            new: file(new),
        }
    }

    fn plan(route: ProviderRoute, host: Option<&str>, model: Option<&str>) -> ReviewPlan {
        ReviewPlan {
            provider: "typesafe".to_owned(),
            route,
            host: host.map(str::to_owned),
            model: model.map(str::to_owned),
            units: 3,
            state_bytes: 512,
            skipped: 1,
            paths: vec![path("src/a.rs"), path("src/b.rs")],
        }
    }

    #[test]
    fn endpoint_host_accepts_http_and_bracketed_hosts() {
        assert_eq!(endpoint_host("http://host").expect("host"), "host");
        assert_eq!(endpoint_host("https://host:8443").expect("host"), "host");
        assert_eq!(endpoint_host("https://[::1]:8080").expect("host"), "::1");
        assert_eq!(
            endpoint_host("http://127.0.0.1:8080").expect("host"),
            "127.0.0.1"
        );
        assert_eq!(
            endpoint_host("https://api.example.com/v1?x=1#f").expect("host"),
            "api.example.com"
        );
    }

    #[test]
    fn endpoint_host_rejects_bad_schemes_credentials_and_missing_hosts() {
        for url in ["ftp://host", "host"] {
            assert!(
                matches!(
                    endpoint_host(url),
                    Err(EndpointError::UnsupportedScheme { .. })
                ),
                "{url} must be an unsupported scheme"
            );
        }
        for url in ["https://user:pass@host", "https://user@host"] {
            assert!(
                matches!(
                    endpoint_host(url),
                    Err(EndpointError::EmbeddedCredentials { .. })
                ),
                "{url} must reject embedded credentials"
            );
        }
        for url in ["https://", "https://:8080"] {
            assert!(
                matches!(endpoint_host(url), Err(EndpointError::MissingHost { .. })),
                "{url} must report a missing host"
            );
        }
    }

    #[test]
    fn check_disclosure_truth_table() {
        let refused = check_disclosure("typesafe", ProviderRoute::Remote, false, false)
            .expect_err("a non-interactive remote review without acknowledgement must fail");
        assert!(refused.to_string().contains("typesafe"), "{refused}");
        assert!(refused.to_string().contains("acknowledg"), "{refused}");

        assert!(check_disclosure("typesafe", ProviderRoute::Remote, true, false).is_ok());
        assert!(check_disclosure("typesafe", ProviderRoute::Remote, false, true).is_ok());
        assert!(check_disclosure("laya", ProviderRoute::Local, false, false).is_ok());
        assert!(check_disclosure("fake", ProviderRoute::LocalTest, false, false).is_ok());
    }

    #[test]
    fn remote_dry_run_is_exact() {
        let rendered = render_plan(
            &plan(
                ProviderRoute::Remote,
                Some("api.example.com"),
                Some("jev-2024-06"),
            ),
            true,
        );
        let expected = concat!(
            "ownai review dry run\n",
            "provider: typesafe\n",
            "route: remote\n",
            "host: api.example.com\n",
            "model: jev-2024-06\n",
            "units: 3\n",
            "serialized-state-bytes: 512\n",
            "skipped: 1\n",
            "paths:\n",
            "  src/a.rs\n",
            "  src/b.rs\n",
            "no request was sent\n",
        );
        assert_eq!(rendered, expected);
    }

    #[test]
    fn remote_disclosure_defaults_the_model_and_ends_with_one_newline() {
        let rendered = render_plan(
            &plan(ProviderRoute::Remote, Some("api.example.com"), None),
            false,
        );
        let expected = concat!(
            "ownai review disclosure\n",
            "provider: typesafe\n",
            "route: remote\n",
            "host: api.example.com\n",
            "model: (provider default)\n",
            "units: 3\n",
            "serialized-state-bytes: 512\n",
        );
        assert_eq!(rendered, expected);
        assert!(rendered.ends_with('\n'));
        assert!(!rendered.ends_with("\n\n"));
        assert!(!rendered.contains('\r'));
        assert!(!rendered.contains('\u{1b}'));
    }

    #[test]
    fn remote_disclosure_defaults_a_missing_host() {
        let rendered = render_plan(&plan(ProviderRoute::Remote, None, Some("m")), false);
        assert!(
            rendered.contains("host: (provider default)\n"),
            "{rendered}"
        );
    }

    #[test]
    fn local_disclosure_names_the_machine_boundary() {
        let mut local = plan(ProviderRoute::Local, None, None);
        local.provider = "laya".to_owned();
        let rendered = render_plan(&local, false);
        let expected = concat!(
            "ownai review disclosure\n",
            "provider: laya\n",
            "route: local\n",
            "model: (provider default)\n",
            "units: 3\n",
            "serialized-state-bytes: 512\n",
            "state stays on this machine\n",
        );
        assert_eq!(rendered, expected);
    }

    #[test]
    fn local_test_disclosure_names_the_offline_boundary() {
        let mut fake = plan(ProviderRoute::LocalTest, None, None);
        fake.provider = "fake".to_owned();
        let rendered = render_plan(&fake, false);
        let expected = concat!(
            "ownai review disclosure\n",
            "provider: fake\n",
            "route: local-test\n",
            "model: (provider default)\n",
            "units: 3\n",
            "serialized-state-bytes: 512\n",
            "no network access\n",
        );
        assert_eq!(rendered, expected);
    }

    #[test]
    fn dry_run_with_no_paths_says_none() {
        let mut empty = plan(ProviderRoute::Remote, Some("h"), None);
        empty.paths.clear();
        let rendered = render_plan(&empty, true);
        assert!(rendered.contains("paths:\n  (none)\n"), "{rendered}");
        assert!(rendered.ends_with("no request was sent\n"));
    }

    #[test]
    fn review_plan_counts_eligible_and_skipped_units_with_ascending_paths() {
        let big = "x".repeat(10_000);
        let diffs = vec![
            file_diff("src/c.rs", &[], &[("function c", "fn c();")]),
            file_diff(
                "src/a.rs",
                &[("function a1", "fn a1();"), ("function a2", "fn a2();")],
                &[("function a1", "fn a1(u8);"), ("function a2", "fn a2(u8);")],
            ),
            file_diff("src/b.rs", &[], &[("struct Big", big.as_str())]),
        ];
        let lens = ReviewLens::new(4096).expect("valid lens");

        let plan = review_plan(
            "typesafe",
            ProviderRoute::Remote,
            Some("api.example.com"),
            Some("jev-2024-06"),
            &lens,
            &diffs,
        );

        assert_eq!(plan.provider, "typesafe");
        assert_eq!(plan.route, ProviderRoute::Remote);
        assert_eq!(plan.host.as_deref(), Some("api.example.com"));
        assert_eq!(plan.model.as_deref(), Some("jev-2024-06"));
        assert_eq!(plan.units, 3);
        assert_eq!(plan.skipped, 1);
        assert_eq!(plan.paths, vec![path("src/a.rs"), path("src/c.rs")]);

        let expected_bytes: usize = diffs
            .iter()
            .flat_map(FileDiff::item_diffs)
            .map(|item| ReviewLens::state_bytes(&item))
            .filter(|bytes| *bytes <= lens.max_state_bytes())
            .sum();
        assert_eq!(plan.state_bytes, expected_bytes);
        assert!(plan.state_bytes < big.len());
    }
}
