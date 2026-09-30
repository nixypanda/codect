//! Loading of the repository's named-area definitions.
//!
//! `.ownai.toml` is repository-controlled, so it is the one config file OwnAI
//! reads and the only one it must treat as hostile: bounded in size, never
//! followed through a symlink, and only ever consulted when an area is
//! selected. Core stays file-format-free, so the conversion from TOML to
//! [`AreaSet`] lives here.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use base::{Area, AreaError, AreaSet, RepoPath, RepoPathError};

// The largest config that will be read. Area definitions are tiny, so a larger
// file is far more likely a mistake or an attack than something worth parsing.
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

// The fixed config filename at the repository root; there is deliberately no
// flag to point at another file.
const FILE_NAME: &str = ".ownai.toml";

/// The repository's areas, already validated and name-sorted.
#[derive(Debug)]
pub struct Config {
    areas: AreaSet,
}

impl Config {
    /// Reads `.ownai.toml` from the repository root. The file is untrusted
    /// input, so it is size-bounded and never followed through a symlink.
    pub fn load(repo_root: &Path) -> Result<Self, ConfigError> {
        let file = repo_root.join(FILE_NAME);

        // `symlink_metadata` describes the link itself, which is what decides
        // whether reading the file is allowed at all; `metadata` would silently
        // follow the link and defeat the check.
        let metadata = std::fs::symlink_metadata(&file).map_err(|source| match source.kind() {
            io::ErrorKind::NotFound => ConfigError::NotFound { file: file.clone() },
            _ => ConfigError::Read {
                file: file.clone(),
                source,
            },
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ConfigError::Symlink { file });
        }
        if metadata.len() > MAX_CONFIG_BYTES {
            return Err(ConfigError::TooLarge { file });
        }

        let text = std::fs::read_to_string(&file).map_err(|source| ConfigError::Read {
            file: file.clone(),
            source,
        })?;
        let parsed: ConfigFile = toml::from_str(&text).map_err(|source| ConfigError::Parse {
            file: file.clone(),
            source,
        })?;

        let mut areas = Vec::with_capacity(parsed.areas.len());
        for (name, paths) in parsed.areas {
            areas.push(build_area(file.clone(), name, paths)?);
        }
        let areas = AreaSet::new(areas).map_err(|source| ConfigError::from_area(&file, source))?;
        Ok(Self { areas })
    }

    pub fn areas(&self) -> &AreaSet {
        &self.areas
    }
}

// The on-disk shape. Unknown keys are rejected so a typo cannot silently drop
// the area it was meant to define.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    #[serde(default)]
    areas: BTreeMap<String, Vec<String>>,
}

fn build_area(file: PathBuf, name: String, paths: Vec<String>) -> Result<Area, ConfigError> {
    // Whitespace-only names are config-specific: `Area::new` rejects only a
    // truly empty name, so the trim check stays here.
    if name.trim().is_empty() {
        return Err(ConfigError::EmptyName { file });
    }

    let mut resolved = Vec::with_capacity(paths.len());
    for value in paths {
        resolved.push(normalize_path(&file, &name, &value)?);
    }
    Area::new(name, resolved).map_err(|source| ConfigError::from_area(&file, source))
}

// Normalizes one area path leniently: the only hard failures are being
// absolute or escaping the repository, while stray empty and `.` components
// are behavior the user almost certainly did not intend to be fatal.
fn normalize_path(file: &Path, area: &str, value: &str) -> Result<RepoPath, ConfigError> {
    let invalid = |source| ConfigError::InvalidPath {
        file: file.to_path_buf(),
        area: area.to_owned(),
        value: value.to_owned(),
        source,
    };

    if value.starts_with('/') {
        return Err(invalid(RepoPathError::Absolute));
    }

    // `..` is left in place so `RepoPath::new` rejects it with its own error;
    // dropping it here would silently retarget the path.
    let normalized = value
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .collect::<Vec<_>>()
        .join("/");
    if normalized.is_empty() {
        return Err(invalid(RepoPathError::Empty));
    }

    RepoPath::new(normalized).map_err(invalid)
}

/// A config file that cannot supply areas.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("no `{file}` was found at the repository root")]
    NotFound { file: PathBuf },

    #[error("`{file}` is a symlink, which is not followed")]
    Symlink { file: PathBuf },

    #[error("`{file}` is larger than the 1 MiB limit")]
    TooLarge { file: PathBuf },

    #[error("`{file}` could not be read")]
    Read {
        file: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("`{file}` is not valid TOML")]
    Parse {
        file: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("`{file}` defines an area with an empty name")]
    EmptyName { file: PathBuf },

    #[error("`{file}` area `{area}` defines no paths")]
    EmptyArea { file: PathBuf, area: String },

    #[error("`{file}` area `{area}` has an unusable path `{value}`")]
    InvalidPath {
        file: PathBuf,
        area: String,
        value: String,
        #[source]
        source: RepoPathError,
    },

    #[error("`{file}` defines the area `{name}` more than once")]
    DuplicateArea { file: PathBuf, name: String },
}

impl ConfigError {
    fn from_area(file: &Path, error: AreaError) -> Self {
        match error {
            AreaError::EmptyName => Self::EmptyName {
                file: file.to_path_buf(),
            },
            AreaError::EmptyPaths { name } => Self::EmptyArea {
                file: file.to_path_buf(),
                area: name,
            },
            AreaError::DuplicateName { name } => Self::DuplicateArea {
                file: file.to_path_buf(),
                name,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> tempfile::TempDir {
        tempfile::TempDir::new().expect("temporary directory")
    }

    fn write_config(root: &Path, contents: &str) {
        std::fs::write(root.join(FILE_NAME), contents).expect("write config");
    }

    fn area_paths(config: &Config, name: &str) -> Vec<String> {
        config
            .areas()
            .get(name)
            .expect("area is defined")
            .paths()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn a_valid_file_loads_areas() {
        let dir = temp();
        write_config(
            dir.path(),
            "[areas]\nfrontend = [\"apps/web\", \"packages/ui\"]\nbackend = [\"services/api\"]\n",
        );

        let config = Config::load(dir.path()).expect("load config");
        assert_eq!(
            config.areas().names().collect::<Vec<_>>(),
            vec!["backend", "frontend"]
        );
        assert_eq!(
            area_paths(&config, "frontend"),
            vec!["apps/web", "packages/ui"]
        );
    }

    #[test]
    fn a_missing_file_is_not_found() {
        let dir = temp();
        let error = Config::load(dir.path()).unwrap_err();
        assert!(matches!(error, ConfigError::NotFound { .. }));
    }

    #[test]
    fn malformed_toml_is_a_parse_error() {
        let dir = temp();
        write_config(dir.path(), "this is not toml");
        let error = Config::load(dir.path()).unwrap_err();
        assert!(matches!(error, ConfigError::Parse { .. }));
    }

    #[test]
    fn an_unknown_top_level_key_is_rejected() {
        let dir = temp();
        write_config(dir.path(), "surprise = true\n");
        let error = Config::load(dir.path()).unwrap_err();
        assert!(matches!(error, ConfigError::Parse { .. }));
    }

    #[test]
    fn duplicate_area_keys_are_rejected() {
        let dir = temp();
        write_config(
            dir.path(),
            "[areas]\nfrontend = [\"apps/web\"]\nfrontend = [\"packages/ui\"]\n",
        );
        let error = Config::load(dir.path()).unwrap_err();
        assert!(matches!(error, ConfigError::Parse { .. }));
    }

    #[test]
    fn an_empty_area_name_is_rejected() {
        let dir = temp();
        write_config(dir.path(), "[areas]\n\"  \" = [\"src\"]\n");
        let error = Config::load(dir.path()).unwrap_err();
        assert!(matches!(error, ConfigError::EmptyName { .. }));
    }

    #[test]
    fn an_empty_area_list_is_rejected() {
        let dir = temp();
        write_config(dir.path(), "[areas]\nfrontend = []\n");
        let error = Config::load(dir.path()).unwrap_err();
        assert!(matches!(error, ConfigError::EmptyArea { .. }));
    }

    #[test]
    fn an_absolute_area_path_is_rejected() {
        let dir = temp();
        write_config(dir.path(), "[areas]\nfrontend = [\"/etc/passwd\"]\n");
        let error = Config::load(dir.path()).unwrap_err();
        assert!(matches!(
            error,
            ConfigError::InvalidPath {
                source: RepoPathError::Absolute,
                ..
            }
        ));
    }

    #[test]
    fn a_parent_traversal_area_path_is_rejected() {
        let dir = temp();
        write_config(dir.path(), "[areas]\nfrontend = [\"src/../etc\"]\n");
        let error = Config::load(dir.path()).unwrap_err();
        assert!(matches!(
            error,
            ConfigError::InvalidPath {
                source: RepoPathError::ParentTraversal,
                ..
            }
        ));
    }

    #[test]
    fn trailing_slashes_and_dot_components_are_normalized() {
        let dir = temp();
        write_config(dir.path(), "[areas]\nfrontend = [\"src/\", \"./src\"]\n");
        let config = Config::load(dir.path()).expect("load config");
        // Both entries normalize to `src`, which `Area::new` then deduplicates.
        assert_eq!(area_paths(&config, "frontend"), vec!["src"]);
    }

    #[test]
    fn a_file_without_an_areas_table_yields_no_areas() {
        let dir = temp();
        write_config(dir.path(), "# area-free config\n");
        let config = Config::load(dir.path()).expect("load config");
        assert_eq!(config.areas().names().count(), 0);
    }

    #[test]
    fn an_oversized_config_is_rejected() {
        let dir = temp();
        let oversized = vec![b' '; (MAX_CONFIG_BYTES + 1) as usize];
        std::fs::write(dir.path().join(FILE_NAME), oversized).expect("write oversized config");
        let error = Config::load(dir.path()).unwrap_err();
        assert!(matches!(error, ConfigError::TooLarge { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_config_is_rejected() {
        let dir = temp();
        let real = dir.path().join("real.toml");
        std::fs::write(&real, "[areas]\nfrontend = [\"apps/web\"]\n").expect("write real config");
        std::os::unix::fs::symlink(&real, dir.path().join(FILE_NAME)).expect("create symlink");

        let error = Config::load(dir.path()).unwrap_err();
        assert!(matches!(error, ConfigError::Symlink { .. }));
    }
}
