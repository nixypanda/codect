//! Decision-provider construction and the deterministic offline fake provider.
//!
//! The fake provider is always compiled and never opens a network connection. It
//! answers every question from the request's own shape, so it is reproducible and
//! backs the Phase 1 end-to-end test. The real providers are wired in later
//! steps; until then [`build_provider`] reports them as unavailable rather than
//! silently substituting a different implementation.

use ownai_decisions::{
    Answer, ChoiceAnswer, DecisionError, DecisionProvider, DecisionRequest, DecisionResponse,
    NoulAnswer, Probability, Question, ScoreAnswer,
};

use crate::args::DecisionProviderChoice;

/// The model revision the fake provider reports when none is configured.
pub const DEFAULT_FAKE_MODEL_REVISION: &str = "fake-v1";

/// A provider construction failure that the command layer reports to the user.
#[derive(Debug, thiserror::Error)]
pub enum ProviderBuildError {
    /// The requested provider is not implemented in this build.
    #[error("the `{provider}` decision provider is not yet available")]
    Unavailable { provider: &'static str },

    /// The requested provider does not accept an explicit endpoint.
    #[error("the `{provider}` decision provider does not support `--decision-endpoint`")]
    EndpointUnsupported { provider: &'static str },
}

/// A deterministic, network-free [`DecisionProvider`] for tests and fixtures.
///
/// The answer scheme depends only on the request's questions and options:
///
/// - a choice selects the first option with probability 0.5 and splits the
///   remaining 0.5 evenly across the other options, at confidence 0.9;
/// - a score returns value 0.0 with a uniform distribution over the levels, at
///   confidence 0.5; and
/// - a noul returns probability 0.5.
pub struct FakeDecisionProvider {
    model_revision: String,
}

impl FakeDecisionProvider {
    /// Builds a fake provider that reports `model_revision`.
    pub fn new(model_revision: impl Into<String>) -> Self {
        Self {
            model_revision: model_revision.into(),
        }
    }
}

impl DecisionProvider for FakeDecisionProvider {
    fn evaluate(&self, request: &DecisionRequest) -> Result<DecisionResponse, DecisionError> {
        let answers = request
            .questions()
            .iter()
            .map(|question| (question.id().clone(), answer_for(question)))
            .collect();

        // The only caller-supplied value is the model revision; an empty one is
        // rejected as a typed invalid response rather than panicking.
        DecisionResponse::new("fake", &self.model_revision, answers, None)
            .map_err(DecisionError::invalid_response)
    }
}

/// Answers one validated question with the deterministic fake scheme.
///
/// Every distribution is a genuine invariant of the question's validated shape,
/// so the `expect`s below assert adapter bugs rather than caller input.
fn answer_for(question: &Question) -> Answer {
    match question {
        Question::Choice(choice) => {
            let options = choice.options();
            let first = options
                .first()
                .expect("a validated choice question has at least two options");
            let others = options
                .len()
                .checked_sub(1)
                .filter(|count| *count > 0)
                .expect("a validated choice question has at least two options");
            let share = Probability::new(0.5 / others as f64)
                .expect("an even share of one half is a valid probability");

            let mut probabilities = Vec::with_capacity(options.len());
            probabilities.push((
                first.key().to_owned(),
                Probability::new(0.5).expect("one half is a valid probability"),
            ));
            for option in &options[1..] {
                probabilities.push((option.key().to_owned(), share));
            }

            Answer::Choice(
                ChoiceAnswer::new(
                    first.key(),
                    probabilities,
                    Probability::new(0.9).expect("0.9 is a valid probability"),
                )
                .expect("the deterministic first-option distribution is a valid choice answer"),
            )
        }
        Question::Score(score) => {
            let share = Probability::new(1.0 / score.levels().len() as f64)
                .expect("a uniform share of a validated score question is a valid probability");
            let probabilities = score
                .levels()
                .iter()
                .map(|level| (level.label().to_owned(), share))
                .collect();

            Answer::Score(
                ScoreAnswer::new(
                    0.0,
                    probabilities,
                    Probability::new(0.5).expect("one half is a valid probability"),
                )
                .expect("a zero score with a uniform distribution is a valid score answer"),
            )
        }
        Question::Noul(_) => Answer::Noul(NoulAnswer::new(
            Probability::new(0.5).expect("one half is a valid probability"),
        )),
    }
}

/// Constructs the decision provider selected on the command line.
///
/// `endpoint` is rejected by the fake provider so a flag that cannot take effect
/// is never silently ignored.
pub fn build_provider(
    choice: DecisionProviderChoice,
    model: Option<&str>,
    endpoint: Option<&str>,
) -> Result<Box<dyn DecisionProvider>, ProviderBuildError> {
    match choice {
        DecisionProviderChoice::Fake => {
            if endpoint.is_some() {
                return Err(ProviderBuildError::EndpointUnsupported { provider: "fake" });
            }
            let model_revision = model.unwrap_or(DEFAULT_FAKE_MODEL_REVISION);
            Ok(Box::new(FakeDecisionProvider::new(model_revision)))
        }
        DecisionProviderChoice::Typesafe => Err(ProviderBuildError::Unavailable {
            provider: "typesafe",
        }),
        DecisionProviderChoice::Laya => Err(ProviderBuildError::Unavailable { provider: "laya" }),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use ownai_decisions::{
        Answer, ChoiceOption, ChoiceQuestion, NoulQuestion, QuestionId, ScoreLevel, ScoreQuestion,
    };

    use super::*;

    fn id(value: &str) -> QuestionId {
        QuestionId::new(value).expect("valid question id")
    }

    fn option(key: &str) -> ChoiceOption {
        ChoiceOption::new(key, None).expect("valid choice option")
    }

    fn level(label: &str) -> ScoreLevel {
        ScoreLevel::new(label, None).expect("valid score level")
    }

    fn sample_request() -> DecisionRequest {
        DecisionRequest::new(
            json!({ "schema": "test" }),
            vec![
                Question::Choice(
                    ChoiceQuestion::new(
                        id("concern"),
                        "Which concern?",
                        vec![option("api-contract"), option("other")],
                    )
                    .expect("valid question"),
                ),
                Question::Score(
                    ScoreQuestion::new(
                        id("risk"),
                        "How risky?",
                        vec![level("low"), level("moderate"), level("high")],
                    )
                    .expect("valid question"),
                ),
                Question::Noul(
                    NoulQuestion::new(id("needs_tests"), "Does this need tests?").expect("valid"),
                ),
            ],
        )
        .expect("valid request")
    }

    #[test]
    fn fake_provider_answers_every_kind_validly_and_deterministically() {
        let request = sample_request();
        let provider = FakeDecisionProvider::new(DEFAULT_FAKE_MODEL_REVISION);

        let first = provider.evaluate(&request).expect("evaluate succeeds");
        let second = provider.evaluate(&request).expect("evaluate succeeds");

        assert_eq!(first, second, "the fake provider must be deterministic");
        first
            .validate_for(&request)
            .expect("every answer matches its question");
        assert_eq!(first.answers().len(), request.questions().len());
        assert_eq!(first.provider(), "fake");
        assert_eq!(first.model_revision(), "fake-v1");
        assert_eq!(first.usage(), None);
    }

    #[test]
    fn fake_choice_selects_the_first_option_and_splits_the_remainder() {
        let request = sample_request();
        let response = FakeDecisionProvider::new(DEFAULT_FAKE_MODEL_REVISION)
            .evaluate(&request)
            .expect("evaluate succeeds");

        let Some(Answer::Choice(choice)) = response.answers().get(&id("concern")) else {
            panic!("concern must be a choice answer");
        };
        assert_eq!(choice.selected(), "api-contract");
        assert_eq!(choice.confidence().get(), 0.9);
        assert_eq!(
            choice.probabilities().get("api-contract").map(|p| p.get()),
            Some(0.5)
        );
        assert_eq!(
            choice.probabilities().get("other").map(|p| p.get()),
            Some(0.5)
        );
    }

    #[test]
    fn fake_score_is_zero_and_uniform_over_the_levels() {
        let request = sample_request();
        let response = FakeDecisionProvider::new(DEFAULT_FAKE_MODEL_REVISION)
            .evaluate(&request)
            .expect("evaluate succeeds");

        let Some(Answer::Score(score)) = response.answers().get(&id("risk")) else {
            panic!("risk must be a score answer");
        };
        assert_eq!(score.value(), 0.0);
        assert_eq!(score.confidence().get(), 0.5);
        assert_eq!(score.probabilities().len(), 3);
        for probability in score.probabilities().values() {
            assert!((probability.get() - 1.0 / 3.0).abs() < 0.000_1);
        }
    }

    #[test]
    fn fake_provider_honours_a_model_revision_override() {
        let provider = build_provider(DecisionProviderChoice::Fake, Some("custom-model"), None)
            .expect("the fake provider builds");
        let response = provider
            .evaluate(&sample_request())
            .expect("evaluate succeeds");
        assert_eq!(response.model_revision(), "custom-model");
    }

    #[test]
    fn build_provider_reports_typesafe_and_laya_as_unavailable() {
        for choice in [
            DecisionProviderChoice::Typesafe,
            DecisionProviderChoice::Laya,
        ] {
            let error = build_error(choice, None, None);
            assert!(matches!(error, ProviderBuildError::Unavailable { .. }));
        }
    }

    #[test]
    fn fake_provider_rejects_an_endpoint() {
        let error = build_error(
            DecisionProviderChoice::Fake,
            None,
            Some("http://127.0.0.1:8080"),
        );
        assert!(matches!(
            error,
            ProviderBuildError::EndpointUnsupported { provider: "fake" }
        ));
    }

    /// Runs `build_provider` and returns its typed error. A success is a test
    /// failure, so the `Ok` side never needs a `Debug` bound.
    fn build_error(
        choice: DecisionProviderChoice,
        model: Option<&str>,
        endpoint: Option<&str>,
    ) -> ProviderBuildError {
        match build_provider(choice, model, endpoint) {
            Ok(_) => panic!("expected provider construction to fail"),
            Err(error) => error,
        }
    }
}
