//! Contract tests for the Laya decision provider against a local mock HTTP
//! server. No test here touches the network, loads a checkpoint, or needs a
//! credential.

#![cfg(feature = "laya")]

mod support;

use std::time::Duration;

use ownai_decisions::{
    Answer, ChoiceOption, ChoiceQuestion, DecisionError, DecisionProvider, DecisionRequest,
    LayaConfig, LayaProvider, NoulQuestion, Question, QuestionId, ScoreLevel, ScoreQuestion,
    Secret, Usage, ValidationError,
};
use serde_json::{Value, json};
use support::{MockServer, ScriptedResponse};

fn qid(value: &str) -> QuestionId {
    QuestionId::new(value).expect("valid question id")
}

fn option(key: &str) -> ChoiceOption {
    ChoiceOption::new(key, None).expect("valid option")
}

fn level(label: &str) -> ScoreLevel {
    ScoreLevel::new(label, None).expect("valid level")
}

/// One request exercising all three primitives.
fn sample_request() -> DecisionRequest {
    DecisionRequest::new(
        json!({ "schema": "ownai.review-unit.v1", "after": "fn changed();" }),
        vec![
            Question::Choice(
                ChoiceQuestion::new(
                    qid("concern"),
                    "Which concern?",
                    vec![option("api-contract"), option("other")],
                )
                .expect("valid choice question"),
            ),
            Question::Score(
                ScoreQuestion::new(
                    qid("risk"),
                    "How risky?",
                    vec![level("routine"), level("low")],
                )
                .expect("valid score question"),
            ),
            Question::Noul(
                NoulQuestion::new(qid("likely_breaking"), "Is it breaking?").expect("valid"),
            ),
        ],
    )
    .expect("valid request")
}

/// A valid Jev-shaped response body covering the sample request.
fn valid_response_body() -> Value {
    json!({
        "model": "laya-rl-agent",
        "answers": {
            "concern": {
                "type": "choice",
                "choice": "api-contract",
                "probabilities": { "api-contract": 0.8, "other": 0.2 },
                "confidence": 0.7,
                "action": { "act_probability": 1.0 }
            },
            "risk": {
                "type": "score",
                "score": 0.6,
                "legend": { "0": "routine", "1": "low" },
                "probabilities": { "0": 0.6, "1": 0.4 },
                "confidence": 0.6,
                "action": { "act_probability": 1.0 }
            },
            "likely_breaking": { "type": "noul", "noul": 0.71, "confidence": 0.71 }
        },
        "usage": { "input_tokens": 100, "output_tokens": 0 },
        "routing": {
            "model": "english",
            "repo": "convaiinnovations/laya",
            "reason": "latin script"
        }
    })
}

/// Builds a provider pointed at a mock server.
fn provider(server: &MockServer, max_response_bytes: usize, max_retries: u32) -> LayaProvider {
    let mut config = LayaConfig::new(server.base_url(), "english");
    config.max_response_bytes = max_response_bytes;
    config.max_retries = max_retries;
    config.connect_timeout = Duration::from_secs(5);
    config.request_timeout = Duration::from_secs(5);
    LayaProvider::new(config)
}

/// Evaluates one scripted response and returns the typed error's validation
/// source. A non-`InvalidResponse` outcome fails the test.
fn invalid_source(body: Value) -> ValidationError {
    let server = MockServer::scripted(vec![ScriptedResponse::json(body.to_string())]);
    match provider(&server, 1024 * 1024, 0).evaluate(&sample_request()) {
        Err(DecisionError::InvalidResponse { source }) => source,
        other => panic!("expected InvalidResponse, got {other:?}"),
    }
}

#[test]
fn a_successful_evaluate_decodes_all_three_primitives() {
    let server = MockServer::scripted(vec![ScriptedResponse::json(
        valid_response_body().to_string(),
    )]);
    let request = sample_request();

    let response = provider(&server, 1024 * 1024, 2)
        .evaluate(&request)
        .expect("evaluate succeeds");

    assert_eq!(response.provider(), "laya");
    assert_eq!(
        response.model_revision(),
        "english",
        "the selected checkpoint is the revision"
    );
    assert_eq!(
        response.usage(),
        Some(Usage {
            input_tokens: 100,
            output_tokens: 0,
        })
    );
    response
        .validate_for(&request)
        .expect("answers match the request");

    let Some(Answer::Choice(choice)) = response.answers().get(&qid("concern")) else {
        panic!("concern must be a choice answer");
    };
    assert_eq!(choice.selected(), "api-contract");
    assert_eq!(choice.confidence().get(), 0.7);

    let Some(Answer::Score(score)) = response.answers().get(&qid("risk")) else {
        panic!("risk must be a score answer");
    };
    assert_eq!(score.value(), 0.6);
    assert_eq!(
        score.probabilities().keys().collect::<Vec<_>>(),
        vec!["low", "routine"],
        "index keys map onto the request's level labels"
    );

    let Some(Answer::Noul(noul)) = response.answers().get(&qid("likely_breaking")) else {
        panic!("likely_breaking must be a noul answer");
    };
    assert_eq!(noul.probability().get(), 0.71);
}

#[test]
fn the_request_body_is_jev_shaped() {
    let server = MockServer::always(ScriptedResponse::json(valid_response_body().to_string()));
    provider(&server, 1024 * 1024, 0)
        .evaluate(&sample_request())
        .expect("evaluate succeeds");

    let requests = server.requests();
    let request = requests.first().expect("one request");
    assert_eq!(request.method, "POST");
    let body: Value = serde_json::from_str(&request.body).expect("request body is JSON");
    assert_eq!(body["model"], "english");
    assert_eq!(body["state"]["schema"], "ownai.review-unit.v1");
    assert_eq!(body["questions"]["concern"]["type"], "choice");
    assert_eq!(
        body["questions"]["concern"]["criteria"]["api-contract"],
        "api-contract"
    );
    assert_eq!(body["questions"]["risk"]["type"], "score");
    assert_eq!(
        body["questions"]["risk"]["criteria"],
        json!(["routine", "low"])
    );
    assert_eq!(body["questions"]["likely_breaking"]["type"], "noul");
    assert!(
        body["questions"]["likely_breaking"]
            .get("criteria")
            .is_none()
    );
}

#[test]
fn rounded_distributions_are_renormalized() {
    let request = DecisionRequest::new(
        json!({}),
        vec![Question::Choice(
            ChoiceQuestion::new(
                qid("concern"),
                "Which concern?",
                vec![option("a"), option("b"), option("c")],
            )
            .expect("valid"),
        )],
    )
    .expect("valid request");
    let body = json!({
        "model": "laya-rl-agent",
        "answers": {
            "concern": {
                "type": "choice",
                "choice": "a",
                "probabilities": { "a": 0.3334, "b": 0.3333, "c": 0.3333 },
                "confidence": 0.34
            }
        },
        "usage": { "input_tokens": 1, "output_tokens": 0 }
    });
    let server = MockServer::scripted(vec![ScriptedResponse::json(body.to_string())]);

    let response = provider(&server, 1024 * 1024, 0)
        .evaluate(&request)
        .expect("a rounded distribution is repaired, not rejected");
    let Some(Answer::Choice(choice)) = response.answers().get(&qid("concern")) else {
        panic!("concern must be a choice answer");
    };
    let sum: f64 = choice.probabilities().values().map(|p| p.get()).sum();
    assert!((sum - 1.0).abs() < 1e-12);
}

#[test]
fn a_choice_without_a_label_derives_the_argmax() {
    let request = DecisionRequest::new(
        json!({}),
        vec![Question::Choice(
            ChoiceQuestion::new(
                qid("concern"),
                "Which concern?",
                vec![option("api-contract"), option("other")],
            )
            .expect("valid"),
        )],
    )
    .expect("valid request");
    let body = json!({
        "model": "laya-rl-agent",
        "answers": {
            "concern": {
                "type": "choice",
                "probabilities": { "api-contract": 0.3, "other": 0.7 },
                "confidence": 0.7
            }
        },
        "usage": { "input_tokens": 1, "output_tokens": 0 }
    });
    let server = MockServer::scripted(vec![ScriptedResponse::json(body.to_string())]);

    let response = provider(&server, 1024 * 1024, 0)
        .evaluate(&request)
        .expect("evaluate succeeds");
    let Some(Answer::Choice(choice)) = response.answers().get(&qid("concern")) else {
        panic!("concern must be a choice answer");
    };
    assert_eq!(choice.selected(), "other");
}

#[test]
fn the_model_name_is_used_when_routing_is_absent() {
    let mut body = valid_response_body();
    body.as_object_mut().expect("object").remove("routing");
    let server = MockServer::scripted(vec![ScriptedResponse::json(body.to_string())]);

    let response = provider(&server, 1024 * 1024, 0)
        .evaluate(&sample_request())
        .expect("evaluate succeeds");
    assert_eq!(response.model_revision(), "laya-rl-agent");
}

#[test]
fn a_missing_answer_is_an_invalid_response() {
    let body = json!({
        "model": "laya-rl-agent",
        "answers": {
            "concern": {
                "type": "choice",
                "choice": "api-contract",
                "probabilities": { "api-contract": 0.8, "other": 0.2 },
                "confidence": 0.7
            }
        },
        "usage": { "input_tokens": 1, "output_tokens": 0 }
    });
    assert!(matches!(
        invalid_source(body),
        ValidationError::MissingAnswer { .. }
    ));
}

#[test]
fn an_unexpected_answer_is_an_invalid_response() {
    let mut body = valid_response_body();
    body["answers"]["surprise"] = json!({ "type": "noul", "noul": 0.5 });
    assert!(matches!(
        invalid_source(body),
        ValidationError::UnexpectedAnswer { .. }
    ));
}

#[test]
fn a_wrong_answer_type_is_an_invalid_response() {
    let mut body = valid_response_body();
    body["answers"]["concern"] = json!({ "type": "noul", "noul": 0.5 });
    assert!(matches!(
        invalid_source(body),
        ValidationError::WrongAnswerType { .. }
    ));
}

#[test]
fn a_score_with_the_wrong_number_of_levels_is_an_invalid_response() {
    let mut body = valid_response_body();
    body["answers"]["risk"]["probabilities"] = json!({ "0": 1.0 });
    assert!(matches!(
        invalid_source(body),
        ValidationError::OptionMismatch { .. }
    ));
}

#[test]
fn authentication_errors_are_typed() {
    let server = MockServer::always(ScriptedResponse::status(401));
    let error = provider(&server, 1024 * 1024, 0)
        .evaluate(&sample_request())
        .expect_err("401 is an error");
    assert!(matches!(error, DecisionError::Authentication));
}

#[test]
fn a_retryable_status_is_retried_then_succeeds() {
    let server = MockServer::scripted(vec![
        ScriptedResponse::status(503),
        ScriptedResponse::json(valid_response_body().to_string()),
    ]);
    let response = provider(&server, 1024 * 1024, 2)
        .evaluate(&sample_request())
        .expect("the retry succeeds");
    assert_eq!(response.model_revision(), "english");
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn an_oversized_response_is_rejected() {
    let server = MockServer::always(ScriptedResponse::json(valid_response_body().to_string()));
    let error = provider(&server, 16, 0)
        .evaluate(&sample_request())
        .expect_err("the body is too large");
    assert!(matches!(
        error,
        DecisionError::ResponseTooLarge { limit_bytes: 16 }
    ));
}

#[test]
fn an_api_key_is_sent_as_a_bearer_header_only_when_configured() {
    let server = MockServer::always(ScriptedResponse::json(valid_response_body().to_string()));

    let mut config = LayaConfig::new(server.base_url(), "english");
    config.max_retries = 0;
    LayaProvider::new(config)
        .evaluate(&sample_request())
        .expect("evaluate succeeds");
    assert_eq!(
        server.requests()[0].header("authorization"),
        None,
        "an unsecured server needs no bearer token"
    );

    let mut config = LayaConfig::new(server.base_url(), "english");
    config.max_retries = 0;
    config.api_key = Some(Secret::new("local-secret"));
    LayaProvider::new(config)
        .evaluate(&sample_request())
        .expect("evaluate succeeds");
    assert_eq!(
        server.requests()[1].header("authorization"),
        Some("Bearer local-secret")
    );
}
