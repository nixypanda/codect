//! Commit tree traversal and blob reads.
//!
//! Recursively visits a commit tree and retains regular and executable blob
//! entries ending in `.elm` or `.rs`, sorted by raw repository path bytes.
