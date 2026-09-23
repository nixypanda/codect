//! Decision-provider construction and the deterministic offline fake provider.
//!
//! The fake provider is always compiled and never opens a network connection. It
//! answers every question from the request's own shape, so it is reproducible and
//! backs the Phase 1 end-to-end test. [`build_provider`] also constructs the
//! optional TypeSafe provider when the `typesafe` feature is enabled; it reports
//! any provider that is unavailable in this build as a typed error rather than
//! silently substituting a different implementation.

#[cfg(any(feature = "typesafe", feature = "laya"))]
use ownai_decisions::Secret;
use ownai_decisions::{
    Answer, ChoiceAnswer, DecisionError, DecisionProvider, DecisionRequest, DecisionResponse,
    NoulAnswer, Probability, Question, ScoreAnswer,
};
#[cfg(feature = "laya")]
use ownai_decisions::{DEFAULT_LAYA_ENDPOINT, DEFAULT_LAYA_MODEL, LayaConfig, LayaProvider};
#[cfg(feature = "typesafe")]
use ownai_decisions::{
    DEFAULT_TYPESAFE_ENDPOINT, DEFAULT_TYPESAFE_MODEL, TypeSafeConfig, TypeSafeProvider,
};

use crate::args::DecisionProviderChoice;

/// Where a decision provider sends review state, which determines the privacy
/// obligations the CLI must satisfy before evaluating anything.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderRoute {
    /// A deterministic, offline provider. State never leaves the process, so no
    /// disclosure is required.
    LocalTest,
    /// A local process. State leaves the process but stays on this machine, so a
    /// distinct notice is shown but no acknowledgement is required.
    Local,
    /// A remote service. State leaves this machine, so a disclosure is required
    /// and non-interactive use must acknowledge it.
    Remote,
}

impl DecisionProviderChoice {
    /// Classifies where the selected provider sends state.
    pub fn route(self) -> ProviderRoute {
        match self {
            Self::Fake => ProviderRoute::LocalTest,
            Self::Laya => ProviderRoute::Local,
            Self::Typesafe => ProviderRoute::Remote,
        }
    }
}

/// The model revision the fake provider reports when none is configured.
pub const DEFAULT_FAKE_MODEL_REVISION: &str = "fake-v1";

/// A provider construction failure that the command layer reports to the user.
#[derive(Debug, thiserror::Error)]
pub enum ProviderBuildError {
    /// The requested provider is not implemented in this build.
    #[cfg_attr(any(feature = "typesafe", feature = "laya"), allow(dead_code))]
    #[error("the `{provider}` decision provider is not yet available")]
    Unavailable { provider: &'static str },

    /// The requested provider does not accept an explicit endpoint.
    #[error("the `{provider}` decision provider does not support `--decision-endpoint`")]
    EndpointUnsupported { provider: &'static str },

    /// The provider needs an environment credential that is missing or empty.
    #[cfg_attr(not(feature = "typesafe"), allow(dead_code))]
    #[error("the `{provider}` decision provider requires the `{variable}` environment variable")]
    MissingCredential {
        provider: &'static str,
        variable: &'static str,
    },

    /// An explicit endpoint was supplied without the opt-in flag.
    #[cfg_attr(not(feature = "typesafe"), allow(dead_code))]
    #[error(
        "the `{provider}` decision provider requires `--allow-custom-endpoint` for a custom endpoint"
    )]
    CustomEndpointNotAllowed { provider: &'static str },
}

/// The provider-construction inputs gathered from the command line.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProviderOptions<'a> {
    /// An explicit model alias from `--decision-model`.
    pub model: Option<&'a str>,
    /// An explicit endpoint from `--decision-endpoint`.
    pub endpoint: Option<&'a str>,
    /// Whether an explicit endpoint has been explicitly allowed.
    #[cfg_attr(not(feature = "typesafe"), allow(dead_code))]
    pub allow_custom_endpoint: bool,
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

/// Resolves the effective model alias a provider would use, without building it.
///
/// The dry-run and disclosure plans call this so their `model:` line matches the
/// provider that will actually run. For the `fake` provider a missing alias
/// stays `None`, because its disclosed default revision is an implementation
/// detail. TypeSafe falls back to `OWNAI_DECISION_MODEL` and then to
/// [`DEFAULT_TYPESAFE_MODEL`], matching [`build_provider`].
pub fn resolve_model(choice: DecisionProviderChoice, flag: Option<&str>) -> Option<String> {
    match choice {
        DecisionProviderChoice::Fake => flag.map(str::to_owned),
        DecisionProviderChoice::Laya => {
            #[cfg(feature = "laya")]
            {
                Some(
                    flag.map(str::to_owned)
                        .or_else(env_model)
                        .unwrap_or_else(|| DEFAULT_LAYA_MODEL.to_owned()),
                )
            }
            #[cfg(not(feature = "laya"))]
            {
                flag.map(str::to_owned)
            }
        }
        DecisionProviderChoice::Typesafe => {
            #[cfg(feature = "typesafe")]
            {
                Some(
                    flag.map(str::to_owned)
                        .or_else(env_model)
                        .unwrap_or_else(|| DEFAULT_TYPESAFE_MODEL.to_owned()),
                )
            }
            #[cfg(not(feature = "typesafe"))]
            {
                flag.map(str::to_owned)
            }
        }
    }
}

/// Reads `OWNAI_DECISION_MODEL`, treating an empty value as unset.
#[cfg(any(feature = "typesafe", feature = "laya"))]
fn env_model() -> Option<String> {
    std::env::var("OWNAI_DECISION_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// Constructs the decision provider selected on the command line.
///
/// `endpoint` is rejected by the fake provider so a flag that cannot take effect
/// is never silently ignored. For TypeSafe any explicit endpoint requires
/// [`ProviderOptions::allow_custom_endpoint`], even one equal to the default, so
/// a configuration typo cannot redirect repository state to another host. The
/// API key is read only from `TYPESAFE_API_KEY`, never from a flag or config.
pub fn build_provider(
    choice: DecisionProviderChoice,
    options: ProviderOptions<'_>,
) -> Result<Box<dyn DecisionProvider>, ProviderBuildError> {
    match choice {
        DecisionProviderChoice::Fake => {
            if options.endpoint.is_some() {
                return Err(ProviderBuildError::EndpointUnsupported { provider: "fake" });
            }
            let model_revision = options.model.unwrap_or(DEFAULT_FAKE_MODEL_REVISION);
            Ok(Box::new(FakeDecisionProvider::new(model_revision)))
        }
        DecisionProviderChoice::Typesafe => build_typesafe(options),
        DecisionProviderChoice::Laya => build_laya(options),
    }
}

/// Builds the Laya provider when the feature is enabled.
///
/// The endpoint defaults to loopback. Any explicit endpoint requires
/// [`ProviderOptions::allow_custom_endpoint`] so a typo cannot redirect review
/// state to an unintended host. The optional API key is read only from
/// `LAYA_API_KEY`, never from a flag or config.
#[cfg(feature = "laya")]
fn build_laya(
    options: ProviderOptions<'_>,
) -> Result<Box<dyn DecisionProvider>, ProviderBuildError> {
    let endpoint = match options.endpoint {
        Some(endpoint) => {
            if !options.allow_custom_endpoint {
                return Err(ProviderBuildError::CustomEndpointNotAllowed { provider: "laya" });
            }
            endpoint.to_owned()
        }
        None => DEFAULT_LAYA_ENDPOINT.to_owned(),
    };

    let model = resolve_model(DecisionProviderChoice::Laya, options.model)
        .unwrap_or_else(|| DEFAULT_LAYA_MODEL.to_owned());

    let mut config = LayaConfig::new(endpoint, model);
    config.api_key = std::env::var("LAYA_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(Secret::new);
    Ok(Box::new(LayaProvider::new(config)))
}

/// Reports Laya as unavailable when the feature is disabled.
#[cfg(not(feature = "laya"))]
fn build_laya(
    _options: ProviderOptions<'_>,
) -> Result<Box<dyn DecisionProvider>, ProviderBuildError> {
    Err(ProviderBuildError::Unavailable { provider: "laya" })
}

/// Builds the TypeSafe provider when the feature is enabled.
#[cfg(feature = "typesafe")]
fn build_typesafe(
    options: ProviderOptions<'_>,
) -> Result<Box<dyn DecisionProvider>, ProviderBuildError> {
    let endpoint = match options.endpoint {
        Some(endpoint) => {
            if !options.allow_custom_endpoint {
                return Err(ProviderBuildError::CustomEndpointNotAllowed {
                    provider: "typesafe",
                });
            }
            endpoint.to_owned()
        }
        None => DEFAULT_TYPESAFE_ENDPOINT.to_owned(),
    };

    let model = resolve_model(DecisionProviderChoice::Typesafe, options.model)
        .unwrap_or_else(|| DEFAULT_TYPESAFE_MODEL.to_owned());

    let api_key = std::env::var("TYPESAFE_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or(ProviderBuildError::MissingCredential {
            provider: "typesafe",
            variable: "TYPESAFE_API_KEY",
        })?;

    let config = TypeSafeConfig::new(endpoint, model, Secret::new(api_key));
    Ok(Box::new(TypeSafeProvider::new(config)))
}

/// Reports TypeSafe as unavailable when the feature is disabled.
#[cfg(not(feature = "typesafe"))]
fn build_typesafe(
    _options: ProviderOptions<'_>,
) -> Result<Box<dyn DecisionProvider>, ProviderBuildError> {
    Err(ProviderBuildError::Unavailable {
        provider: "typesafe",
    })
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
        let provider = build_provider(
            DecisionProviderChoice::Fake,
            ProviderOptions {
                model: Some("custom-model"),
                ..ProviderOptions::default()
            },
        )
        .expect("the fake provider builds");
        let response = provider
            .evaluate(&sample_request())
            .expect("evaluate succeeds");
        assert_eq!(response.model_revision(), "custom-model");
    }

    #[cfg(not(feature = "laya"))]
    #[test]
    fn build_provider_reports_laya_as_unavailable() {
        let error = build_error(DecisionProviderChoice::Laya, ProviderOptions::default());
        assert!(matches!(
            error,
            ProviderBuildError::Unavailable { provider: "laya" }
        ));
    }

    #[cfg(feature = "laya")]
    #[test]
    fn laya_builds_with_the_default_local_endpoint() {
        // Construction must not contact the server; only `evaluate` does.
        let provider = build_provider(DecisionProviderChoice::Laya, ProviderOptions::default())
            .expect("the laya provider builds without a server");
        let _ = provider;
    }

    #[cfg(feature = "laya")]
    #[test]
    fn laya_rejects_a_custom_endpoint_without_opt_in() {
        let error = build_error(
            DecisionProviderChoice::Laya,
            ProviderOptions {
                endpoint: Some("http://127.0.0.1:9000/v1/systemone"),
                ..ProviderOptions::default()
            },
        );
        assert!(matches!(
            error,
            ProviderBuildError::CustomEndpointNotAllowed { provider: "laya" }
        ));
    }

    #[cfg(feature = "typesafe")]
    #[test]
    fn typesafe_rejects_a_custom_endpoint_without_opt_in() {
        // The endpoint check precedes credential lookup, so this is
        // deterministic even when `TYPESAFE_API_KEY` is set in the environment.
        let error = build_error(
            DecisionProviderChoice::Typesafe,
            ProviderOptions {
                endpoint: Some("http://127.0.0.1:8080"),
                ..ProviderOptions::default()
            },
        );
        assert!(matches!(
            error,
            ProviderBuildError::CustomEndpointNotAllowed {
                provider: "typesafe"
            }
        ));
    }

    #[test]
    fn fake_provider_rejects_an_endpoint() {
        let error = build_error(
            DecisionProviderChoice::Fake,
            ProviderOptions {
                endpoint: Some("http://127.0.0.1:8080"),
                ..ProviderOptions::default()
            },
        );
        assert!(matches!(
            error,
            ProviderBuildError::EndpointUnsupported { provider: "fake" }
        ));
    }

    #[test]
    fn provider_routes_match_the_disclosure_contract() {
        assert_eq!(
            DecisionProviderChoice::Fake.route(),
            ProviderRoute::LocalTest
        );
        assert_eq!(DecisionProviderChoice::Laya.route(), ProviderRoute::Local);
        assert_eq!(
            DecisionProviderChoice::Typesafe.route(),
            ProviderRoute::Remote
        );
    }

    /// Runs `build_provider` and returns its typed error. A success is a test
    /// failure, so the `Ok` side never needs a `Debug` bound.
    fn build_error(
        choice: DecisionProviderChoice,
        options: ProviderOptions<'_>,
    ) -> ProviderBuildError {
        match build_provider(choice, options) {
            Ok(_) => panic!("expected provider construction to fail"),
            Err(error) => error,
        }
    }
}
