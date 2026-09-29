//! Nested generic arguments and tuple elements break one item per indented line
//! when their flat form does not fit, at every nesting level. Flat types are
//! unchanged.

pub struct Registry {
    pub index: HashMap<String, Result<(&'static str, Vec<(RepoPath, SnapshotEntry)>), EngineError>>,
}

impl Registry {
    pub fn snapshot_entries(
        &self,
        spec: &str,
    ) -> Result<(&'static str, String, Vec<(RepoPath, SnapshotEntry)>), EngineError> {
        todo!()
    }
}
