pub struct Wrapper<'a, T: Clone + Send + Sync + std::fmt::Debug, const N: usize, U = u8>
where
    T: 'a,
    U: Default,
{
    pub item: &'a T,
    pub size: [U; N],
}

pub async fn load<T>(
    id: Id,
    source: &T,
    cache: &Cache,
    config: &Config,
    timeout: Duration,
) -> Result<User, T::Error>
where
    T: Repository<User>,
{
    todo!()
}

fn short(a: i32, b: i32) -> i32 {
    a + b
}
