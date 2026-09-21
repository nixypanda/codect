//! Revision resolution and commit peeling.
//!
//! Resolves a user-provided revision string to exactly one commit, peeling
//! annotated tags and other commit-ish objects. Ranges are rejected.
