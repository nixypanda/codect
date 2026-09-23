//! The offline semantic-lens evaluation harness.
//!
//! A versioned JSON corpus describes synthetic declaration diffs and their
//! human labels. This module loads that corpus, reconstructs one [`ItemDiff`]
//! per case, reviews it through the lens with any [`DecisionProvider`], and
//! aggregates provider-independent metrics. It returns data only: rendering the
//! report belongs to the caller.
//!
//! The harness never performs I/O of its own and never opens a network
//! connection. A caller supplies the corpus text and the provider, so normal
//! tests can exercise the whole pipeline with the deterministic fake provider.

use std::collections::BTreeMap;
use std::time::Instant;

use ownai_core::{ItemKind, Language, ProjectionMode, RepoPath};
use ownai_decisions::DecisionProvider;

use crate::item_diff::{ItemChangeKind, ItemDiff};
use crate::review::{ReviewJudgment, ReviewLens};

/// The versioned schema identifier for one evaluation fixture file.
pub const REVIEW_EVAL_SCHEMA: &str = "ownai.review-eval.v1";

/// The generous serialized-state limit used during evaluation. It admits every
/// realistic fixture; `ReviewLens::new` rejects only zero.
const EVAL_STATE_LIMIT: usize = 1 << 20;

/// One synthetic declaration diff with optional human labels.
///
/// `before` and `after` are JSON `null` on the absent side of an addition or
/// removal, mirroring the review state encoding.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvalCase {
    pub id: String,
    pub path: String,
    pub language: String,
    pub mode: String,
    pub change: String,
    pub item_kind: String,
    pub stable_key: String,
    pub name: String,
    pub before: Option<String>,
    pub after: Option<String>,
    #[serde(default)]
    pub labels: Option<EvalLabels>,
    #[serde(default)]
    pub ambiguous: bool,
}

/// The human labels for one case, one per fixed review question.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvalLabels {
    pub concern: String,
    pub risk: f64,
    pub likely_breaking: bool,
    pub needs_tests: bool,
    pub needs_docs: bool,
    pub needs_migration: bool,
    pub security_sensitive: bool,
}

/// The top-level fixture file shape. Kept private so only [`load_cases`] is the
/// entry point.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EvalFile {
    schema: String,
    cases: Vec<EvalCase>,
}

/// A typed failure while loading an evaluation corpus.
///
/// `serde_json::Error` is not `Clone`, so this type deliberately does not
/// derive it even though the other public evaluation types do.
#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    /// The file declared a schema this build does not understand.
    #[error("unsupported evaluation schema `{schema}`; expected `{expected}`")]
    UnknownSchema {
        schema: String,
        expected: &'static str,
    },

    /// One case violated the corpus contract.
    #[error("invalid evaluation case `{id}`: {message}")]
    InvalidCase { id: String, message: String },

    /// The text was not valid JSON for the fixture shape.
    #[error("the evaluation fixture file is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// Tunable evaluation parameters.
#[derive(Clone, Copy, Debug)]
pub struct EvalOptions {
    /// The minimum choice confidence counted as covered by the gate.
    pub choice_confidence_threshold: f64,
}

/// The aggregate metrics for one set of cases.
///
/// Every ratio is `0.0` when its denominator is zero. `labeled` counts only
/// cases that carried labels *and* were reviewed successfully, so a failed or
/// ambiguous case never dilutes accuracy.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Metrics {
    pub cases: usize,
    pub labeled: usize,
    pub ambiguous: usize,
    pub failed: usize,
    pub choice_correct: usize,
    pub choice_accuracy: f64,
    pub covered: usize,
    pub coverage: f64,
    pub covered_accuracy: f64,
    pub noul_brier: f64,
    pub noul_calibration_error: f64,
    pub score_mae: f64,
    pub input_bytes: usize,
    pub total_latency_ms: f64,
    pub mean_latency_ms: f64,
}

/// One named metric group.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupMetrics {
    pub name: String,
    pub metrics: Metrics,
}

/// The complete evaluation result, overall plus three groupings.
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluationReport {
    pub overall: Metrics,
    pub by_language: Vec<GroupMetrics>,
    pub by_change: Vec<GroupMetrics>,
    pub by_item_kind: Vec<GroupMetrics>,
}

/// Parses and validates a fixture file into its cases.
///
/// Unknown fields, an unknown schema, and a case that has neither labels nor
/// `ambiguous == true` (or both) are rejected before any case can be evaluated.
pub fn load_cases(json: &str) -> Result<Vec<EvalCase>, EvalError> {
    let file: EvalFile = serde_json::from_str(json)?;
    if file.schema != REVIEW_EVAL_SCHEMA {
        return Err(EvalError::UnknownSchema {
            schema: file.schema,
            expected: REVIEW_EVAL_SCHEMA,
        });
    }
    for case in &file.cases {
        validate_case(case)?;
    }
    Ok(file.cases)
}

/// Reconstructs and reviews every case, then aggregates the metrics.
///
/// The provider is called exactly once per case, in corpus order, so a scripted
/// provider observes one request per case. Cases whose enum keys cannot be
/// reconstructed count as failures rather than panicking.
pub fn evaluate(
    provider: &dyn DecisionProvider,
    cases: &[EvalCase],
    options: EvalOptions,
) -> EvaluationReport {
    let lens = ReviewLens::new(EVAL_STATE_LIMIT).expect("the evaluation state limit is non-zero");
    let mut outcomes = Vec::with_capacity(cases.len());

    for case in cases {
        let mut outcome = CaseOutcome {
            language: case.language.clone(),
            change: case.change.clone(),
            item_kind: case.item_kind.clone(),
            labels: case.labels.clone(),
            ambiguous: case.ambiguous,
            failed: false,
            input_bytes: 0,
            latency_ms: 0.0,
            scored: None,
        };

        let item = match build_item(case) {
            Ok(item) => item,
            Err(_) => {
                outcome.failed = true;
                outcomes.push(outcome);
                continue;
            }
        };

        outcome.input_bytes = ReviewLens::state_bytes(&item);
        let start = Instant::now();
        let result = lens.review_item(provider, item);
        outcome.latency_ms = start.elapsed().as_secs_f64() * 1000.0;
        match result {
            Ok(reviewed) => outcome.scored = Some(scored_case(&reviewed.judgment)),
            Err(_) => outcome.failed = true,
        }
        outcomes.push(outcome);
    }

    let all: Vec<&CaseOutcome> = outcomes.iter().collect();
    EvaluationReport {
        overall: metrics_for(&all, options),
        by_language: group_by(&outcomes, options, |case| case.language.clone()),
        by_change: group_by(&outcomes, options, |case| case.change.clone()),
        by_item_kind: group_by(&outcomes, options, |case| case.item_kind.clone()),
    }
}

/// One case's raw grouping keys plus the values the metrics need.
struct CaseOutcome {
    language: String,
    change: String,
    item_kind: String,
    labels: Option<EvalLabels>,
    ambiguous: bool,
    failed: bool,
    input_bytes: usize,
    latency_ms: f64,
    scored: Option<ScoredCase>,
}

/// The judgment-derived values used by the labeled metrics.
struct ScoredCase {
    concern_selected: String,
    concern_confidence: f64,
    noul_probabilities: [f64; 5],
    risk_raw: f64,
}

fn scored_case(judgment: &ReviewJudgment) -> ScoredCase {
    ScoredCase {
        concern_selected: judgment.concern.selected.clone(),
        concern_confidence: judgment.concern.confidence.get(),
        noul_probabilities: [
            judgment.likely_breaking.get(),
            judgment.needs_tests.get(),
            judgment.needs_docs.get(),
            judgment.needs_migration.get(),
            judgment.security_sensitive.get(),
        ],
        risk_raw: judgment.risk.raw_index,
    }
}

fn build_item(case: &EvalCase) -> Result<ItemDiff, String> {
    let path = RepoPath::new(case.path.as_bytes()).map_err(|error| error.to_string())?;
    let language = language_from_key(&case.language)
        .ok_or_else(|| format!("unknown language key `{}`", case.language))?;
    let mode = projection_mode_from_key(&case.mode)
        .ok_or_else(|| format!("unknown projection mode key `{}`", case.mode))?;
    let change = change_from_key(&case.change)
        .ok_or_else(|| format!("unknown change key `{}`", case.change))?;
    let item_kind = item_kind_from_key(&case.item_kind)
        .ok_or_else(|| format!("unknown item kind key `{}`", case.item_kind))?;

    Ok(ItemDiff {
        path,
        language,
        mode,
        change,
        stable_key: case.stable_key.clone(),
        item_kind,
        name: case.name.clone(),
        old_text: case.before.clone(),
        new_text: case.after.clone(),
    })
}

fn validate_case(case: &EvalCase) -> Result<(), EvalError> {
    let invalid = |message: &str| EvalError::InvalidCase {
        id: case.id.clone(),
        message: message.to_owned(),
    };

    match (&case.labels, case.ambiguous) {
        (None, false) => Err(invalid("must set `labels` or `ambiguous` to true")),
        (Some(_), true) => Err(invalid("must not set both `labels` and `ambiguous`")),
        (Some(labels), false) => {
            if !labels.risk.is_finite() || !(0.0..=4.0).contains(&labels.risk) {
                Err(invalid("`labels.risk` must be finite and within 0.0..=4.0"))
            } else {
                Ok(())
            }
        }
        (None, true) => Ok(()),
    }
}

fn group_by(
    outcomes: &[CaseOutcome],
    options: EvalOptions,
    key: impl Fn(&CaseOutcome) -> String,
) -> Vec<GroupMetrics> {
    let mut groups: BTreeMap<String, Vec<&CaseOutcome>> = BTreeMap::new();
    for outcome in outcomes {
        groups.entry(key(outcome)).or_default().push(outcome);
    }
    groups
        .into_iter()
        .map(|(name, cases)| GroupMetrics {
            name,
            metrics: metrics_for(&cases, options),
        })
        .collect()
}

fn metrics_for(cases: &[&CaseOutcome], options: EvalOptions) -> Metrics {
    let mut metrics = Metrics::default();
    let mut brier_sum = 0.0;
    let mut brier_count = 0usize;
    let mut calibration = [(0usize, 0.0f64, 0.0f64); 10];
    let mut score_error_sum = 0.0;
    let mut score_error_count = 0usize;
    let mut covered_correct = 0usize;

    for case in cases {
        metrics.cases += 1;
        metrics.input_bytes += case.input_bytes;
        metrics.total_latency_ms += case.latency_ms;
        if case.ambiguous {
            metrics.ambiguous += 1;
        }
        if case.failed {
            metrics.failed += 1;
            continue;
        }
        let (Some(labels), Some(scored)) = (&case.labels, &case.scored) else {
            continue;
        };

        metrics.labeled += 1;
        if scored.concern_selected == labels.concern {
            metrics.choice_correct += 1;
        }
        if scored.concern_confidence >= options.choice_confidence_threshold {
            metrics.covered += 1;
            if scored.concern_selected == labels.concern {
                covered_correct += 1;
            }
        }

        let truths = [
            labels.likely_breaking,
            labels.needs_tests,
            labels.needs_docs,
            labels.needs_migration,
            labels.security_sensitive,
        ];
        for (probability, truth) in scored.noul_probabilities.iter().zip(truths) {
            let truth = if truth { 1.0 } else { 0.0 };
            brier_sum += (*probability - truth).powi(2);
            brier_count += 1;
            let bin = ((*probability * 10.0).floor() as usize).min(9);
            calibration[bin].0 += 1;
            calibration[bin].1 += *probability;
            calibration[bin].2 += truth;
        }

        score_error_sum += (scored.risk_raw - labels.risk).abs();
        score_error_count += 1;
    }

    metrics.choice_accuracy = ratio(metrics.choice_correct, metrics.labeled);
    metrics.coverage = ratio(metrics.covered, metrics.labeled);
    metrics.covered_accuracy = ratio(covered_correct, metrics.covered);
    metrics.noul_brier = mean(brier_sum, brier_count);
    metrics.noul_calibration_error = calibration_error(&calibration, brier_count);
    metrics.score_mae = mean(score_error_sum, score_error_count);
    metrics.mean_latency_ms = if metrics.cases == 0 {
        0.0
    } else {
        metrics.total_latency_ms / metrics.cases as f64
    };
    metrics
}

/// The count-weighted expected calibration error over ten equal-width bins.
fn calibration_error(bins: &[(usize, f64, f64); 10], total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let mut error = 0.0;
    for (count, sum_probability, sum_truth) in bins {
        if *count == 0 {
            continue;
        }
        let mean_probability = sum_probability / *count as f64;
        let mean_truth = sum_truth / *count as f64;
        error += (*count as f64 / total as f64) * (mean_probability - mean_truth).abs();
    }
    error
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn mean(sum: f64, count: usize) -> f64 {
    if count == 0 { 0.0 } else { sum / count as f64 }
}

/// Every inverse kebab key maps explicitly. No wildcard arm, so a new core
/// variant is a compile error until its evaluation key is chosen.
fn language_from_key(key: &str) -> Option<Language> {
    match key {
        "elm" => Some(Language::Elm),
        "haskell" => Some(Language::Haskell),
        "python" => Some(Language::Python),
        "rust" => Some(Language::Rust),
        _ => None,
    }
}

fn projection_mode_from_key(key: &str) -> Option<ProjectionMode> {
    match key {
        "types" => Some(ProjectionMode::Types),
        "signatures" => Some(ProjectionMode::Signatures),
        _ => None,
    }
}

fn item_kind_from_key(key: &str) -> Option<ItemKind> {
    match key {
        "module" => Some(ItemKind::Module),
        "type" => Some(ItemKind::Type),
        "type-alias" => Some(ItemKind::TypeAlias),
        "constructor" => Some(ItemKind::Constructor),
        "field" => Some(ItemKind::Field),
        "variant" => Some(ItemKind::Variant),
        "trait" => Some(ItemKind::Trait),
        "trait-implementation" => Some(ItemKind::TraitImplementation),
        "associated-type" => Some(ItemKind::AssociatedType),
        "type-family" => Some(ItemKind::TypeFamily),
        "pattern-synonym" => Some(ItemKind::PatternSynonym),
        "function" => Some(ItemKind::Function),
        "method" => Some(ItemKind::Method),
        "value" => Some(ItemKind::Value),
        "constant" => Some(ItemKind::Constant),
        "static" => Some(ItemKind::Static),
        "port" => Some(ItemKind::Port),
        "operator" => Some(ItemKind::Operator),
        "foreign-block" => Some(ItemKind::ForeignBlock),
        _ => None,
    }
}

fn change_from_key(key: &str) -> Option<ItemChangeKind> {
    match key {
        "added" => Some(ItemChangeKind::Added),
        "removed" => Some(ItemChangeKind::Removed),
        "modified" => Some(ItemChangeKind::Modified),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use ownai_decisions::fake::FakeProvider;
    use ownai_decisions::{
        Answer, ChoiceAnswer, DecisionError, DecisionResponse, NoulAnswer, Probability, QuestionId,
        ScoreAnswer,
    };

    use super::*;

    const CONCERN_KEYS: [&str; 10] = [
        "api-contract",
        "data-model",
        "authentication-authorization",
        "storage",
        "networking-integration",
        "configuration-operations",
        "observability",
        "user-interface",
        "testing-tooling",
        "other",
    ];
    const RISK_LABELS: [&str; 5] = ["routine", "low", "moderate", "high", "critical"];

    fn prob(value: f64) -> Probability {
        Probability::new(value).expect("valid probability")
    }

    fn qid(value: &str) -> QuestionId {
        QuestionId::new(value).expect("valid question id")
    }

    fn choice_answer(selected: &str, selected_probability: f64, confidence: f64) -> Answer {
        let share = (1.0 - selected_probability) / (CONCERN_KEYS.len() - 1) as f64;
        let probabilities = CONCERN_KEYS
            .iter()
            .map(|key| {
                let value = if *key == selected {
                    selected_probability
                } else {
                    share
                };
                ((*key).to_owned(), prob(value))
            })
            .collect();
        Answer::Choice(
            ChoiceAnswer::new(selected, probabilities, prob(confidence))
                .expect("valid choice answer"),
        )
    }

    fn score_answer(value: f64) -> Answer {
        let probabilities = RISK_LABELS
            .iter()
            .map(|label| ((*label).to_owned(), prob(0.2)))
            .collect();
        Answer::Score(ScoreAnswer::new(value, probabilities, prob(0.7)).expect("valid score"))
    }

    fn response(
        concern: &str,
        selected_probability: f64,
        confidence: f64,
        risk: f64,
        noul: f64,
    ) -> DecisionResponse {
        let answers = vec![
            (
                qid("concern"),
                choice_answer(concern, selected_probability, confidence),
            ),
            (qid("risk"), score_answer(risk)),
            (
                qid("likely_breaking"),
                Answer::Noul(NoulAnswer::new(prob(noul))),
            ),
            (
                qid("needs_tests"),
                Answer::Noul(NoulAnswer::new(prob(noul))),
            ),
            (qid("needs_docs"), Answer::Noul(NoulAnswer::new(prob(noul)))),
            (
                qid("needs_migration"),
                Answer::Noul(NoulAnswer::new(prob(noul))),
            ),
            (
                qid("security_sensitive"),
                Answer::Noul(NoulAnswer::new(prob(noul))),
            ),
        ];
        DecisionResponse::new("test", "test-v1", answers, None).expect("valid response")
    }

    fn labels(concern: &str, risk: f64, all_true: bool) -> EvalLabels {
        EvalLabels {
            concern: concern.to_owned(),
            risk,
            likely_breaking: all_true,
            needs_tests: all_true,
            needs_docs: all_true,
            needs_migration: all_true,
            security_sensitive: all_true,
        }
    }

    fn case(
        id: &str,
        language: &str,
        change: &str,
        item_kind: &str,
        labels: Option<EvalLabels>,
        ambiguous: bool,
    ) -> EvalCase {
        EvalCase {
            id: id.to_owned(),
            path: "src/lib.rs".to_owned(),
            language: language.to_owned(),
            mode: "signatures".to_owned(),
            change: change.to_owned(),
            item_kind: item_kind.to_owned(),
            stable_key: format!("src/lib.rs::fn::{id}"),
            name: id.to_owned(),
            before: Some(format!("fn {id}() -> u8;")),
            after: Some(format!("fn {id}() -> u16;")),
            labels,
            ambiguous,
        }
    }

    fn tiny_corpus() -> Vec<EvalCase> {
        vec![
            case(
                "a",
                "rust",
                "added",
                "function",
                Some(labels("api-contract", 2.0, false)),
                false,
            ),
            case(
                "b",
                "rust",
                "removed",
                "function",
                Some(labels("data-model", 0.0, true)),
                false,
            ),
            case("c", "elm", "added", "type", None, true),
            case(
                "d",
                "rust",
                "modified",
                "function",
                Some(labels("api-contract", 1.0, false)),
                false,
            ),
        ]
    }

    fn tiny_provider() -> FakeProvider {
        FakeProvider::new(vec![
            Ok(response("api-contract", 0.9, 0.9, 2.0, 0.5)),
            Ok(response("api-contract", 0.9, 0.4, 2.0, 0.5)),
            Ok(response("other", 0.9, 0.9, 0.0, 0.5)),
            Err(DecisionError::Provider {
                message: "boom".to_owned(),
            }),
        ])
    }

    fn options() -> EvalOptions {
        EvalOptions {
            choice_confidence_threshold: 0.6,
        }
    }

    #[test]
    fn load_cases_accepts_a_valid_file() {
        let json = r#"{
            "schema": "ownai.review-eval.v1",
            "cases": [
                {
                    "id": "a",
                    "path": "src/lib.rs",
                    "language": "rust",
                    "mode": "signatures",
                    "change": "added",
                    "item_kind": "function",
                    "stable_key": "src/lib.rs::fn::greet",
                    "name": "greet",
                    "before": null,
                    "after": "pub fn greet() -> u8;",
                    "labels": {
                        "concern": "api-contract",
                        "risk": 1.0,
                        "likely_breaking": false,
                        "needs_tests": true,
                        "needs_docs": false,
                        "needs_migration": false,
                        "security_sensitive": false
                    }
                },
                {
                    "id": "b",
                    "path": "src/lib.rs",
                    "language": "rust",
                    "mode": "signatures",
                    "change": "removed",
                    "item_kind": "function",
                    "stable_key": "src/lib.rs::fn::old",
                    "name": "old",
                    "before": "pub fn old() -> u8;",
                    "after": null,
                    "ambiguous": true
                }
            ]
        }"#;

        let cases = load_cases(json).expect("valid corpus");
        assert_eq!(cases.len(), 2);
        assert_eq!(cases[0].labels.as_ref().expect("labels").risk, 1.0);
        assert!(cases[1].ambiguous);
        assert!(cases[1].labels.is_none());
    }

    #[test]
    fn load_cases_rejects_an_unknown_schema() {
        let error = load_cases(r#"{"schema": "ownai.review-eval.v2", "cases": []}"#)
            .expect_err("unknown schema");
        assert!(matches!(error, EvalError::UnknownSchema { .. }));
    }

    #[test]
    fn load_cases_rejects_unknown_fields_at_the_file_and_case_level() {
        let file_level =
            load_cases(r#"{"schema": "ownai.review-eval.v1", "cases": [], "extra": 1}"#)
                .expect_err("unknown file field");
        assert!(matches!(file_level, EvalError::Json(_)));

        let case_level = load_cases(
            r#"{"schema": "ownai.review-eval.v1", "cases": [{
                "id": "a",
                "path": "src/lib.rs",
                "language": "rust",
                "mode": "signatures",
                "change": "added",
                "item_kind": "function",
                "stable_key": "src/lib.rs::fn::a",
                "name": "a",
                "before": null,
                "after": "fn a();",
                "surprise": true,
                "ambiguous": true
            }]}"#,
        )
        .expect_err("unknown case field");
        assert!(matches!(case_level, EvalError::Json(_)));
    }

    #[test]
    fn load_cases_rejects_a_case_with_neither_labels_nor_ambiguous() {
        let json = r#"{"schema": "ownai.review-eval.v1", "cases": [{
            "id": "bare",
            "path": "src/lib.rs",
            "language": "rust",
            "mode": "signatures",
            "change": "added",
            "item_kind": "function",
            "stable_key": "src/lib.rs::fn::a",
            "name": "a",
            "before": null,
            "after": "fn a();"
        }]}"#;
        let error = load_cases(json).expect_err("unlabeled case");
        assert!(matches!(error, EvalError::InvalidCase { ref id, .. } if id == "bare"));
    }

    #[test]
    fn load_cases_rejects_a_case_with_both_labels_and_ambiguous() {
        let json = r#"{"schema": "ownai.review-eval.v1", "cases": [{
            "id": "both",
            "path": "src/lib.rs",
            "language": "rust",
            "mode": "signatures",
            "change": "added",
            "item_kind": "function",
            "stable_key": "src/lib.rs::fn::a",
            "name": "a",
            "before": null,
            "after": "fn a();",
            "ambiguous": true,
            "labels": {
                "concern": "api-contract",
                "risk": 1.0,
                "likely_breaking": false,
                "needs_tests": false,
                "needs_docs": false,
                "needs_migration": false,
                "security_sensitive": false
            }
        }]}"#;
        let error = load_cases(json).expect_err("both labels and ambiguous");
        assert!(matches!(error, EvalError::InvalidCase { ref id, .. } if id == "both"));
    }

    #[test]
    fn load_cases_rejects_an_out_of_range_risk() {
        for risk in ["5.0", "-0.5"] {
            let json = format!(
                r#"{{"schema": "ownai.review-eval.v1", "cases": [{{
                    "id": "risky",
                    "path": "src/lib.rs",
                    "language": "rust",
                    "mode": "signatures",
                    "change": "added",
                    "item_kind": "function",
                    "stable_key": "src/lib.rs::fn::a",
                    "name": "a",
                    "before": null,
                    "after": "fn a();",
                    "labels": {{
                        "concern": "api-contract",
                        "risk": {risk},
                        "likely_breaking": false,
                        "needs_tests": false,
                        "needs_docs": false,
                        "needs_migration": false,
                        "security_sensitive": false
                    }}
                }}]}}"#
            );
            let error = load_cases(&json).expect_err("out-of-range risk");
            assert!(matches!(error, EvalError::InvalidCase { .. }));
        }
    }

    #[test]
    fn evaluate_computes_known_metrics_and_excludes_failures() {
        let provider = tiny_provider();
        let report = evaluate(&provider, &tiny_corpus(), options());
        let metrics = &report.overall;

        assert_eq!(metrics.cases, 4);
        assert_eq!(metrics.labeled, 2, "only successful labeled cases count");
        assert_eq!(metrics.ambiguous, 1);
        assert_eq!(metrics.failed, 1);
        assert_eq!(metrics.choice_correct, 1);
        assert!((metrics.choice_accuracy - 0.5).abs() < 1e-12);
        assert_eq!(metrics.covered, 1);
        assert!((metrics.coverage - 0.5).abs() < 1e-12);
        assert!((metrics.covered_accuracy - 1.0).abs() < 1e-12);
        assert!((metrics.noul_brier - 0.25).abs() < 1e-12);
        assert!((metrics.noul_calibration_error - 0.0).abs() < 1e-12);
        assert!((metrics.score_mae - 1.0).abs() < 1e-12);
        assert!(metrics.input_bytes > 0);
        assert!(metrics.total_latency_ms >= 0.0);
        assert_eq!(provider.remaining(), 0, "every case is evaluated once");
    }

    #[test]
    fn evaluate_groups_by_language_change_and_item_kind_ascending() {
        let report = evaluate(&tiny_provider(), &tiny_corpus(), options());

        let languages: Vec<&str> = report
            .by_language
            .iter()
            .map(|group| group.name.as_str())
            .collect();
        assert_eq!(languages, vec!["elm", "rust"]);
        let rust = report
            .by_language
            .iter()
            .find(|group| group.name == "rust")
            .expect("rust group");
        assert_eq!(rust.metrics.cases, 3);
        assert_eq!(rust.metrics.labeled, 2);
        assert_eq!(rust.metrics.failed, 1);
        let elm = report
            .by_language
            .iter()
            .find(|group| group.name == "elm")
            .expect("elm group");
        assert_eq!(elm.metrics.cases, 1);
        assert_eq!(elm.metrics.ambiguous, 1);
        assert_eq!(elm.metrics.labeled, 0);

        let changes: Vec<&str> = report
            .by_change
            .iter()
            .map(|group| group.name.as_str())
            .collect();
        assert_eq!(changes, vec!["added", "modified", "removed"]);

        let kinds: Vec<&str> = report
            .by_item_kind
            .iter()
            .map(|group| group.name.as_str())
            .collect();
        assert_eq!(kinds, vec!["function", "type"]);
    }

    #[test]
    fn evaluate_uses_zero_for_empty_denominators() {
        let report = evaluate(&FakeProvider::new(vec![]), &[], options());
        assert_eq!(report.overall, Metrics::default());
        assert!(report.by_language.is_empty());
        assert!(report.by_change.is_empty());
        assert!(report.by_item_kind.is_empty());
    }

    #[test]
    fn evaluate_counts_an_unreconstructable_key_as_a_failure() {
        let mut bad = case(
            "bad",
            "rust",
            "added",
            "function",
            Some(labels("api-contract", 1.0, false)),
            false,
        );
        bad.language = "cobol".to_owned();
        let provider = FakeProvider::new(vec![]);

        let report = evaluate(&provider, &[bad], options());

        assert_eq!(report.overall.cases, 1);
        assert_eq!(report.overall.failed, 1);
        assert_eq!(report.overall.labeled, 0);
        assert_eq!(report.overall.input_bytes, 0, "no state for a bad key");
        assert!(
            provider.requests().is_empty(),
            "a bad key never reaches the provider"
        );
    }
}
