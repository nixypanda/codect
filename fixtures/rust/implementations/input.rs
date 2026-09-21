//! Inherent and trait implementations.

impl User {
    pub const fn id(&self) -> Id {
        self.id
    }

    fn private(&mut self) {}
}

impl<T: Clone> Iterator for Users<T>
where
    T: Send,
{
    type Item = User<T>;

    fn next(&mut self) -> Option<Self::Item> {
        None
    }
}

impl Display for User {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.name)
    }
}

impl Clone for User {}

unsafe impl Send for User {}
