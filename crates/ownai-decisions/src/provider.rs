//! The synchronous decision-provider boundary.

use crate::{DecisionError, DecisionRequest, DecisionResponse};

/// Evaluates typed questions against one structured state.
///
/// The boundary is synchronous because OwnAI's current engine is synchronous.
/// Implementations must return their concrete model revision in the response.
pub trait DecisionProvider: Send + Sync {
    fn evaluate(&self, request: &DecisionRequest) -> Result<DecisionResponse, DecisionError>;
}
