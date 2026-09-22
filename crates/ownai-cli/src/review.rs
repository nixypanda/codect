//! Plain-text rendering of semantic review results.
//!
//! The document has three parts in a fixed order: a header that attributes the
//! judgments to a provider and model revision, the underlying canonical diff
//! verbatim, and one annotation block per outcome in the deterministic order
//! the lens produced them. This module renders only; it never calls a provider,
//! reads configuration, or applies ANSI styling (that lives in `output.rs`, so
//! `--color=never` stays byte-for-byte plain).

use std::collections::BTreeSet;
use std::fmt::Write as _;

use ownai_engine::{
    ChoiceJudgment, ItemChangeKind, ItemDiff, REVIEW_STATE_SCHEMA, ReviewError, ReviewOutcome,
    ReviewedItemDiff,
};

/// Default choice confidence threshold for the built-in review lens.
///
/// A concern below this threshold renders as `uncertain` with its leading
/// candidates instead of a single selected value.
pub const DEFAULT_CHOICE_CONFIDENCE_THRESHOLD: f64 = 0.55;

/// Rendering options for the review document.
pub struct ReviewRenderOptions {
    /// The confidence at or above which a choice is rendered as confident.
    pub choice_confidence_threshold: f64,
}

impl Default for ReviewRenderOptions {
    fn default() -> Self {
        Self {
            choice_confidence_threshold: DEFAULT_CHOICE_CONFIDENCE_THRESHOLD,
        }
    }
}

/// Renders the complete review document: a header, the underlying canonical
/// diff verbatim, and one annotation per outcome in order.
///
/// The result is deterministic plain text with no escape bytes. Every section is
/// separated by exactly one blank line and the document ends with exactly one
/// trailing newline.
pub fn review_document(
    provider: &str,
    canonical_diff: &str,
    outcomes: &[ReviewOutcome],
    options: &ReviewRenderOptions,
) -> String {
    let mut document = String::new();

    let _ = writeln!(document, "ownai review");
    let _ = writeln!(document, "provider: {provider}");
    let _ = writeln!(document, "model-revision: {}", returned_revisions(outcomes));
    let _ = writeln!(document, "lens: review");
    let _ = writeln!(document, "state-schema: {REVIEW_STATE_SCHEMA}");

    let counts = OutcomeCounts::of(outcomes);
    let _ = writeln!(
        document,
        "units: {} reviewed, {} skipped, {} failed",
        counts.reviewed, counts.skipped, counts.failed
    );

    if !canonical_diff.is_empty() {
        document.push('\n');
        document.push_str(canonical_diff);
    }

    for outcome in outcomes {
        document.push('\n');
        match outcome {
            ReviewOutcome::Reviewed(reviewed) => render_reviewed(&mut document, reviewed, options),
            ReviewOutcome::Failed { item, error } => render_failed(&mut document, item, error),
        }
    }

    document
}

/// The distinct, ascending `model_revision` values across successful outcomes.
///
/// `unknown` is used when no outcome was reviewed, so the header never claims a
/// revision that was not returned.
fn returned_revisions(outcomes: &[ReviewOutcome]) -> String {
    let revisions: BTreeSet<&str> = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            ReviewOutcome::Reviewed(reviewed) => Some(reviewed.model_revision.as_str()),
            ReviewOutcome::Failed { .. } => None,
        })
        .collect();

    if revisions.is_empty() {
        "unknown".to_owned()
    } else {
        revisions.into_iter().collect::<Vec<_>>().join(", ")
    }
}

/// Reviewed, skipped, and failed counts for the `units:` header line.
///
/// A `StateTooLarge` failure is a skip, not a provider failure, because no
/// provider call was made.
struct OutcomeCounts {
    reviewed: usize,
    skipped: usize,
    failed: usize,
}

impl OutcomeCounts {
    fn of(outcomes: &[ReviewOutcome]) -> Self {
        let mut counts = Self {
            reviewed: 0,
            skipped: 0,
            failed: 0,
        };
        for outcome in outcomes {
            match outcome {
                ReviewOutcome::Reviewed(_) => counts.reviewed += 1,
                ReviewOutcome::Failed {
                    error: ReviewError::StateTooLarge { .. },
                    ..
                } => counts.skipped += 1,
                ReviewOutcome::Failed { .. } => counts.failed += 1,
            }
        }
        counts
    }
}

/// Writes one `Reviewed` annotation block, ending with a newline.
fn render_reviewed(out: &mut String, reviewed: &ReviewedItemDiff, options: &ReviewRenderOptions) {
    let item = &reviewed.item;
    let judgment = &reviewed.judgment;

    let _ = writeln!(out, "{} :: {}", item.path, item.stable_key);
    let _ = writeln!(out, "  change: {}", change_key(item.change));
    let _ = writeln!(
        out,
        "  concern: {}",
        concern_text(&judgment.concern, options)
    );
    let _ = writeln!(
        out,
        "  risk: {:.1}/5 (confidence {:.2})",
        judgment.risk.display_value,
        judgment.risk.confidence.get()
    );
    let _ = writeln!(
        out,
        "  likely-breaking: {:.2}",
        judgment.likely_breaking.get()
    );
    let _ = writeln!(out, "  needs-tests: {:.2}", judgment.needs_tests.get());
    let _ = writeln!(out, "  needs-docs: {:.2}", judgment.needs_docs.get());
    let _ = writeln!(
        out,
        "  needs-migration: {:.2}",
        judgment.needs_migration.get()
    );
    let _ = writeln!(
        out,
        "  security-sensitive: {:.2}",
        judgment.security_sensitive.get()
    );
}

/// Writes one `Failed` annotation block, ending with a newline.
fn render_failed(out: &mut String, item: &ItemDiff, error: &ReviewError) {
    let _ = writeln!(out, "{} :: {}", item.path, item.stable_key);
    let _ = writeln!(out, "  {}", failure_reason(error));
}

/// The stable lowercase kebab-case key for a change kind, matching the state
/// schema's `change` field.
fn change_key(change: ItemChangeKind) -> &'static str {
    match change {
        ItemChangeKind::Added => "added",
        ItemChangeKind::Removed => "removed",
        ItemChangeKind::Modified => "modified",
    }
}

/// Renders the concern field: the selected value when confident, otherwise the
/// two leading candidates in probability order.
fn concern_text(concern: &ChoiceJudgment, options: &ReviewRenderOptions) -> String {
    if concern.confidence.get() >= options.choice_confidence_threshold {
        return format!(
            "{} (confidence {:.2})",
            concern.selected,
            concern.confidence.get()
        );
    }

    // The distribution is guaranteed to hold at least two options for a choice
    // question, so the two leading candidates always exist.
    let mut candidates: Vec<(&str, f64)> = concern
        .probabilities
        .iter()
        .map(|(key, probability)| (key.as_str(), probability.get()))
        .collect();
    candidates.sort_by(|left, right| right.1.total_cmp(&left.1).then_with(|| left.0.cmp(right.0)));

    let (leading_key, leading_probability) = candidates[0];
    let (runner_up_key, runner_up_probability) = candidates[1];
    format!(
        "uncertain (leading: {leading_key} {leading_probability:.2}, \
         {runner_up_key} {runner_up_probability:.2})"
    )
}

/// The failure reason, derived from the typed error rather than a parsed
/// message.
fn failure_reason(error: &ReviewError) -> String {
    match error {
        ReviewError::StateTooLarge {
            actual_bytes,
            limit_bytes,
            ..
        } => format!(
            "skipped: state is {actual_bytes} bytes, exceeding the {limit_bytes}-byte limit"
        ),
        ReviewError::Provider { source, .. } => format!("failed: {source}"),
        ReviewError::InvalidResponse { source, .. } => {
            format!("failed: invalid response: {source}")
        }
        ReviewError::InvalidStateLimit { limit_bytes } => {
            format!("failed: invalid state limit {limit_bytes}")
        }
        ReviewError::InvalidQuestions { source } => {
            format!("failed: invalid review questions: {source}")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ownai_core::{ItemKind, Language, ProjectionMode, RepoPath};
    use ownai_decisions::{DecisionError, Probability, ValidationError};
    use ownai_engine::{ChoiceJudgment, ItemDiff, ReviewJudgment, ReviewedItemDiff, ScoreJudgment};

    use super::*;

    /// The canonical diff embedded verbatim by every document fixture.
    const CANONICAL_DIFF: &str = concat!(
        "diff --ownai a/src/auth.rs b/src/auth.rs\n",
        "--- a/src/auth.rs\n",
        "+++ b/src/auth.rs\n",
        "@@ -1 +1 @@\n",
        "-fn refresh(&mut self, token: Token) -> Result<(), Error>;\n",
        "+async fn refresh(&mut self, token: RefreshToken) -> Result<Session, Error>;\n",
    );

    fn prob(value: f64) -> Probability {
        Probability::new(value).expect("valid probability")
    }

    fn path(value: &str) -> RepoPath {
        RepoPath::new(value).expect("valid path")
    }

    fn item(path_value: &str, stable_key: &str, change: ItemChangeKind) -> ItemDiff {
        ItemDiff {
            path: path(path_value),
            language: Language::Rust,
            mode: ProjectionMode::Signatures,
            change,
            stable_key: stable_key.to_owned(),
            item_kind: ItemKind::Function,
            name: stable_key.to_owned(),
            old_text: Some("old\n".to_owned()),
            new_text: Some("new\n".to_owned()),
        }
    }

    fn choice(selected: &str, pairs: &[(&str, f64)], confidence: f64) -> ChoiceJudgment {
        let probabilities: BTreeMap<String, Probability> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), prob(*value)))
            .collect();
        ChoiceJudgment {
            selected: selected.to_owned(),
            probabilities,
            confidence: prob(confidence),
        }
    }

    fn score(display_value: f64, confidence: f64) -> ScoreJudgment {
        ScoreJudgment {
            raw_index: display_value - 1.0,
            display_value,
            probabilities: BTreeMap::new(),
            confidence: prob(confidence),
        }
    }

    fn judgment(concern: ChoiceJudgment, risk: ScoreJudgment) -> ReviewJudgment {
        ReviewJudgment {
            concern,
            risk,
            likely_breaking: prob(0.71),
            needs_tests: prob(0.91),
            needs_docs: prob(0.43),
            needs_migration: prob(0.66),
            security_sensitive: prob(0.86),
        }
    }

    fn reviewed(
        item: ItemDiff,
        concern: ChoiceJudgment,
        risk: ScoreJudgment,
        revision: &str,
    ) -> ReviewOutcome {
        ReviewOutcome::Reviewed(ReviewedItemDiff {
            item,
            judgment: judgment(concern, risk),
            provider: "fake".to_owned(),
            model_revision: revision.to_owned(),
            usage: None,
        })
    }

    fn failed(item: ItemDiff, error: ReviewError) -> ReviewOutcome {
        ReviewOutcome::Failed { item, error }
    }

    fn confident_outcome() -> ReviewOutcome {
        reviewed(
            item(
                "src/auth.rs",
                "impl Session::refresh",
                ItemChangeKind::Modified,
            ),
            choice(
                "authentication-authorization",
                &[
                    ("api-contract", 0.05),
                    ("authentication-authorization", 0.90),
                    ("other", 0.05),
                ],
                0.88,
            ),
            score(3.6, 0.74),
            "jev-2024-06",
        )
    }

    fn uncertain_outcome() -> ReviewOutcome {
        reviewed(
            item("src/lib.rs", "function helper", ItemChangeKind::Added),
            choice(
                "api-contract",
                &[
                    ("api-contract", 0.40),
                    ("testing-tooling", 0.30),
                    ("other", 0.30),
                ],
                0.40,
            ),
            score(2.5, 0.60),
            "laya-v1",
        )
    }

    fn skipped_outcome() -> ReviewOutcome {
        failed(
            item("src/big.rs", "struct Big", ItemChangeKind::Removed),
            ReviewError::StateTooLarge {
                path: path("src/big.rs"),
                stable_key: "struct Big".to_owned(),
                actual_bytes: 2048,
                limit_bytes: 1024,
            },
        )
    }

    fn provider_failed_outcome() -> ReviewOutcome {
        failed(
            item("src/net.rs", "function fetch", ItemChangeKind::Modified),
            ReviewError::Provider {
                path: path("src/net.rs"),
                stable_key: "function fetch".to_owned(),
                source: DecisionError::Provider {
                    message: "boom".to_owned(),
                },
            },
        )
    }

    #[test]
    fn complete_document_is_exact() {
        let outcomes = vec![
            confident_outcome(),
            uncertain_outcome(),
            skipped_outcome(),
            provider_failed_outcome(),
        ];

        let document = review_document(
            "fake",
            CANONICAL_DIFF,
            &outcomes,
            &ReviewRenderOptions::default(),
        );

        let expected = concat!(
            "ownai review\n",
            "provider: fake\n",
            "model-revision: jev-2024-06, laya-v1\n",
            "lens: review\n",
            "state-schema: ownai.review-unit.v1\n",
            "units: 2 reviewed, 1 skipped, 1 failed\n",
            "\n",
            "diff --ownai a/src/auth.rs b/src/auth.rs\n",
            "--- a/src/auth.rs\n",
            "+++ b/src/auth.rs\n",
            "@@ -1 +1 @@\n",
            "-fn refresh(&mut self, token: Token) -> Result<(), Error>;\n",
            "+async fn refresh(&mut self, token: RefreshToken) -> Result<Session, Error>;\n",
            "\n",
            "src/auth.rs :: impl Session::refresh\n",
            "  change: modified\n",
            "  concern: authentication-authorization (confidence 0.88)\n",
            "  risk: 3.6/5 (confidence 0.74)\n",
            "  likely-breaking: 0.71\n",
            "  needs-tests: 0.91\n",
            "  needs-docs: 0.43\n",
            "  needs-migration: 0.66\n",
            "  security-sensitive: 0.86\n",
            "\n",
            "src/lib.rs :: function helper\n",
            "  change: added\n",
            "  concern: uncertain (leading: api-contract 0.40, other 0.30)\n",
            "  risk: 2.5/5 (confidence 0.60)\n",
            "  likely-breaking: 0.71\n",
            "  needs-tests: 0.91\n",
            "  needs-docs: 0.43\n",
            "  needs-migration: 0.66\n",
            "  security-sensitive: 0.86\n",
            "\n",
            "src/big.rs :: struct Big\n",
            "  skipped: state is 2048 bytes, exceeding the 1024-byte limit\n",
            "\n",
            "src/net.rs :: function fetch\n",
            "  failed: decision provider failed: boom\n",
        );
        assert_eq!(document, expected);
    }

    #[test]
    fn document_has_exactly_one_blank_line_between_sections_and_one_trailing_newline() {
        let outcomes = vec![confident_outcome(), skipped_outcome()];
        let document = review_document(
            "fake",
            CANONICAL_DIFF,
            &outcomes,
            &ReviewRenderOptions::default(),
        );

        assert!(document.ends_with('\n'));
        assert!(!document.ends_with("\n\n"));
        assert!(!document.contains("\n\n\n"));
    }

    #[test]
    fn empty_diff_omits_the_diff_section() {
        let document = review_document(
            "fake",
            "",
            &[confident_outcome()],
            &ReviewRenderOptions::default(),
        );

        assert!(document.starts_with("ownai review\n"));
        assert!(!document.contains("diff --ownai"));
        assert!(document.contains("\n\nsrc/auth.rs :: impl Session::refresh\n"));
    }

    #[test]
    fn header_shows_sorted_distinct_revisions_and_unknown_without_reviewed_outcomes() {
        let document = review_document(
            "fake",
            "",
            &[uncertain_outcome(), confident_outcome()],
            &ReviewRenderOptions::default(),
        );
        assert!(document.contains("model-revision: jev-2024-06, laya-v1\n"));

        let only_failures = review_document(
            "fake",
            "",
            &[skipped_outcome(), provider_failed_outcome()],
            &ReviewRenderOptions::default(),
        );
        assert!(only_failures.contains("model-revision: unknown\n"));
    }

    #[test]
    fn units_line_counts_mixed_outcomes() {
        let outcomes = vec![
            confident_outcome(),
            uncertain_outcome(),
            reviewed(
                item("src/a.rs", "function a", ItemChangeKind::Added),
                choice("other", &[("other", 0.8), ("api-contract", 0.2)], 0.8),
                score(1.0, 0.9),
                "jev-2024-06",
            ),
            skipped_outcome(),
            provider_failed_outcome(),
        ];
        let document = review_document("fake", "", &outcomes, &ReviewRenderOptions::default());
        assert!(document.contains("units: 3 reviewed, 1 skipped, 1 failed\n"));
    }

    #[test]
    fn probabilities_use_two_decimals_and_risk_uses_one_decimal() {
        let outcome = reviewed(
            item("src/a.rs", "function a", ItemChangeKind::Modified),
            choice(
                "api-contract",
                &[("api-contract", 0.7), ("other", 0.3)],
                0.7,
            ),
            score(4.0, 0.755),
            "rev",
        );
        let document = review_document("fake", "", &[outcome], &ReviewRenderOptions::default());

        assert!(document.contains("  concern: api-contract (confidence 0.70)\n"));
        assert!(document.contains("  risk: 4.0/5 (confidence 0.76)\n"));
        assert!(document.contains("  likely-breaking: 0.71\n"));
        assert!(document.contains("  needs-tests: 0.91\n"));
        assert!(document.contains("  needs-docs: 0.43\n"));
        assert!(document.contains("  needs-migration: 0.66\n"));
        assert!(document.contains("  security-sensitive: 0.86\n"));
    }

    #[test]
    fn uncertain_concern_lists_the_two_leading_candidates_in_probability_order() {
        let concern = choice(
            "a",
            &[("only-top", 0.5), ("runner-up", 0.3), ("last", 0.2)],
            0.4,
        );
        let text = concern_text(&concern, &ReviewRenderOptions::default());
        assert_eq!(text, "uncertain (leading: only-top 0.50, runner-up 0.30)");
    }

    #[test]
    fn uncertain_breaks_probability_ties_by_ascending_key() {
        let concern = choice("z", &[("z", 0.5), ("alpha", 0.25), ("beta", 0.25)], 0.3);
        let text = concern_text(&concern, &ReviewRenderOptions::default());
        assert_eq!(text, "uncertain (leading: z 0.50, alpha 0.25)");
    }

    #[test]
    fn confidence_at_the_threshold_is_confident() {
        let options = ReviewRenderOptions {
            choice_confidence_threshold: 0.55,
        };
        let at_threshold = choice(
            "api-contract",
            &[("api-contract", 0.8), ("other", 0.2)],
            0.55,
        );
        assert_eq!(
            concern_text(&at_threshold, &options),
            "api-contract (confidence 0.55)"
        );

        let below = choice(
            "api-contract",
            &[("api-contract", 0.8), ("other", 0.2)],
            0.54,
        );
        assert_eq!(
            concern_text(&below, &options),
            "uncertain (leading: api-contract 0.80, other 0.20)"
        );
    }

    #[test]
    fn default_options_use_the_published_threshold() {
        assert_eq!(
            ReviewRenderOptions::default().choice_confidence_threshold,
            DEFAULT_CHOICE_CONFIDENCE_THRESHOLD
        );
    }

    #[test]
    fn every_error_variant_renders_its_typed_reason() {
        let cases = [
            (
                ReviewError::StateTooLarge {
                    path: path("src/a.rs"),
                    stable_key: "function a".to_owned(),
                    actual_bytes: 12,
                    limit_bytes: 10,
                },
                "skipped: state is 12 bytes, exceeding the 10-byte limit",
            ),
            (
                ReviewError::Provider {
                    path: path("src/a.rs"),
                    stable_key: "function a".to_owned(),
                    source: DecisionError::Transport {
                        message: "reset".to_owned(),
                    },
                },
                "failed: decision provider transport failed: reset",
            ),
            (
                ReviewError::InvalidResponse {
                    path: path("src/a.rs"),
                    stable_key: "function a".to_owned(),
                    source: ValidationError::MissingAnswer {
                        id: "concern".to_owned(),
                    },
                },
                "failed: invalid response: response is missing answer `concern`",
            ),
            (
                ReviewError::InvalidResponse {
                    path: path("src/a.rs"),
                    stable_key: "function a".to_owned(),
                    source: ValidationError::WrongAnswerType {
                        id: "concern".to_owned(),
                        expected: "choice",
                        actual: "noul",
                    },
                },
                "failed: invalid response: answer `concern` has type noul, expected choice",
            ),
            (
                ReviewError::InvalidStateLimit { limit_bytes: 0 },
                "failed: invalid state limit 0",
            ),
            (
                ReviewError::InvalidQuestions {
                    source: ValidationError::Empty { field: "questions" },
                },
                "failed: invalid review questions: questions must not be empty",
            ),
        ];

        for (error, expected) in &cases {
            assert_eq!(&failure_reason(error), expected);
        }
    }

    #[test]
    fn annotations_follow_input_order() {
        let outcomes = vec![
            failed(
                item("src/a.rs", "function a", ItemChangeKind::Added),
                ReviewError::InvalidStateLimit { limit_bytes: 0 },
            ),
            failed(
                item("src/b.rs", "function b", ItemChangeKind::Added),
                ReviewError::InvalidStateLimit { limit_bytes: 0 },
            ),
            failed(
                item("src/c.rs", "function c", ItemChangeKind::Added),
                ReviewError::InvalidStateLimit { limit_bytes: 0 },
            ),
        ];
        let document = review_document("fake", "", &outcomes, &ReviewRenderOptions::default());

        let a = document.find("src/a.rs :: function a").expect("a");
        let b = document.find("src/b.rs :: function b").expect("b");
        let c = document.find("src/c.rs :: function c").expect("c");
        assert!(a < b && b < c, "annotation order must be input order");
    }

    #[test]
    fn document_contains_no_escape_bytes() {
        let outcomes = vec![
            confident_outcome(),
            uncertain_outcome(),
            skipped_outcome(),
            provider_failed_outcome(),
        ];
        let document = review_document(
            "fake",
            CANONICAL_DIFF,
            &outcomes,
            &ReviewRenderOptions::default(),
        );
        assert!(!document.contains('\u{1b}'));
        assert!(!document.contains('\r'));
    }
}
