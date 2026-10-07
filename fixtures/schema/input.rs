//! Golden fixture for the `codect.show.v1` document.
//!
//! It exercises the contract directly: Types mode drops the inherent `impl`
//! block and the free function, so their outline entries carry
//! `retained_in_mode: false`, while the struct, its field, and the trait with
//! its associated type remain. Tests mode keeps only the declarations in the
//! test module, so most of its outline entries are also not retained.

pub struct Session {
    token: Token,
}

impl Session {
    pub fn refresh(&mut self, token: Token) -> Result<(), Error> {
        self.token = token;
        Ok(())
    }
}

pub trait Store {
    type Error;
    fn get(&self) -> Result<(), Self::Error>;
}

pub fn open() -> Session {
    Session {
        token: Token::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::Session;

    #[test]
    fn a_test_is_retained_in_tests_mode() {}

    pub fn helper_is_not() {}
}
