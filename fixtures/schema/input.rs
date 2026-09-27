//! Golden fixture for the `ownai.show.v1` document.
//!
//! It exercises the contract directly: Types mode drops the inherent `impl`
//! block and the free function, so their outline entries carry
//! `retained_in_mode: false`, while the struct, its field, and the trait with
//! its associated type remain.

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
