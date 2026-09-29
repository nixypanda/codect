pub enum Error {
    Invalid {
        path: RepositoryPath,
        language: Language,
        source: std::str::Utf8Error,
    },
}
