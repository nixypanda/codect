pub struct User<T>
where T: Identity
{
    pub id: T,
    profile: Profile,
}

pub enum Status {
    Active,
    Suspended { reason: String },
}

pub trait Repository<T>: Send + Sync {
    type Error: std::error::Error;
}

impl Iterator for Users {
    type Item = User<Id>;
}
