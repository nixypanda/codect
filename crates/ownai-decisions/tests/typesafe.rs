//! Contract tests for the TypeSafe decision provider against a local mock
//! HTTP server. No test here touches the network or needs a credential.

#![cfg(feature = "typesafe")]

mod support;

use std::time::Duration;

use ownai_decisions::{
    Answer, ChoiceOption, ChoiceQuestion, DecisionError, DecisionProvider, DecisionRequest,
    NoulQuestion, Question, QuestionId, ScoreLevel, ScoreQuestion, Secret, TypeSafeConfig,
    TypeSafeProvider, Usage, ValidationError,
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

/// A valid response body covering the sample request's three answers.
fn valid_response_body() -> Value {
    json!({
        "provider": "typesafe",
        "model_revision": "system-one-2026-01-15",
        "answers": {
            "concern": {
                "type": "choice",
                "choice": "api-contract",
                "probabilities": { "api-contract": 0.8, "other": 0.2 },
                "confidence": 0.7
            },
            "risk": {
                "type": "score",
                "score": 0.5,
                "probabilities": { "routine": 0.6, "low": 0.4 },
                "confidence": 0.6
            },
            "likely_breaking": { "type": "noul", "noul": 0.71 }
        },
        "usage": { "input_tokens": 100, "output_tokens": 10 }
    })
}

/// Builds a provider pointed at a mock server.
fn provider(server: &MockServer, max_response_bytes: usize, max_retries: u32) -> TypeSafeProvider {
    let mut config = TypeSafeConfig::new(server.base_url(), "system-one", Secret::new("test-key"));
    config.max_response_bytes = max_response_bytes;
    config.max_retries = max_retries;
    config.connect_timeout = Duration::from_secs(5);
    config.request_timeout = Duration::from_secs(5);
    TypeSafeProvider::new(config)
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

    assert_eq!(response.provider(), "typesafe");
    assert_eq!(response.model_revision(), "system-one-2026-01-15");
    assert_eq!(
        response.usage(),
        Some(Usage {
            input_tokens: 100,
            output_tokens: 10,
        })
    );

    let Some(Answer::Choice(concern)) = response.answers().get(&qid("concern")) else {
        panic!("concern must be a choice answer");
    };
    assert_eq!(concern.selected(), "api-contract");
    assert_eq!(concern.confidence().get(), 0.7);
    assert_eq!(
        concern.probabilities().get("api-contract").map(|p| p.get()),
        Some(0.8)
    );

    let Some(Answer::Score(risk)) = response.answers().get(&qid("risk")) else {
        panic!("risk must be a score answer");
    };
    assert_eq!(risk.value(), 0.5);
    assert_eq!(risk.confidence().get(), 0.6);

    let Some(Answer::Noul(noul)) = response.answers().get(&qid("likely_breaking")) else {
        panic!("likely_breaking must be a noul answer");
    };
    assert_eq!(noul.probability().get(), 0.71);

    response
        .validate_for(&request)
        .expect("matches its request");
}

#[test]
fn the_recorded_request_carries_auth_content_type_model_state_and_questions() {
    let server = MockServer::scripted(vec![ScriptedResponse::json(
        valid_response_body().to_string(),
    )]);
    let request = sample_request();
    provider(&server, 1024 * 1024, 2)
        .evaluate(&request)
        .expect("evaluate succeeds");

    let recorded = server.requests();
    assert_eq!(recorded.len(), 1);
    let recorded = &recorded[0];

    assert_eq!(recorded.method, "POST");
    assert_eq!(recorded.header("authorization"), Some("Bearer test-key"));
    assert_eq!(recorded.header("content-type"), Some("application/json"));

    let body: Value = serde_json::from_str(&recorded.body).expect("request body is JSON");
    assert_eq!(body["model"], json!("system-one"));
    assert_eq!(body["state"], *request.state());

    let questions = body["questions"].as_array().expect("questions array");
    let ids: Vec<&str> = questions
        .iter()
        .map(|question| question["id"].as_str().expect("question id"))
        .collect();
    assert_eq!(ids, vec!["concern", "risk", "likely_breaking"]);

    assert_eq!(questions[0]["type"], json!("choice"));
    assert_eq!(questions[0]["instructions"], json!("Which concern?"));
    assert_eq!(questions[0]["options"][0]["key"], json!("api-contract"));
    assert_eq!(questions[0]["options"][0]["description"], Value::Null);
    assert_eq!(questions[1]["type"], json!("score"));
    assert_eq!(questions[1]["levels"][1]["label"], json!("low"));
    assert_eq!(questions[2]["type"], json!("noul"));
    assert_eq!(questions[2]["instructions"], json!("Is it breaking?"));
}

#[test]
fn unauthorized_and_forbidden_are_authentication_errors() {
    for status in [401, 403] {
        let server = MockServer::scripted(vec![ScriptedResponse::status(status)]);
        let error = provider(&server, 1024 * 1024, 2)
            .evaluate(&sample_request())
            .expect_err("the provider refuses");
        assert!(
            matches!(error, DecisionError::Authentication),
            "{status} must map to Authentication, got {error:?}"
        );
        assert_eq!(server.requests().len(), 1, "auth failures are not retried");
    }
}

#[test]
fn a_rate_limit_with_a_zero_retry_after_succeeds_after_one_retry() {
    let server = MockServer::scripted(vec![
        ScriptedResponse::status(429).with_header("Retry-After", "0"),
        ScriptedResponse::json(valid_response_body().to_string()),
    ]);

    let response = provider(&server, 1024 * 1024, 2)
        .evaluate(&sample_request())
        .expect("evaluate succeeds after a retry");

    assert_eq!(response.model_revision(), "system-one-2026-01-15");
    assert_eq!(server.requests().len(), 2, "one retry was made");
}

#[test]
fn a_persistent_rate_limit_is_reported_after_the_retry_budget() {
    let server = MockServer::always(ScriptedResponse::status(429).with_header("Retry-After", "0"));

    let error = provider(&server, 1024 * 1024, 2)
        .evaluate(&sample_request())
        .expect_err("the provider gives up");

    match error {
        DecisionError::RateLimited { retry_after } => {
            assert_eq!(retry_after, Some(Duration::ZERO));
        }
        other => panic!("expected RateLimited, got {other:?}"),
    }
    assert_eq!(server.requests().len(), 3, "one attempt plus two retries");
}

#[test]
fn a_transient_gateway_error_is_retried() {
    let server = MockServer::scripted(vec![
        ScriptedResponse::status(503),
        ScriptedResponse::json(valid_response_body().to_string()),
    ]);

    let response = provider(&server, 1024 * 1024, 2)
        .evaluate(&sample_request())
        .expect("evaluate succeeds after a retry");

    assert_eq!(response.model_revision(), "system-one-2026-01-15");
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn a_persistent_server_error_is_an_http_error_and_is_not_retried() {
    let server = MockServer::always(ScriptedResponse::status(500));

    let error = provider(&server, 1024 * 1024, 2)
        .evaluate(&sample_request())
        .expect_err("the provider reports the failure");

    match error {
        DecisionError::Http { status, .. } => assert_eq!(status, 500),
        other => panic!("expected Http, got {other:?}"),
    }
    assert_eq!(server.requests().len(), 1, "500 is not retried");
}

#[test]
fn an_oversized_body_is_rejected_before_parsing() {
    let server = MockServer::scripted(vec![ScriptedResponse::json("x".repeat(64))]);

    let error = provider(&server, 16, 0)
        .evaluate(&sample_request())
        .expect_err("the body exceeds the limit");

    match error {
        DecisionError::ResponseTooLarge { limit_bytes } => assert_eq!(limit_bytes, 16),
        other => panic!("expected ResponseTooLarge, got {other:?}"),
    }
}

#[test]
fn malformed_json_is_an_invalid_response() {
    let server = MockServer::scripted(vec![ScriptedResponse::json("{ not json")]);

    let error = provider(&server, 1024 * 1024, 0)
        .evaluate(&sample_request())
        .expect_err("the body is not JSON");

    assert!(
        matches!(
            error,
            DecisionError::InvalidResponse {
                source: ValidationError::MalformedResponse { .. }
            }
        ),
        "expected MalformedResponse, got {error:?}"
    );
}

#[test]
fn a_missing_answer_is_rejected_by_validate_for() {
    let mut body = valid_response_body();
    body["answers"]
        .as_object_mut()
        .expect("answers object")
        .remove("likely_breaking");
    assert!(matches!(
        invalid_source(body),
        ValidationError::MissingAnswer { .. }
    ));
}

#[test]
fn an_extra_answer_is_rejected_by_validate_for() {
    let mut body = valid_response_body();
    body["answers"]["surprise"] = json!({ "type": "noul", "noul": 0.5 });
    assert!(matches!(
        invalid_source(body),
        ValidationError::UnexpectedAnswer { .. }
    ));
}

#[test]
fn a_wrong_answer_kind_is_rejected_by_validate_for() {
    let mut body = valid_response_body();
    body["answers"]["concern"] = json!({ "type": "noul", "noul": 0.5 });
    assert!(matches!(
        invalid_source(body),
        ValidationError::WrongAnswerType { .. }
    ));
}

#[test]
fn an_option_mismatch_is_rejected_by_validate_for() {
    let mut body = valid_response_body();
    body["answers"]["concern"] = json!({
        "type": "choice",
        "choice": "api",
        "probabilities": { "api": 0.8, "other": 0.2 },
        "confidence": 0.7
    });
    assert!(matches!(
        invalid_source(body),
        ValidationError::OptionMismatch { .. }
    ));
}

#[test]
fn a_supplied_choice_that_is_not_the_maximum_is_invalid() {
    let mut body = valid_response_body();
    body["answers"]["concern"] = json!({
        "type": "choice",
        "choice": "other",
        "probabilities": { "api-contract": 0.8, "other": 0.2 },
        "confidence": 0.7
    });
    assert!(matches!(
        invalid_source(body),
        ValidationError::SelectedChoiceNotMaximum { .. }
    ));
}

#[test]
fn an_absent_choice_is_computed_from_the_argmax() {
    let mut body = valid_response_body();
    body["answers"]["concern"] = json!({
        "type": "choice",
        "probabilities": { "api-contract": 0.2, "other": 0.8 },
        "confidence": 0.7
    });

    let server = MockServer::scripted(vec![ScriptedResponse::json(body.to_string())]);
    let response = provider(&server, 1024 * 1024, 0)
        .evaluate(&sample_request())
        .expect("evaluate succeeds");

    let Some(Answer::Choice(concern)) = response.answers().get(&qid("concern")) else {
        panic!("concern must be a choice answer");
    };
    assert_eq!(concern.selected(), "other");
}

#[test]
fn an_absent_choice_breaks_a_tie_by_ascending_key() {
    let mut body = valid_response_body();
    body["answers"]["concern"] = json!({
        "type": "choice",
        "probabilities": { "other": 0.5, "api-contract": 0.5 },
        "confidence": 0.7
    });

    let server = MockServer::scripted(vec![ScriptedResponse::json(body.to_string())]);
    let response = provider(&server, 1024 * 1024, 0)
        .evaluate(&sample_request())
        .expect("evaluate succeeds");

    let Some(Answer::Choice(concern)) = response.answers().get(&qid("concern")) else {
        panic!("concern must be a choice answer");
    };
    assert_eq!(concern.selected(), "api-contract");
}

#[test]
fn an_invalid_probability_is_rejected() {
    let mut body = valid_response_body();
    body["answers"]["likely_breaking"] = json!({ "type": "noul", "noul": 1.5 });
    assert!(matches!(
        invalid_source(body),
        ValidationError::InvalidProbability { .. }
    ));
}

#[test]
fn a_missing_model_revision_is_a_malformed_response() {
    let mut body = valid_response_body();
    body.as_object_mut()
        .expect("object")
        .remove("model_revision");

    // A shape failure fails during deserialization, before `InvalidResponse`.
    let server = MockServer::scripted(vec![ScriptedResponse::json(body.to_string())]);
    let error = provider(&server, 1024 * 1024, 0)
        .evaluate(&sample_request())
        .expect_err("model_revision is required");
    assert!(matches!(
        error,
        DecisionError::InvalidResponse {
            source: ValidationError::MalformedResponse { .. }
        }
    ));
}

#[test]
fn the_provider_defaults_and_the_provider_field() {
    let mut body = valid_response_body();
    body.as_object_mut().expect("object").remove("provider");

    let server = MockServer::scripted(vec![ScriptedResponse::json(body.to_string())]);
    let response = provider(&server, 1024 * 1024, 0)
        .evaluate(&sample_request())
        .expect("evaluate succeeds");
    assert_eq!(response.provider(), "typesafe");
}

#[test]
fn secret_debug_never_contains_the_key() {
    let secret = Secret::new("sk-test-super-secret");
    let rendered = format!("{secret:?}");
    assert!(!rendered.contains("sk-test-super-secret"), "{rendered}");
    assert_eq!(secret.expose(), "sk-test-super-secret");
}

#[test]
fn probabilities_are_validated_before_construction() {
    let mut body = valid_response_body();
    body["answers"]["risk"]["probabilities"] = json!({ "routine": 0.9, "low": 0.9 });
    assert!(matches!(
        invalid_source(body),
        ValidationError::ProbabilitySum { .. }
    ));
}
