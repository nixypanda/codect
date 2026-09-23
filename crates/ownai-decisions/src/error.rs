//! Typed failures at the decision-provider boundary.

use std::time::Duration;

/// Invalid caller input or provider output.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ValidationError {
    #[error("{field} must not be empty")]
    Empty { field: &'static str },

    #[error("provider response could not be parsed: {message}")]
    MalformedResponse { message: String },

    #[error("question id `{value}` contains unsupported characters")]
    InvalidQuestionId { value: String },

    #[error("question id `{id}` is duplicated")]
    DuplicateQuestion { id: String },

    #[error("answer id `{id}` is duplicated")]
    DuplicateAnswer { id: String },

    #[error("option `{key}` is duplicated in question `{question}`")]
    DuplicateOption { question: String, key: String },

    #[error("question `{question}` requires at least {minimum} options")]
    TooFewOptions { question: String, minimum: usize },

    #[error("probability must be finite and between 0 and 1, got {value}")]
    InvalidProbability { value: String },

    #[error("probability key `{key}` is duplicated")]
    DuplicateProbability { key: String },

    #[error("probability distribution must not be empty")]
    EmptyDistribution,

    #[error("probabilities must sum to 1, got {sum}")]
    ProbabilitySum { sum: String },

    #[error("selected choice `{selected}` is not in the probability distribution")]
    UnknownSelectedChoice { selected: String },

    #[error("selected choice `{selected}` does not have the highest probability")]
    SelectedChoiceNotMaximum { selected: String },

    #[error("score must be finite, got {value}")]
    InvalidScore { value: String },

    #[error("response is missing answer `{id}`")]
    MissingAnswer { id: String },

    #[error("response contains unexpected answer `{id}`")]
    UnexpectedAnswer { id: String },

    #[error("answer `{id}` has type {actual}, expected {expected}")]
    WrongAnswerType {
        id: String,
        expected: &'static str,
        actual: &'static str,
    },

    #[error("answer `{id}` uses options that differ from its question")]
    OptionMismatch { id: String },

    #[error("score for answer `{id}` is outside the range 0..={maximum}, got {value}")]
    ScoreOutOfRange {
        id: String,
        maximum: usize,
        value: String,
    },
}

/// A request could not be evaluated by a provider.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DecisionError {
    #[error("decision provider returned an invalid response")]
    InvalidResponse {
        #[source]
        source: ValidationError,
    },

    #[error("decision provider is not authenticated")]
    Authentication,

    #[error("decision provider rate limit was exceeded")]
    RateLimited { retry_after: Option<Duration> },

    #[error("decision provider request timed out")]
    Timeout,

    #[error("decision provider transport failed: {message}")]
    Transport { message: String },

    #[error("decision provider returned HTTP {status}: {message}")]
    Http { status: u16, message: String },

    #[error("decision provider response exceeded {limit_bytes} bytes")]
    ResponseTooLarge { limit_bytes: usize },

    #[error("decision provider failed: {message}")]
    Provider { message: String },
}

impl DecisionError {
    pub fn invalid_response(source: ValidationError) -> Self {
        Self::InvalidResponse { source }
    }
}
