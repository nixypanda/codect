//! A credential value that never appears in `Debug` output.

use std::fmt;

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

#[cfg(test)]
mod tests {
    use super::Secret;

    #[test]
    fn secret_debug_never_contains_the_key() {
        let secret = Secret::new("sk-super-secret-value");
        let rendered = format!("{secret:?}");
        assert_eq!(rendered, "Secret([redacted])");
        assert!(!rendered.contains("sk-super-secret-value"));
        assert_eq!(secret.expose(), "sk-super-secret-value");
    }
}
