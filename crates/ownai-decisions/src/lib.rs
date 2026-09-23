//! Provider-independent typed decisions for semantic code lenses.
//!
//! This crate owns validated question and answer values plus the synchronous
//! provider boundary. It deliberately knows nothing about Git, source paths,
//! language parsers, or OwnAI projections.

mod error;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
#[cfg(feature = "laya")]
pub mod laya;
mod model;
mod provider;
mod secret;
#[cfg(feature = "typesafe")]
pub mod typesafe;

pub use error::{DecisionError, ValidationError};
#[cfg(feature = "laya")]
pub use laya::{DEFAULT_LAYA_ENDPOINT, DEFAULT_LAYA_MODEL, LayaConfig, LayaProvider};
pub use model::{
    Answer, AnswerKind, ChoiceAnswer, ChoiceOption, ChoiceQuestion, DecisionRequest,
    DecisionResponse, NoulAnswer, NoulQuestion, Probability, Question, QuestionId, QuestionKind,
    ScoreAnswer, ScoreLevel, ScoreQuestion, Usage,
};
pub use provider::DecisionProvider;
pub use secret::Secret;
#[cfg(feature = "typesafe")]
pub use typesafe::{
    DEFAULT_TYPESAFE_ENDPOINT, DEFAULT_TYPESAFE_MODEL, TypeSafeConfig, TypeSafeProvider,
};
