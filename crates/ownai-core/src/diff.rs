//! Line-oriented patience diff over canonical projected text.
//!
//! Diffs are textual: canonical projection is the semantic step, and the diff
//! engine compares projection text rather than syntax trees. Output is plain and
//! deterministic; ANSI styling is applied only in the CLI.
