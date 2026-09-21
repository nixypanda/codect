pub async fn load<T>(id: Id, source: &T) -> Result<User, T::Error>
where T: Repository<User>
{
    todo!()
}

impl User {
    pub const fn id(&self) -> Id {
        self.id
    }
}

impl Display for User {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.id)
    }
}
