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

use ownai_core::{Area, AreaError, AreaSet, RepoPath, RepoPathError};

use crate::review::{ReviewConcerns, ReviewConcernsError};

/// The largest config that will be read. Area definitions are tiny, so a larger
/// file is far more likely a mistake or an attack than something worth parsing.
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// The fixed config filename at the repository root; there is deliberately no
/// flag to point at another file.
const FILE_NAME: &str = ".ownai.toml";

/// The default choice confidence threshold when the repository sets none.
const DEFAULT_REVIEW_CHOICE_CONFIDENCE_THRESHOLD: f64 = 0.55;

/// The repository's areas, already validated and name-sorted.
#[derive(Debug)]
pub struct Config {
    areas: AreaSet,
    review: ReviewConfig,
}

/// The repository's optional review-lens settings.
///
/// `concerns` is `None` when the repository did not configure a taxonomy, in
/// which case the review lens's built-in taxonomy applies. A configured
/// taxonomy is read from a [`BTreeMap`], so it is name-sorted; the built-in
/// taxonomy keeps its fixed source order.
#[derive(Clone, Debug)]
pub struct ReviewConfig {
    /// The confidence at or above which a concern is rendered as confident.
    pub choice_confidence_threshold: f64,
    /// The configured concern taxonomy, when the repository defines one.
    pub concerns: Option<ReviewConcerns>,
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            choice_confidence_threshold: DEFAULT_REVIEW_CHOICE_CONFIDENCE_THRESHOLD,
            concerns: None,
        }
    }
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
        let review = build_review(&file, parsed.lenses.review)?;
        Ok(Self { areas, review })
    }

    /// Reads only the review settings, treating a missing file as defaults.
    ///
    /// Every other failure, including a malformed or oversized file, is
    /// propagated so a broken config is never silently ignored.
    pub fn load_for_review(repo_root: &Path) -> Result<ReviewConfig, ConfigError> {
        match Self::load(repo_root) {
            Ok(config) => Ok(config.review),
            Err(ConfigError::NotFound { .. }) => Ok(ReviewConfig::default()),
            Err(error) => Err(error),
        }
    }

    pub fn areas(&self) -> &AreaSet {
        &self.areas
    }

    pub fn review(&self) -> &ReviewConfig {
        &self.review
    }
}

/// The on-disk shape. Unknown keys are rejected so a typo cannot silently drop
/// the area it was meant to define.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    #[serde(default)]
    areas: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    lenses: Lenses,
}

/// The `[lenses]` table. Only the review lens is defined in v1.
#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Lenses {
    #[serde(default)]
    review: ReviewSection,
}

/// The `[lenses.review]` table.
#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewSection {
    choice_confidence_threshold: Option<f64>,
    #[serde(default)]
    concerns: Option<BTreeMap<String, String>>,
}

/// Validates the review section into owned settings.
///
/// A configured taxonomy is built from a name-sorted map, so its order is
/// deterministic but sorted rather than source order.
fn build_review(file: &Path, section: ReviewSection) -> Result<ReviewConfig, ConfigError> {
    let threshold = section
        .choice_confidence_threshold
        .unwrap_or(DEFAULT_REVIEW_CHOICE_CONFIDENCE_THRESHOLD);
    if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
        return Err(ConfigError::InvalidReviewThreshold {
            file: file.to_path_buf(),
            value: threshold.to_string(),
        });
    }

    let concerns = match section.concerns {
        Some(concerns) => Some(ReviewConcerns::new(concerns).map_err(|source| {
            ConfigError::InvalidReviewConcerns {
                file: file.to_path_buf(),
                source,
            }
        })?),
        None => None,
    };

    Ok(ReviewConfig {
        choice_confidence_threshold: threshold,
        concerns,
    })
}

fn build_area(file: PathBuf, name: String, paths: Vec<String>) -> Result<Area, ConfigError> {
    if name.trim().is_empty() {
        return Err(ConfigError::EmptyName { file });
    }
    if paths.is_empty() {
        return Err(ConfigError::EmptyArea { file, area: name });
    }

    let mut resolved = Vec::with_capacity(paths.len());
    for value in paths {
        resolved.push(normalize_path(&file, &name, &value)?);
    }
    Ok(Area {
        name,
        paths: resolved,
    })
}

/// Normalizes one area path leniently: the only hard failures are being
/// absolute or escaping the repository, while stray empty and `.` components
/// are behavior the user almost certainly did not intend to be fatal.
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

    #[error(
        "`{file}` sets an invalid review choice confidence threshold `{value}`; it must be finite and within 0.0..=1.0"
    )]
    InvalidReviewThreshold { file: PathBuf, value: String },

    #[error("`{file}` defines invalid review concerns")]
    InvalidReviewConcerns {
        file: PathBuf,
        #[source]
        source: ReviewConcernsError,
    },
}

impl ConfigError {
    fn from_area(file: &Path, error: AreaError) -> Self {
        match error {
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
            .paths
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
        assert_eq!(area_paths(&config, "frontend"), vec!["src", "src"]);
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

    #[test]
    fn a_valid_review_section_loads_the_threshold_and_sorted_concerns() {
        let dir = temp();
        write_config(
            dir.path(),
            concat!(
                "[lenses.review]\n",
                "choice_confidence_threshold = 0.75\n",
                "\n",
                "[lenses.review.concerns]\n",
                "tui = \"Terminal interaction\"\n",
                "engine = \"Engine behavior\"\n",
                "other = \"No listed concern is a good fit\"\n",
            ),
        );

        let config = Config::load(dir.path()).expect("load config");
        let review = config.review();
        assert_eq!(review.choice_confidence_threshold, 0.75);
        let concerns = review.concerns.as_ref().expect("configured concerns");
        let keys: Vec<&str> = concerns
            .options()
            .iter()
            .map(|(key, _)| key.as_str())
            .collect();
        assert_eq!(
            keys,
            vec!["engine", "other", "tui"],
            "configured order is sorted"
        );
    }

    #[test]
    fn an_areas_only_config_yields_the_default_review_config() {
        let dir = temp();
        write_config(dir.path(), "[areas]\nfrontend = [\"apps/web\"]\n");

        let config = Config::load(dir.path()).expect("load config");
        let review = config.review();
        assert_eq!(review.choice_confidence_threshold, 0.55);
        assert!(review.concerns.is_none());
    }

    #[test]
    fn unknown_keys_under_lenses_are_rejected() {
        let dir = temp();
        for contents in [
            "[lenses]\nsurprise = true\n",
            "[lenses.review]\nsurprise = true\n",
        ] {
            write_config(dir.path(), contents);
            let error = Config::load(dir.path()).unwrap_err();
            assert!(
                matches!(error, ConfigError::Parse { .. }),
                "`{contents}` must be rejected, got {error:?}"
            );
        }
    }

    #[test]
    fn an_out_of_range_or_non_finite_review_threshold_is_rejected() {
        let dir = temp();
        for value in ["1.5", "-0.1", "nan", "inf"] {
            write_config(
                dir.path(),
                &format!("[lenses.review]\nchoice_confidence_threshold = {value}\n"),
            );
            let parsed: f64 = value.parse().expect("the fixture is a float");
            let error = Config::load(dir.path()).unwrap_err();
            match error {
                ConfigError::InvalidReviewThreshold {
                    value: reported, ..
                } => {
                    assert_eq!(reported, parsed.to_string());
                }
                other => panic!("expected InvalidReviewThreshold for {value}, got {other:?}"),
            }
        }
    }

    #[test]
    fn review_concerns_without_other_are_rejected() {
        let dir = temp();
        write_config(
            dir.path(),
            "[lenses.review.concerns]\nengine = \"Engine behavior\"\n",
        );

        let error = Config::load(dir.path()).unwrap_err();
        match error {
            ConfigError::InvalidReviewConcerns { source, .. } => {
                assert_eq!(source, ReviewConcernsError::MissingOther);
            }
            other => panic!("expected InvalidReviewConcerns, got {other:?}"),
        }
    }

    #[test]
    fn too_many_review_concerns_are_rejected() {
        let dir = temp();
        let mut contents = String::from("[lenses.review.concerns]\n");
        for index in 0..33 {
            contents.push_str(&format!("concern-{index} = \"A concern\"\n"));
        }
        contents.push_str("other = \"No listed concern\"\n");
        write_config(dir.path(), &contents);

        let error = Config::load(dir.path()).unwrap_err();
        match error {
            ConfigError::InvalidReviewConcerns { source, .. } => {
                assert!(matches!(
                    source,
                    ReviewConcernsError::TooManyConcerns {
                        count: 34,
                        maximum: 32,
                    }
                ));
            }
            other => panic!("expected InvalidReviewConcerns, got {other:?}"),
        }
    }

    #[test]
    fn an_oversized_review_concern_description_is_rejected() {
        let dir = temp();
        let description = "x".repeat(257);
        write_config(
            dir.path(),
            &format!(
                "[lenses.review.concerns]\nengine = \"{description}\"\nother = \"No listed concern\"\n"
            ),
        );

        let error = Config::load(dir.path()).unwrap_err();
        match error {
            ConfigError::InvalidReviewConcerns { source, .. } => {
                assert!(matches!(
                    source,
                    ReviewConcernsError::ConcernDescriptionTooLong {
                        bytes: 257,
                        maximum: 256,
                        ..
                    }
                ));
            }
            other => panic!("expected InvalidReviewConcerns, got {other:?}"),
        }
    }

    #[test]
    fn load_for_review_defaults_a_missing_file_but_rejects_a_malformed_one() {
        let missing = temp();
        let review = Config::load_for_review(missing.path()).expect("defaults");
        assert_eq!(review.choice_confidence_threshold, 0.55);
        assert!(review.concerns.is_none());

        let malformed = temp();
        write_config(malformed.path(), "this is not toml");
        let error = Config::load_for_review(malformed.path()).unwrap_err();
        assert!(matches!(error, ConfigError::Parse { .. }));
    }
}
