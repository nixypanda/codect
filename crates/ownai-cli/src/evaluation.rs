//! Deterministic plain-text rendering of a semantic-lens evaluation report.
//!
//! Rendering lives in the CLI, never in the engine: the harness returns data,
//! and this module owns the field order and numeric precision. The result ends
//! with exactly one newline and never contains ANSI escapes.

use std::fmt::Write as _;

use ownai_engine::{EvaluationReport, GroupMetrics};

/// Renders one evaluation report.
///
/// Ratios use four decimal places and latency uses two, matching the fixed
/// field order of the report. The three group sections are always present, even
/// when a group list is empty.
pub fn render_report(report: &EvaluationReport, schema: &str, threshold: f64) -> String {
    let metrics = &report.overall;
    let mut out = String::new();

    let _ = writeln!(out, "ownai review evaluation");
    let _ = writeln!(out, "schema: {schema}");
    let _ = writeln!(out, "choice-confidence-threshold: {threshold}");
    let _ = writeln!(out, "cases: {}", metrics.cases);
    let _ = writeln!(out, "labeled: {}", metrics.labeled);
    let _ = writeln!(out, "ambiguous: {}", metrics.ambiguous);
    let _ = writeln!(out, "failed: {}", metrics.failed);
    let _ = writeln!(out, "choice-accuracy: {:.4}", metrics.choice_accuracy);
    let _ = writeln!(out, "coverage: {:.4}", metrics.coverage);
    let _ = writeln!(out, "covered-accuracy: {:.4}", metrics.covered_accuracy);
    let _ = writeln!(out, "noul-brier: {:.4}", metrics.noul_brier);
    let _ = writeln!(
        out,
        "noul-calibration-error: {:.4}",
        metrics.noul_calibration_error
    );
    let _ = writeln!(out, "score-mae: {:.4}", metrics.score_mae);
    let _ = writeln!(out, "input-bytes: {}", metrics.input_bytes);
    let _ = writeln!(out, "mean-latency-ms: {:.2}", metrics.mean_latency_ms);

    let _ = writeln!(out, "\nby language:");
    write_group_section(&mut out, &report.by_language);
    let _ = writeln!(out, "\nby change:");
    write_group_section(&mut out, &report.by_change);
    let _ = writeln!(out, "\nby item kind:");
    write_group_section(&mut out, &report.by_item_kind);

    out
}

fn write_group_section(out: &mut String, groups: &[GroupMetrics]) {
    for group in groups {
        let metrics = &group.metrics;
        let _ = writeln!(
            out,
            "  {}: cases={} labeled={} ambiguous={} failed={} choice-accuracy={:.4} \
             coverage={:.4} covered-accuracy={:.4} noul-brier={:.4} \
             noul-calibration-error={:.4} score-mae={:.4} input-bytes={}",
            group.name,
            metrics.cases,
            metrics.labeled,
            metrics.ambiguous,
            metrics.failed,
            metrics.choice_accuracy,
            metrics.coverage,
            metrics.covered_accuracy,
            metrics.noul_brier,
            metrics.noul_calibration_error,
            metrics.score_mae,
            metrics.input_bytes,
        );
    }
}

#[cfg(test)]
mod tests {
    use ownai_engine::{EvaluationReport, GroupMetrics, Metrics};

    use super::*;

    fn metrics() -> Metrics {
        Metrics {
            cases: 16,
            labeled: 15,
            ambiguous: 1,
            failed: 0,
            choice_correct: 5,
            choice_accuracy: 0.3333,
            covered: 15,
            coverage: 1.0,
            covered_accuracy: 0.3333,
            noul_brier: 0.25,
            noul_calibration_error: 0.0,
            score_mae: 2.0,
            input_bytes: 12345,
            total_latency_ms: 1.92,
            mean_latency_ms: 0.12,
        }
    }

    #[test]
    fn report_renders_the_fixed_field_order() {
        let report = EvaluationReport {
            overall: metrics(),
            by_language: vec![GroupMetrics {
                name: "elm".to_owned(),
                metrics: Metrics {
                    cases: 3,
                    labeled: 3,
                    choice_accuracy: 0.0,
                    coverage: 1.0,
                    covered_accuracy: 0.0,
                    noul_brier: 0.25,
                    noul_calibration_error: 0.0,
                    score_mae: 1.0,
                    input_bytes: 1234,
                    ..Metrics::default()
                },
            }],
            by_change: vec![],
            by_item_kind: vec![],
        };

        let rendered = render_report(&report, "ownai.review-eval.v1", 0.55);

        let expected = concat!(
            "ownai review evaluation\n",
            "schema: ownai.review-eval.v1\n",
            "choice-confidence-threshold: 0.55\n",
            "cases: 16\n",
            "labeled: 15\n",
            "ambiguous: 1\n",
            "failed: 0\n",
            "choice-accuracy: 0.3333\n",
            "coverage: 1.0000\n",
            "covered-accuracy: 0.3333\n",
            "noul-brier: 0.2500\n",
            "noul-calibration-error: 0.0000\n",
            "score-mae: 2.0000\n",
            "input-bytes: 12345\n",
            "mean-latency-ms: 0.12\n",
            "\n",
            "by language:\n",
            "  elm: cases=3 labeled=3 ambiguous=0 failed=0 choice-accuracy=0.0000 \
             coverage=1.0000 covered-accuracy=0.0000 noul-brier=0.2500 \
             noul-calibration-error=0.0000 score-mae=1.0000 input-bytes=1234\n",
            "\n",
            "by change:\n",
            "\n",
            "by item kind:\n",
        );
        assert_eq!(rendered, expected);
    }

    #[test]
    fn report_ends_with_exactly_one_newline_and_no_escapes() {
        let report = EvaluationReport {
            overall: metrics(),
            by_language: vec![],
            by_change: vec![],
            by_item_kind: vec![],
        };
        let rendered = render_report(&report, "ownai.review-eval.v1", 0.55);
        assert!(rendered.ends_with('\n'));
        assert!(!rendered.ends_with("\n\n"));
        assert!(!rendered.contains('\r'));
        assert!(!rendered.contains('\u{1b}'));
    }
}
