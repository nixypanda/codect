//! The optional blocking Laya decision provider.
//!
//! This module is compiled only behind the `laya` feature. It talks to a local
//! [`laya-serve`](https://github.com/NandhaKishorM/laya) process over blocking
//! HTTP with `ureq`; it never creates an async runtime and it never logs request
//! headers, request state, or response bodies.
//!
//! # Wire protocol
//!
//! `laya-serve` speaks the public TypeSafe Jev `POST /v1/systemone` shape. One
//! evaluation is one `POST` to the configured endpoint with
//! `Content-Type: application/json` and an optional
//! `Authorization: Bearer <key>`:
//!
//! ```json
//! {
//!   "model": "english",
//!   "state": { "schema": "ownai.review-unit.v1", "...": "..." },
//!   "questions": {
//!     "concern": {
//!       "type": "choice",
//!       "instructions": "...",
//!       "criteria": { "api-contract": "...", "other": "..." }
//!     },
//!     "risk": {
//!       "type": "score",
//!       "instructions": "...",
//!       "criteria": ["routine ...", "low ..."]
//!     },
//!     "likely_breaking": { "type": "noul", "instructions": "..." }
//!   }
//! }
//! ```
//!
//! `model` names a Laya checkpoint (`english`, `multilingual`,
//! `typed-decisions`); any other value is ignored by the server and the router
//! auto-selects. A successful (`2xx`) response is:
//!
//! ```json
//! {
//!   "model": "laya-rl-agent",
//!   "answers": {
//!     "concern": {
//!       "type": "choice",
//!       "choice": "api-contract",
//!       "probabilities": { "api-contract": 0.8, "other": 0.2 },
//!       "confidence": 0.7
//!     },
//!     "risk": {
//!       "type": "score",
//!       "score": 0.5,
//!       "legend": { "0": "routine ...", "1": "low ..." },
//!       "probabilities": { "0": 0.6, "1": 0.4 },
//!       "confidence": 0.6
//!     },
//!     "likely_breaking": { "type": "noul", "noul": 0.71 }
//!   },
//!   "usage": { "input_tokens": 100, "output_tokens": 0 },
//!   "routing": { "model": "english", "repo": "...", "reason": "..." }
//! }
//! ```
//!
//! Laya's score probabilities are keyed by level index (`"0"`..`"n-1"`) and its
//! `score` is the expected level in that index space, which is exactly OwnAI's
//! `raw_index`. The adapter maps the index keys onto the request's level labels
//! by position and renormalizes the distribution, because Laya rounds each
//! probability to four decimals and the rounded sum can fall outside the shared
//! model's `1e-4` tolerance for more than a few options.
//!
//! # Failure handling
//!
//! The body is read through a hard byte limit before any parsing. `401`/`403`
//! map to [`DecisionError::Authentication`]; `429` and `502`/`503`/`504` are
//! retried up to [`LayaConfig::max_retries`] extra times, sleeping a
//! server-provided `Retry-After` (capped at five seconds) when one is present.
//! Other `4xx`/`5xx` statuses become [`DecisionError::Http`]. A timeout becomes
//! [`DecisionError::Timeout`] and any other transport failure becomes
//! [`DecisionError::Transport`]. A `2xx` body that is not a valid, matching
//! response is never retried.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    Answer, ChoiceAnswer, DecisionError, DecisionProvider, DecisionRequest, DecisionResponse,
    NoulAnswer, Probability, Question, QuestionId, ScoreAnswer, ScoreQuestion, Secret, Usage,
    ValidationError,
};

/// The default local endpoint when none is configured.
pub const DEFAULT_LAYA_ENDPOINT: &str = "http://127.0.0.1:8000/v1/systemone";

/// The default checkpoint name when neither a flag nor the environment supplies
/// one. The server routes English state to this checkpoint.
pub const DEFAULT_LAYA_MODEL: &str = "english";

/// The provider attribution recorded in every result.
const LAYA_PROVIDER: &str = "laya";

/// The model revision used when the server reports neither a selected
/// checkpoint nor a model name.
const DEFAULT_LAYA_MODEL_REVISION: &str = "laya-rl-agent";

/// The default maximum response body in bytes (1 MiB).
const DEFAULT_MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// The default number of retries after the first attempt.
const DEFAULT_MAX_RETRIES: u32 = 2;

/// The default timeout for establishing a connection.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The default timeout for a complete request, including reading the body.
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The greatest server-requested retry delay this client will honour.
const RETRY_AFTER_CAP: Duration = Duration::from_secs(5);

/// Configuration for [`LayaProvider`].
///
/// Fields are public so a caller can tune limits and timeouts. The optional API
/// key is a [`Secret`] and is never read from flags or configuration by the CLI;
/// when present it comes only from the `LAYA_API_KEY` environment variable.
pub struct LayaConfig {
    /// The endpoint URL, including scheme, host, and path.
    pub endpoint: String,
    /// The checkpoint name sent in the request.
    pub model: String,
    /// The optional bearer credential required by a secured server.
    pub api_key: Option<Secret>,
    /// The maximum response body read before parsing.
    pub max_response_bytes: usize,
    /// The number of retries after the first attempt for transient failures.
    pub max_retries: u32,
    /// The timeout for establishing a connection.
    pub connect_timeout: Duration,
    /// The timeout for a complete request, including reading the response body.
    pub request_timeout: Duration,
}

impl LayaConfig {
    /// Builds a configuration with the documented defaults for limits and
    /// timeouts and no API key.
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            model: model.into(),
            api_key: None,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            max_retries: DEFAULT_MAX_RETRIES,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }
}

impl Default for LayaConfig {
    /// Uses [`DEFAULT_LAYA_ENDPOINT`], [`DEFAULT_LAYA_MODEL`], no key, a 1 MiB
    /// response limit, two retries, a 10 s connect timeout, and a 30 s request
    /// timeout.
    fn default() -> Self {
        Self::new(DEFAULT_LAYA_ENDPOINT, DEFAULT_LAYA_MODEL)
    }
}

/// A synchronous Laya provider over blocking HTTP.
///
/// The underlying [`ureq::Agent`] is cheaply cloneable and `Send + Sync`, so the
/// provider satisfies the [`DecisionProvider`] boundary without an async
/// runtime.
pub struct LayaProvider {
    agent: ureq::Agent,
    config: LayaConfig,
}

impl LayaProvider {
    /// Builds a provider and its connection-pooling agent.
    ///
    /// HTTP status codes are returned as responses rather than errors so a
    /// `Retry-After` header can be read from `429` and `503` replies.
    pub fn new(config: LayaConfig) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(config.connect_timeout))
            .timeout_global(Some(config.request_timeout))
            .timeout_recv_body(Some(config.request_timeout))
            .build()
            .new_agent();
        Self { agent, config }
    }

    /// Builds the Jev-shaped wire request body from the shared request.
    fn request_body(&self, request: &DecisionRequest) -> Value {
        let mut questions = Map::new();
        for question in request.questions() {
            questions.insert(question.id().to_string(), question_body(question));
        }
        json!({
            "model": self.config.model.as_str(),
            "state": request.state(),
            "questions": Value::Object(questions),
        })
    }

    /// Sends one attempt and classifies its outcome.
    ///
    /// The bounded body is always read before the status is interpreted, so an
    /// oversized reply is rejected identically for success and failure states.
    fn send_once(&self, body: &Value) -> Result<WireResponse, Attempt> {
        let mut builder = self
            .agent
            .post(&self.config.endpoint)
            .header("Content-Type", "application/json");
        if let Some(key) = &self.config.api_key
            && !key.expose().is_empty()
        {
            builder = builder.header("Authorization", format!("Bearer {}", key.expose()));
        }

        let mut response = builder.send_json(body).map_err(transport_error)?;

        let status = response.status().as_u16();
        let retry_after = retry_after(&response);
        let bytes =
            read_bounded(&mut response, self.config.max_response_bytes).map_err(Attempt::Fatal)?;

        if is_retryable(status) {
            return Err(Attempt::Retry {
                status,
                retry_after,
            });
        }
        if (200..=299).contains(&status) {
            return serde_json::from_slice(&bytes).map_err(|error| {
                Attempt::Fatal(DecisionError::invalid_response(
                    ValidationError::MalformedResponse {
                        message: error.to_string(),
                    },
                ))
            });
        }
        Err(Attempt::Fatal(status_error(status)))
    }
}

impl DecisionProvider for LayaProvider {
    fn evaluate(&self, request: &DecisionRequest) -> Result<DecisionResponse, DecisionError> {
        let body = self.request_body(request);
        let mut attempt = 0;
        loop {
            match self.send_once(&body) {
                Ok(wire) => return decode(request, wire),
                Err(Attempt::Retry {
                    status,
                    retry_after,
                }) => {
                    if attempt >= self.config.max_retries {
                        return Err(if status == 429 {
                            DecisionError::RateLimited { retry_after }
                        } else {
                            DecisionError::Http {
                                status,
                                message: status_text(status).to_owned(),
                            }
                        });
                    }
                    attempt += 1;
                    if let Some(delay) = retry_after
                        && !delay.is_zero()
                    {
                        std::thread::sleep(delay);
                    }
                }
                Err(Attempt::Fatal(error)) => return Err(error),
            }
        }
    }
}

/// The outcome of one send attempt that is not a parsed success.
enum Attempt {
    /// A transient status the caller may retry.
    Retry {
        status: u16,
        retry_after: Option<Duration>,
    },
    /// A failure that must be returned to the caller unchanged.
    Fatal(DecisionError),
}

/// Maps a `ureq` transport failure, keeping timeouts distinct.
fn transport_error(error: ureq::Error) -> Attempt {
    match error {
        ureq::Error::Timeout(_) => Attempt::Fatal(DecisionError::Timeout),
        _ => Attempt::Fatal(DecisionError::Transport {
            message: error.to_string(),
        }),
    }
}

/// Whether a status is retried by the provider.
fn is_retryable(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504)
}

/// Maps a non-retryable, non-success status to a typed error.
fn status_error(status: u16) -> DecisionError {
    match status {
        401 | 403 => DecisionError::Authentication,
        _ => DecisionError::Http {
            status,
            message: status_text(status).to_owned(),
        },
    }
}

/// A short, stable status description that contains no request data.
fn status_text(status: u16) -> &'static str {
    match status {
        400 => "bad request",
        401 => "unauthorized",
        403 => "forbidden",
        404 => "not found",
        405 => "method not allowed",
        409 => "conflict",
        422 => "unprocessable entity",
        429 => "too many requests",
        500 => "internal server error",
        502 => "bad gateway",
        503 => "service unavailable",
        504 => "gateway timeout",
        _ => "request failed",
    }
}

/// Reads a server-provided integer `Retry-After` in seconds, capped at five.
fn retry_after(response: &ureq::http::Response<ureq::Body>) -> Option<Duration> {
    let value = response.headers().get("retry-after")?.to_str().ok()?;
    parse_retry_after(value)
}

/// Parses an integer-seconds `Retry-After` value and applies the cap.
fn parse_retry_after(value: &str) -> Option<Duration> {
    let seconds: u64 = value.trim().parse().ok()?;
    Some(RETRY_AFTER_CAP.min(Duration::from_secs(seconds)))
}

/// Reads at most `limit + 1` bytes so a reply over the limit is detected
/// without buffering the whole body.
fn read_bounded(
    response: &mut ureq::http::Response<ureq::Body>,
    limit: usize,
) -> Result<Vec<u8>, DecisionError> {
    use std::io::Read as _;

    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| DecisionError::Transport {
            message: error.to_string(),
        })?;
    if bytes.len() > limit {
        return Err(DecisionError::ResponseTooLarge { limit_bytes: limit });
    }
    Ok(bytes)
}

/// Builds one Jev-shaped question from a validated question.
fn question_body(question: &Question) -> Value {
    match question {
        Question::Choice(choice) => {
            let mut criteria = Map::new();
            for option in choice.options() {
                let text = option.description().unwrap_or_else(|| option.key());
                criteria.insert(option.key().to_owned(), Value::String(text.to_owned()));
            }
            json!({
                "type": "choice",
                "instructions": choice.instructions(),
                "criteria": Value::Object(criteria),
            })
        }
        Question::Score(score) => {
            let criteria: Vec<&str> = score
                .levels()
                .iter()
                .map(|level| level.description().unwrap_or_else(|| level.label()))
                .collect();
            json!({
                "type": "score",
                "instructions": score.instructions(),
                "criteria": criteria,
            })
        }
        Question::Noul(noul) => json!({
            "type": "noul",
            "instructions": noul.instructions(),
        }),
    }
}

/// Decodes a parsed wire response into the validated shared model.
fn decode(
    request: &DecisionRequest,
    wire: WireResponse,
) -> Result<DecisionResponse, DecisionError> {
    // The selected checkpoint (`routing.model`) is the most specific revision;
    // fall back to the model name, then to the runtime default.
    let model_revision = wire
        .routing
        .as_ref()
        .and_then(|routing| routing.model.clone())
        .or_else(|| wire.model.clone())
        .unwrap_or_else(|| DEFAULT_LAYA_MODEL_REVISION.to_owned());

    let mut remaining = wire.answers;
    let usage = wire.usage.map(|usage| Usage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    });

    let mut answers = Vec::with_capacity(request.questions().len());
    for question in request.questions() {
        let id = question.id();
        let Some(wire_answer) = remaining.remove(id.as_str()) else {
            return Err(DecisionError::invalid_response(
                ValidationError::MissingAnswer { id: id.to_string() },
            ));
        };
        let answer =
            decode_answer(id, question, wire_answer).map_err(DecisionError::invalid_response)?;
        answers.push((id.clone(), answer));
    }

    // `validate_for` would not see answers the request never asked about,
    // because they were dropped above, so reject them explicitly.
    if let Some((id, _)) = remaining.into_iter().next() {
        return Err(DecisionError::invalid_response(
            ValidationError::UnexpectedAnswer { id },
        ));
    }

    let response = DecisionResponse::new(LAYA_PROVIDER, model_revision, answers, usage)
        .map_err(DecisionError::invalid_response)?;
    response
        .validate_for(request)
        .map_err(DecisionError::invalid_response)?;
    Ok(response)
}

/// Decodes one tagged wire answer against the question it answers.
fn decode_answer(
    id: &QuestionId,
    question: &Question,
    wire: WireAnswer,
) -> Result<Answer, ValidationError> {
    match (question, wire) {
        (
            Question::Choice(_),
            WireAnswer::Choice {
                choice,
                probabilities,
                confidence,
            },
        ) => decode_choice(choice, probabilities, confidence),
        (
            Question::Score(question),
            WireAnswer::Score {
                score,
                probabilities,
                confidence,
            },
        ) => decode_score(id, question, score, probabilities, confidence),
        (Question::Noul(_), WireAnswer::Noul { noul, .. }) => {
            Ok(Answer::Noul(NoulAnswer::new(Probability::new(noul)?)))
        }
        (question, wire) => Err(ValidationError::WrongAnswerType {
            id: id.to_string(),
            expected: question.kind().as_str(),
            actual: wire.kind(),
        }),
    }
}

/// Decodes a choice answer, deriving `selected` from the argmax when absent.
fn decode_choice(
    choice: Option<String>,
    probabilities: BTreeMap<String, f64>,
    confidence: f64,
) -> Result<Answer, ValidationError> {
    let probabilities = renormalize(probabilities.into_iter().collect())?;
    let selected = match choice {
        Some(selected) => selected,
        None => argmax(&probabilities)?,
    };
    Ok(Answer::Choice(ChoiceAnswer::new(
        selected,
        probabilities,
        Probability::new(confidence)?,
    )?))
}

/// Decodes a score answer, mapping Laya's index keys onto level labels.
fn decode_score(
    id: &QuestionId,
    question: &ScoreQuestion,
    score: f64,
    probabilities: BTreeMap<String, f64>,
    confidence: f64,
) -> Result<Answer, ValidationError> {
    let levels = question.levels();
    if probabilities.len() != levels.len() {
        return Err(ValidationError::OptionMismatch { id: id.to_string() });
    }
    let mut raw = Vec::with_capacity(levels.len());
    for (index, level) in levels.iter().enumerate() {
        let Some(value) = probabilities.get(&index.to_string()) else {
            return Err(ValidationError::OptionMismatch { id: id.to_string() });
        };
        raw.push((level.label().to_owned(), *value));
    }
    Ok(Answer::Score(ScoreAnswer::new(
        score,
        renormalize(raw)?,
        Probability::new(confidence)?,
    )?))
}

/// Normalizes a probability distribution to sum to exactly one.
///
/// Laya rounds each probability to four decimals, so a raw distribution can
/// miss the shared model's sum tolerance. Dividing by the finite, positive sum
/// restores a valid distribution without changing the ranking.
fn renormalize(entries: Vec<(String, f64)>) -> Result<Vec<(String, Probability)>, ValidationError> {
    if entries.is_empty() {
        return Err(ValidationError::EmptyDistribution);
    }
    let sum: f64 = entries.iter().map(|(_, value)| *value).sum();
    if !sum.is_finite() || sum <= 0.0 {
        return Err(ValidationError::ProbabilitySum {
            sum: sum.to_string(),
        });
    }
    entries
        .into_iter()
        .map(|(key, value)| {
            let normalized = (value / sum).clamp(0.0, 1.0);
            Ok((key, Probability::new(normalized)?))
        })
        .collect()
}

/// Returns the lexicographically smallest key among the distribution's maxima.
///
/// `ChoiceAnswer::new` requires `selected` to be tied for the maximum, so a
/// supplied label that is not the maximum is rejected there instead. The input
/// is key-sorted because it comes from a [`BTreeMap`], so the first maximum seen
/// is the lexicographically smallest.
fn argmax(probabilities: &[(String, Probability)]) -> Result<String, ValidationError> {
    let mut best: Option<&(String, Probability)> = None;
    for entry in probabilities {
        match best {
            Some(current) if entry.1.get() <= current.1.get() => {}
            _ => best = Some(entry),
        }
    }
    best.map(|(key, _)| key.clone())
        .ok_or(ValidationError::EmptyDistribution)
}

/// The private wire response shape.
#[derive(Deserialize)]
struct WireResponse {
    #[serde(default)]
    model: Option<String>,
    answers: BTreeMap<String, WireAnswer>,
    #[serde(default)]
    usage: Option<WireUsage>,
    #[serde(default)]
    routing: Option<WireRouting>,
}

/// The private routing metadata shape. Only the selected checkpoint is read.
#[derive(Deserialize)]
struct WireRouting {
    #[serde(default)]
    model: Option<String>,
}

/// The private tagged wire answer shape.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum WireAnswer {
    Choice {
        #[serde(default)]
        choice: Option<String>,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
    Score {
        score: f64,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
    Noul {
        noul: f64,
    },
}

impl WireAnswer {
    /// The wire answer kind, for a wrong-type diagnostic.
    fn kind(&self) -> &'static str {
        match self {
            Self::Choice { .. } => "choice",
            Self::Score { .. } => "score",
            Self::Noul { .. } => "noul",
        }
    }
}

/// The private wire usage shape.
#[derive(Deserialize)]
struct WireUsage {
    input_tokens: u64,
    output_tokens: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChoiceOption, ChoiceQuestion, NoulQuestion, ScoreLevel};

    fn id(value: &str) -> QuestionId {
        QuestionId::new(value).expect("valid question id")
    }

    #[test]
    fn config_defaults_are_documented() {
        let config = LayaConfig::default();
        assert_eq!(config.endpoint, DEFAULT_LAYA_ENDPOINT);
        assert_eq!(config.model, DEFAULT_LAYA_MODEL);
        assert!(config.api_key.is_none());
        assert_eq!(config.max_response_bytes, 1024 * 1024);
        assert_eq!(config.max_retries, 2);
        assert_eq!(config.connect_timeout, Duration::from_secs(10));
        assert_eq!(config.request_timeout, Duration::from_secs(30));
    }

    #[test]
    fn choice_questions_send_key_to_description_criteria() {
        let question = Question::Choice(
            ChoiceQuestion::new(
                id("concern"),
                "Which concern?",
                vec![
                    ChoiceOption::new("api-contract", Some("contract changes".to_owned())).unwrap(),
                    ChoiceOption::new("other", None).unwrap(),
                ],
            )
            .unwrap(),
        );
        let body = question_body(&question);
        assert_eq!(body["type"], "choice");
        assert_eq!(body["criteria"]["api-contract"], "contract changes");
        assert_eq!(body["criteria"]["other"], "other");
    }

    #[test]
    fn score_questions_send_an_ordered_criteria_list() {
        let question = Question::Score(
            ScoreQuestion::new(
                id("risk"),
                "How risky?",
                vec![
                    ScoreLevel::new("routine", Some("local".to_owned())).unwrap(),
                    ScoreLevel::new("low", None).unwrap(),
                ],
            )
            .unwrap(),
        );
        let body = question_body(&question);
        assert_eq!(body["type"], "score");
        assert_eq!(body["criteria"], json!(["local", "low"]));
    }

    #[test]
    fn noul_questions_send_no_criteria() {
        let question = Question::Noul(NoulQuestion::new(id("needs_tests"), "Tests?").unwrap());
        let body = question_body(&question);
        assert_eq!(body["type"], "noul");
        assert!(body.get("criteria").is_none());
    }

    #[test]
    fn renormalize_repairs_a_rounded_distribution() {
        let entries = vec![
            ("a".to_owned(), 0.3334),
            ("b".to_owned(), 0.3333),
            ("c".to_owned(), 0.3333),
        ];
        let normalized = renormalize(entries).expect("renormalizes");
        let sum: f64 = normalized.iter().map(|(_, p)| p.get()).sum();
        assert!((sum - 1.0).abs() < 1e-12);
    }

    #[test]
    fn renormalize_rejects_a_non_positive_sum() {
        let entries = vec![("a".to_owned(), 0.0), ("b".to_owned(), 0.0)];
        assert!(matches!(
            renormalize(entries),
            Err(ValidationError::ProbabilitySum { .. })
        ));
    }

    #[test]
    fn retryable_statuses_are_the_documented_set() {
        for status in [429, 502, 503, 504] {
            assert!(is_retryable(status), "{status} must be retryable");
        }
        for status in [200, 400, 401, 403, 404, 422, 500, 501] {
            assert!(!is_retryable(status), "{status} must not be retryable");
        }
    }

    #[test]
    fn a_bare_string_retry_after_is_parsed_and_capped() {
        assert_eq!(parse_retry_after("0"), Some(Duration::from_secs(0)));
        assert_eq!(parse_retry_after("2"), Some(Duration::from_secs(2)));
        assert_eq!(parse_retry_after("60"), Some(Duration::from_secs(5)));
        assert_eq!(parse_retry_after("later"), None);
    }
}
