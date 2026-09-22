//! The optional blocking TypeSafe/Jev decision provider.
//!
//! This module is compiled only behind the `typesafe` feature. It talks to a
//! remote service over blocking HTTPS with `ureq`; it never creates an async
//! runtime and it never logs request headers, request state, or response bodies.
//!
//! # Wire protocol
//!
//! The protocol is OwnAI's own contract; there is no public TypeSafe
//! specification. One evaluation is one `POST` to the configured endpoint with
//! `Authorization: Bearer <key>` and `Content-Type: application/json`:
//!
//! ```json
//! {
//!   "model": "<alias>",
//!   "state": { "schema": "ownai.review-unit.v1", "...": "..." },
//!   "questions": [
//!     { "type": "choice", "id": "concern", "instructions": "...",
//!       "options": [ { "key": "api-contract", "description": "..." } ] },
//!     { "type": "score", "id": "risk", "instructions": "...",
//!       "levels": [ { "label": "routine", "description": "..." } ] },
//!     { "type": "noul", "id": "likely_breaking", "instructions": "..." }
//!   ]
//! }
//! ```
//!
//! A successful (`2xx`) response is:
//!
//! ```json
//! {
//!   "provider": "typesafe",
//!   "model_revision": "system-one-2026-01-15",
//!   "answers": {
//!     "concern": {
//!       "type": "choice",
//!       "choice": "api-contract",
//!       "probabilities": { "api-contract": 0.8, "other": 0.2 },
//!       "confidence": 0.7
//!     },
//!     "risk": {
//!       "type": "score",
//!       "score": 2.5,
//!       "probabilities": { "routine": 0.1, "low": 0.9 },
//!       "confidence": 0.6
//!     },
//!     "likely_breaking": { "type": "noul", "noul": 0.71 }
//!   },
//!   "usage": { "input_tokens": 100, "output_tokens": 10 }
//! }
//! ```
//!
//! `provider` is optional and defaults to `"typesafe"`. `model_revision` is
//! required and must be non-empty. `usage` is optional. Every probability,
//! probability distribution, and answer kind is re-validated against the shared
//! model before it is returned, so a malformed body becomes a typed
//! [`DecisionError::InvalidResponse`] rather than a silently wrong judgment.
//!
//! # Failure handling
//!
//! The body is read through a hard byte limit before any parsing. `401`/`403`
//! map to [`DecisionError::Authentication`]; `429` and `502`/`503`/`504` are
//! retried up to [`TypeSafeConfig::max_retries`] extra times, sleeping a
//! server-provided `Retry-After` (capped at five seconds) when one is present.
//! Other `4xx`/`5xx` statuses become [`DecisionError::Http`]. A timeout becomes
//! [`DecisionError::Timeout`] and any other transport failure becomes
//! [`DecisionError::Transport`]. A `2xx` body that is not a valid, matching
//! response is never retried.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    Answer, ChoiceAnswer, DecisionError, DecisionProvider, DecisionRequest, DecisionResponse,
    NoulAnswer, Probability, Question, QuestionId, ScoreAnswer, Usage, ValidationError,
};

/// The default remote endpoint when none is configured.
pub const DEFAULT_TYPESAFE_ENDPOINT: &str = "https://api.typesafe.example/v1/system-one";

/// The default model alias when neither a flag nor the environment supplies one.
pub const DEFAULT_TYPESAFE_MODEL: &str = "system-one";

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

/// The default provider attribution when the wire response omits `provider`.
const DEFAULT_WIRE_PROVIDER: &str = "typesafe";

/// An API key that never appears in `Debug` output.
///
/// The value is deliberately not exposed by any formatting trait; callers that
/// need the bytes for an `Authorization` header call [`Secret::expose`].
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    /// Wraps an API key.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the raw key. The caller is responsible for never logging it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

/// Configuration for [`TypeSafeProvider`].
///
/// Fields are public so a caller can tune limits and timeouts. The API key is a
/// [`Secret`] and is never read from flags or configuration by the CLI; it comes
/// only from the `TYPESAFE_API_KEY` environment variable.
pub struct TypeSafeConfig {
    /// The endpoint URL, including scheme, host, and path.
    pub endpoint: String,
    /// The model alias sent in the request and retained in the response.
    pub model: String,
    /// The bearer credential.
    pub api_key: Secret,
    /// The maximum response body read before parsing.
    pub max_response_bytes: usize,
    /// The number of retries after the first attempt for transient failures.
    pub max_retries: u32,
    /// The timeout for establishing a connection.
    pub connect_timeout: Duration,
    /// The timeout for a complete request, including reading the response body.
    pub request_timeout: Duration,
}

impl TypeSafeConfig {
    /// Builds a configuration with the documented defaults for limits and
    /// timeouts.
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>, api_key: Secret) -> Self {
        Self {
            endpoint: endpoint.into(),
            model: model.into(),
            api_key,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            max_retries: DEFAULT_MAX_RETRIES,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }
}

impl Default for TypeSafeConfig {
    /// Uses [`DEFAULT_TYPESAFE_ENDPOINT`], [`DEFAULT_TYPESAFE_MODEL`], an empty
    /// key, a 1 MiB response limit, two retries, a 10 s connect timeout, and a
    /// 30 s request timeout.
    fn default() -> Self {
        Self::new(
            DEFAULT_TYPESAFE_ENDPOINT,
            DEFAULT_TYPESAFE_MODEL,
            Secret::new(String::new()),
        )
    }
}

/// A synchronous TypeSafe/Jev provider over blocking HTTPS.
///
/// The underlying [`ureq::Agent`] is cheaply cloneable and `Send + Sync`, so the
/// provider satisfies the [`DecisionProvider`] boundary without an async
/// runtime.
pub struct TypeSafeProvider {
    agent: ureq::Agent,
    config: TypeSafeConfig,
}

impl TypeSafeProvider {
    /// Builds a provider and its connection-pooling agent.
    ///
    /// HTTP status codes are returned as responses rather than errors so a
    /// `Retry-After` header can be read from `429` and `503` replies.
    pub fn new(config: TypeSafeConfig) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(config.connect_timeout))
            .timeout_global(Some(config.request_timeout))
            .timeout_recv_body(Some(config.request_timeout))
            .build()
            .new_agent();
        Self { agent, config }
    }

    /// Builds the wire request body from the shared request's public getters.
    fn request_body(&self, request: &DecisionRequest) -> Value {
        json!({
            "model": self.config.model.as_str(),
            "state": request.state(),
            "questions": request.questions().iter().map(question_body).collect::<Vec<_>>(),
        })
    }

    /// Sends one attempt and classifies its outcome.
    ///
    /// The bounded body is always read before the status is interpreted, so an
    /// oversized reply is rejected identically for success and failure states.
    fn send_once(&self, body: &Value) -> Result<WireResponse, Attempt> {
        let mut response = self
            .agent
            .post(&self.config.endpoint)
            .header(
                "Authorization",
                format!("Bearer {}", self.config.api_key.expose()),
            )
            .header("Content-Type", "application/json")
            .send_json(body)
            .map_err(transport_error)?;

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

impl DecisionProvider for TypeSafeProvider {
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
///
/// Only the integer-seconds form is understood, which is what the provider's
/// contract returns. A present but zero delay yields `Some(0)`; the caller does
/// not sleep for it.
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

/// Builds one wire question from a validated question.
fn question_body(question: &Question) -> Value {
    match question {
        Question::Choice(choice) => json!({
            "type": "choice",
            "id": choice.id().as_str(),
            "instructions": choice.instructions(),
            "options": choice.options().iter().map(|option| json!({
                "key": option.key(),
                "description": option.description(),
            })).collect::<Vec<_>>(),
        }),
        Question::Score(score) => json!({
            "type": "score",
            "id": score.id().as_str(),
            "instructions": score.instructions(),
            "levels": score.levels().iter().map(|level| json!({
                "label": level.label(),
                "description": level.description(),
            })).collect::<Vec<_>>(),
        }),
        Question::Noul(noul) => json!({
            "type": "noul",
            "id": noul.id().as_str(),
            "instructions": noul.instructions(),
        }),
    }
}

/// Decodes a parsed wire response into the validated shared model.
fn decode(
    request: &DecisionRequest,
    wire: WireResponse,
) -> Result<DecisionResponse, DecisionError> {
    let mut answers = Vec::with_capacity(wire.answers.len());
    for (raw_id, wire_answer) in wire.answers {
        let id = QuestionId::new(raw_id).map_err(DecisionError::invalid_response)?;
        let answer = decode_answer(wire_answer).map_err(DecisionError::invalid_response)?;
        answers.push((id, answer));
    }
    let usage = wire.usage.map(|usage| Usage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    });

    // `validate_for` is the defense-in-depth check: the lens also validates, but
    // the adapter must not return an answer set that does not match its request.
    let response = DecisionResponse::new(wire.provider, wire.model_revision, answers, usage)
        .map_err(DecisionError::invalid_response)?;
    response
        .validate_for(request)
        .map_err(DecisionError::invalid_response)?;
    Ok(response)
}

/// Decodes one tagged wire answer into a validated shared answer.
fn decode_answer(answer: WireAnswer) -> Result<Answer, ValidationError> {
    match answer {
        WireAnswer::Choice {
            choice,
            probabilities,
            confidence,
        } => {
            let probabilities = decode_probabilities(probabilities)?;
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
        WireAnswer::Score {
            score,
            probabilities,
            confidence,
        } => Ok(Answer::Score(ScoreAnswer::new(
            score,
            decode_probabilities(probabilities)?,
            Probability::new(confidence)?,
        )?)),
        WireAnswer::Noul { noul } => Ok(Answer::Noul(NoulAnswer::new(Probability::new(noul)?))),
    }
}

/// Validates every probability in a wire distribution.
fn decode_probabilities(
    probabilities: BTreeMap<String, f64>,
) -> Result<Vec<(String, Probability)>, ValidationError> {
    probabilities
        .into_iter()
        .map(|(key, value)| Ok((key, Probability::new(value)?)))
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
    #[serde(default = "default_wire_provider")]
    provider: String,
    model_revision: String,
    answers: BTreeMap<String, WireAnswer>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

/// The provider attribution used when the wire response omits `provider`.
fn default_wire_provider() -> String {
    DEFAULT_WIRE_PROVIDER.to_owned()
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

/// The private wire usage shape.
#[derive(Deserialize)]
struct WireUsage {
    input_tokens: u64,
    output_tokens: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_never_contains_the_key() {
        let secret = Secret::new("sk-super-secret-value");
        let rendered = format!("{secret:?}");
        assert_eq!(rendered, "Secret([redacted])");
        assert!(!rendered.contains("sk-super-secret-value"));
        assert_eq!(secret.expose(), "sk-super-secret-value");
    }

    #[test]
    fn config_defaults_are_documented() {
        let config = TypeSafeConfig::default();
        assert_eq!(config.endpoint, DEFAULT_TYPESAFE_ENDPOINT);
        assert_eq!(config.model, DEFAULT_TYPESAFE_MODEL);
        assert_eq!(config.api_key.expose(), "");
        assert_eq!(config.max_response_bytes, 1024 * 1024);
        assert_eq!(config.max_retries, 2);
        assert_eq!(config.connect_timeout, Duration::from_secs(10));
        assert_eq!(config.request_timeout, Duration::from_secs(30));
    }

    #[test]
    fn retryable_statuses_are_the_documented_set() {
        for status in [429, 502, 503, 504] {
            assert!(is_retryable(status), "{status} must be retryable");
        }
        for status in [200, 400, 401, 403, 404, 500, 501, 505] {
            assert!(!is_retryable(status), "{status} must not be retryable");
        }
    }

    #[test]
    fn a_bare_string_retry_after_is_parsed_and_capped() {
        assert_eq!(RETRY_AFTER_CAP, Duration::from_secs(5));
        assert_eq!(parse_retry_after("0"), Some(Duration::from_secs(0)));
        assert_eq!(parse_retry_after("2"), Some(Duration::from_secs(2)));
        assert_eq!(parse_retry_after(" 2 "), Some(Duration::from_secs(2)));
        assert_eq!(parse_retry_after("60"), Some(Duration::from_secs(5)));
        assert_eq!(parse_retry_after("later"), None);
        assert_eq!(parse_retry_after("1.5"), None);
    }
}
