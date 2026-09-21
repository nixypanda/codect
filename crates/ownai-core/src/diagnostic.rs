//! Typed library errors and user-facing diagnostics.
//!
//! Library crates use `thiserror` to define typed errors. Diagnostics carry
//! repository location, revision string, source path, language, source range,
//! and the underlying error chain where applicable.
