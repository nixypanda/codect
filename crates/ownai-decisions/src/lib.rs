//! Provider-independent typed decisions for semantic code lenses.
//!
//! This crate owns validated question and answer values plus the synchronous
//! provider boundary. It deliberately knows nothing about Git, source paths,
//! language parsers, or OwnAI projections.

mod error;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
mod model;
mod provider;
#[cfg(feature = "typesafe")]
pub mod typesafe;

pub use error::{DecisionError, ValidationError};
pub use model::{
    Answer, AnswerKind, ChoiceAnswer, ChoiceOption, ChoiceQuestion, DecisionRequest,
    DecisionResponse, NoulAnswer, NoulQuestion, Probability, Question, QuestionId, QuestionKind,
    ScoreAnswer, ScoreLevel, ScoreQuestion, Usage,
};
pub use provider::DecisionProvider;
#[cfg(feature = "typesafe")]
pub use typesafe::{
    DEFAULT_TYPESAFE_ENDPOINT, DEFAULT_TYPESAFE_MODEL, Secret, TypeSafeConfig, TypeSafeProvider,
};
