//! The provider-independent semantic review lens.
//!
//! A lens turns one deterministic [`ItemDiff`] into a versioned JSON decision
//! state, asks the fixed v1 review questions through a [`DecisionProvider`], and
//! decodes the validated answers into an OwnAI-owned [`ReviewJudgment`]. It owns
//! the question taxonomy, the local serialized-state byte limit, and nothing
//! else: no rendering, I/O, configuration, persistence, or concurrency.
//!
//! The lens never invents deterministic facts. Language, path, change kind, item
//! kind, and canonical text come from the projection model; the provider only
//! supplies bounded judgments.

use std::collections::BTreeMap;

use ownai_core::{ItemKind, Language, ProjectionMode, RepoPath};
use ownai_decisions::{
    Answer, ChoiceOption, ChoiceQuestion, DecisionProvider, DecisionRequest, DecisionResponse,
    NoulQuestion, Probability, Question, QuestionId, ScoreLevel, ScoreQuestion, Usage,
    ValidationError,
};
use serde_json::{Value, json};

use crate::engine::FileDiff;
use crate::item_diff::{ItemChangeKind, ItemDiff};

/// The versioned schema identifier for one review unit's serialized state.
pub const REVIEW_STATE_SCHEMA: &str = "ownai.review-unit.v1";

/// The stable v1 question identifiers, in the order they are sent.
const CONCERN_ID: &str = "concern";
const RISK_ID: &str = "risk";
const LIKELY_BREAKING_ID: &str = "likely_breaking";
const NEEDS_TESTS_ID: &str = "needs_tests";
const NEEDS_DOCS_ID: &str = "needs_docs";
const NEEDS_MIGRATION_ID: &str = "needs_migration";
const SECURITY_SENSITIVE_ID: &str = "security_sensitive";

/// The built-in concern choices, in stable order, with distinguishing text.
///
/// `other` is required by the review lens; the shared choice constructor
/// deliberately does not require it.
const CONCERN_OPTIONS: [(&str, &str); 10] = [
    (
        "api-contract",
        "Public function, type, trait, or module interface",
    ),
    ("data-model", "Persisted or in-memory data shape and schema"),
    (
        "authentication-authorization",
        "Identity, permissions, or access control",
    ),
    ("storage", "Persistence, database, or filesystem behavior"),
    (
        "networking-integration",
        "Remote calls, protocols, or third-party integration",
    ),
    (
        "configuration-operations",
        "Configuration, deployment, or operational behavior",
    ),
    ("observability", "Logging, metrics, tracing, or diagnostics"),
    (
        "user-interface",
        "Rendered output or interactive user experience",
    ),
    ("testing-tooling", "Test code, fixtures, or build tooling"),
    ("other", "None of the named concerns is a good fit"),
];

/// The ordered review-risk levels, matching the provider's `0..=4` score index.
const RISK_LEVELS: [(&str, &str); 5] = [
    ("routine", "Local, mechanical, or very low review risk"),
    ("low", "Limited behavior or contract risk"),
    (
        "moderate",
        "Meaningful behavior, caller, or data-shape risk",
    ),
    (
        "high",
        "Cross-boundary, compatibility, deployment, or security risk",
    ),
    (
        "critical",
        "Broad or irreversible risk requiring specialist review",
    ),
];

/// A decoded choice answer: the selected value, its full distribution, and the
/// provider's confidence.
#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceJudgment {
    pub selected: String,
    pub probabilities: BTreeMap<String, Probability>,
    pub confidence: Probability,
}

/// A decoded score answer.
///
/// `raw_index` is the provider's `0..=4` value; `display_value` is OwnAI's
/// one-based `1..=5` rendering, always `raw_index + 1.0`.
#[derive(Clone, Debug, PartialEq)]
pub struct ScoreJudgment {
    pub raw_index: f64,
    pub display_value: f64,
    pub probabilities: BTreeMap<String, Probability>,
    pub confidence: Probability,
}

/// The complete v1 review judgment for one changed declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewJudgment {
    pub concern: ChoiceJudgment,
    pub risk: ScoreJudgment,
    pub likely_breaking: Probability,
    pub needs_tests: Probability,
    pub needs_docs: Probability,
    pub needs_migration: Probability,
    pub security_sensitive: Probability,
}

/// One reviewed item diff with its provider attribution.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewedItemDiff {
    pub item: ItemDiff,
    pub judgment: ReviewJudgment,
    pub provider: String,
    pub model_revision: String,
    pub usage: Option<Usage>,
}

/// The result of evaluating one item, preserving failures next to successes.
#[derive(Clone, Debug)]
pub enum ReviewOutcome {
    Reviewed(ReviewedItemDiff),
    Failed { item: ItemDiff, error: ReviewError },
}

/// A typed failure while building or evaluating a review request.
#[derive(Clone, Debug, thiserror::Error)]
pub enum ReviewError {
    /// The configured serialized-state byte limit was zero.
    #[error("the review state byte limit must be greater than zero, got {limit_bytes}")]
    InvalidStateLimit { limit_bytes: usize },

    /// The fixed v1 questions could not be constructed. This indicates a bug in
    /// the lens's own constants, never caller input.
    #[error("the fixed review questions could not be constructed")]
    InvalidQuestions {
        #[source]
        source: ValidationError,
    },

    /// The serialized state exceeded the configured limit; no provider call was
    /// made and the state was not truncated.
    #[error(
        "review state for `{path}::{stable_key}` is {actual_bytes} bytes, exceeding the {limit_bytes}-byte limit"
    )]
    StateTooLarge {
        path: RepoPath,
        stable_key: String,
        actual_bytes: usize,
        limit_bytes: usize,
    },

    /// The provider's `evaluate` returned an error.
    #[error("the decision provider failed for `{path}::{stable_key}`")]
    Provider {
        path: RepoPath,
        stable_key: String,
        #[source]
        source: ownai_decisions::DecisionError,
    },

    /// The provider returned `Ok`, but the lens's mandatory post-evaluation
    /// validation rejected the response.
    #[error("the decision provider returned an invalid response for `{path}::{stable_key}`")]
    InvalidResponse {
        path: RepoPath,
        stable_key: String,
        #[source]
        source: ValidationError,
    },
}

/// The fixed v1 question set plus the local serialized-state byte limit.
pub struct ReviewLens {
    questions: Vec<Question>,
    max_state_bytes: usize,
}

impl ReviewLens {
    /// Builds the fixed v1 question set.
    ///
    /// A zero `max_state_bytes` is rejected before a provider can ever be
    /// called.
    pub fn new(max_state_bytes: usize) -> Result<Self, ReviewError> {
        if max_state_bytes == 0 {
            return Err(ReviewError::InvalidStateLimit { limit_bytes: 0 });
        }
        let questions =
            build_questions().map_err(|source| ReviewError::InvalidQuestions { source })?;
        Ok(Self {
            questions,
            max_state_bytes,
        })
    }

    /// Converts one item diff into state, checks its size, evaluates exactly one
    /// request, validates the response, and decodes its answers.
    pub fn review_item(
        &self,
        provider: &dyn DecisionProvider,
        item: ItemDiff,
    ) -> Result<ReviewedItemDiff, ReviewError> {
        let state = review_state(&item);
        let encoded = serde_json::to_vec(&state).expect("review state is always serializable");
        if encoded.len() > self.max_state_bytes {
            return Err(ReviewError::StateTooLarge {
                path: item.path.clone(),
                stable_key: item.stable_key.clone(),
                actual_bytes: encoded.len(),
                limit_bytes: self.max_state_bytes,
            });
        }

        let request = DecisionRequest::new(state, self.questions.clone())
            .map_err(|source| ReviewError::InvalidQuestions { source })?;
        let response = provider
            .evaluate(&request)
            .map_err(|source| ReviewError::Provider {
                path: item.path.clone(),
                stable_key: item.stable_key.clone(),
                source,
            })?;
        // The trait cannot require every implementation to validate its own
        // response, so the lens always re-checks coverage, kinds, options, and
        // score range against the request it built.
        response
            .validate_for(&request)
            .map_err(|source| ReviewError::InvalidResponse {
                path: item.path.clone(),
                stable_key: item.stable_key.clone(),
                source,
            })?;

        let judgment = decode_judgment(&response);
        Ok(ReviewedItemDiff {
            item,
            judgment,
            provider: response.provider().to_owned(),
            model_revision: response.model_revision().to_owned(),
            usage: response.usage(),
        })
    }

    /// Reviews every changed top-level declaration of one file diff.
    ///
    /// Units come from [`FileDiff::item_diffs`], so matching and ordering are
    /// not repeated here. Evaluation is sequential and one outcome is returned
    /// per unit in that same order; a failure never discards earlier successes.
    pub fn review_file(
        &self,
        provider: &dyn DecisionProvider,
        diff: &FileDiff,
    ) -> Vec<ReviewOutcome> {
        diff.item_diffs()
            .into_iter()
            .map(|item| match self.review_item(provider, item.clone()) {
                Ok(reviewed) => ReviewOutcome::Reviewed(reviewed),
                Err(error) => ReviewOutcome::Failed { item, error },
            })
            .collect()
    }
}

fn build_questions() -> Result<Vec<Question>, ValidationError> {
    // The review lens requires an `other` fallback even though the shared
    // choice constructor does not. The constants are static, so this is an
    // invariant assertion rather than caller validation.
    assert!(
        CONCERN_OPTIONS.iter().any(|(key, _)| *key == "other"),
        "the review lens concern question must include an `other` option"
    );

    let options = CONCERN_OPTIONS
        .iter()
        .map(|(key, description)| ChoiceOption::new(*key, Some((*description).to_owned())))
        .collect::<Result<Vec<_>, _>>()?;
    let concern = ChoiceQuestion::new(
        QuestionId::new(CONCERN_ID)?,
        "Select the engineering concern most directly affected by this change.",
        options,
    )?;

    let levels = RISK_LEVELS
        .iter()
        .map(|(label, description)| ScoreLevel::new(*label, Some((*description).to_owned())))
        .collect::<Result<Vec<_>, _>>()?;
    let risk = ScoreQuestion::new(
        QuestionId::new(RISK_ID)?,
        "Rate the review risk of this change.",
        levels,
    )?;

    let likely_breaking = NoulQuestion::new(
        QuestionId::new(LIKELY_BREAKING_ID)?,
        "Existing callers or persisted data may require changes.",
    )?;
    let needs_tests = NoulQuestion::new(
        QuestionId::new(NEEDS_TESTS_ID)?,
        "The change warrants new or updated tests.",
    )?;
    let needs_docs = NoulQuestion::new(
        QuestionId::new(NEEDS_DOCS_ID)?,
        "User-facing or developer-facing documentation may need updating.",
    )?;
    let needs_migration = NoulQuestion::new(
        QuestionId::new(NEEDS_MIGRATION_ID)?,
        "Deployment, data, configuration, or caller migration may be required.",
    )?;
    let security_sensitive = NoulQuestion::new(
        QuestionId::new(SECURITY_SENSITIVE_ID)?,
        "The change affects a trust, authorization, secret, or sensitive-data boundary.",
    )?;

    Ok(vec![
        Question::Choice(concern),
        Question::Score(risk),
        Question::Noul(likely_breaking),
        Question::Noul(needs_tests),
        Question::Noul(needs_docs),
        Question::Noul(needs_migration),
        Question::Noul(security_sensitive),
    ])
}

/// Builds the versioned review state for one item.
///
/// The absent side of an addition or removal is JSON `null`, never an empty
/// string. Object keys are not ordered here; canonical serialization sorts them.
fn review_state(item: &ItemDiff) -> Value {
    json!({
        "schema": REVIEW_STATE_SCHEMA,
        "path": {
            "display": item.path.to_string(),
            "bytes_hex": hex_lower(item.path.as_bytes()),
        },
        "language": language_key(item.language),
        "projection_mode": projection_mode_key(item.mode),
        "change": change_key(item.change),
        "item": {
            "kind": item_kind_key(item.item_kind),
            "name": item.name,
            "stable_key": item.stable_key,
        },
        "before": item.old_text,
        "after": item.new_text,
    })
}

/// Lowercase hexadecimal, written locally so the state schema adds no
/// dependency.
fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Every core variant maps explicitly to a stable wire key. No wildcard arm, so
/// adding a variant is a compile error until its schema string is chosen.
fn projection_mode_key(mode: ProjectionMode) -> &'static str {
    match mode {
        ProjectionMode::Types => "types",
        ProjectionMode::Signatures => "signatures",
    }
}

fn language_key(language: Language) -> &'static str {
    match language {
        Language::Elm => "elm",
        Language::Haskell => "haskell",
        Language::Python => "python",
        Language::Rust => "rust",
    }
}

fn item_kind_key(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Module => "module",
        ItemKind::Type => "type",
        ItemKind::TypeAlias => "type-alias",
        ItemKind::Constructor => "constructor",
        ItemKind::Field => "field",
        ItemKind::Variant => "variant",
        ItemKind::Trait => "trait",
        ItemKind::TraitImplementation => "trait-implementation",
        ItemKind::AssociatedType => "associated-type",
        ItemKind::TypeFamily => "type-family",
        ItemKind::PatternSynonym => "pattern-synonym",
        ItemKind::Function => "function",
        ItemKind::Method => "method",
        ItemKind::Value => "value",
        ItemKind::Constant => "constant",
        ItemKind::Static => "static",
        ItemKind::Port => "port",
        ItemKind::Operator => "operator",
        ItemKind::ForeignBlock => "foreign-block",
    }
}

fn change_key(change: ItemChangeKind) -> &'static str {
    match change {
        ItemChangeKind::Added => "added",
        ItemChangeKind::Removed => "removed",
        ItemChangeKind::Modified => "modified",
    }
}

fn decode_judgment(response: &DecisionResponse) -> ReviewJudgment {
    ReviewJudgment {
        concern: decode_choice(response, CONCERN_ID),
        risk: decode_score(response, RISK_ID),
        likely_breaking: decode_noul(response, LIKELY_BREAKING_ID),
        needs_tests: decode_noul(response, NEEDS_TESTS_ID),
        needs_docs: decode_noul(response, NEEDS_DOCS_ID),
        needs_migration: decode_noul(response, NEEDS_MIGRATION_ID),
        security_sensitive: decode_noul(response, SECURITY_SENSITIVE_ID),
    }
}

fn decode_choice(response: &DecisionResponse, id: &str) -> ChoiceJudgment {
    match answer_for(response, id) {
        Answer::Choice(answer) => ChoiceJudgment {
            selected: answer.selected().to_owned(),
            probabilities: answer.probabilities().clone(),
            confidence: answer.confidence(),
        },
        _ => unreachable!("validation guarantees `{id}` is a choice answer"),
    }
}

fn decode_score(response: &DecisionResponse, id: &str) -> ScoreJudgment {
    match answer_for(response, id) {
        Answer::Score(answer) => {
            let raw_index = answer.value();
            ScoreJudgment {
                raw_index,
                display_value: raw_index + 1.0,
                probabilities: answer.probabilities().clone(),
                confidence: answer.confidence(),
            }
        }
        _ => unreachable!("validation guarantees `{id}` is a score answer"),
    }
}

fn decode_noul(response: &DecisionResponse, id: &str) -> Probability {
    match answer_for(response, id) {
        Answer::Noul(answer) => answer.probability(),
        _ => unreachable!("validation guarantees `{id}` is a noul answer"),
    }
}

/// Looks up one answer by its exact question ID.
///
/// After `validate_for` succeeds every question has exactly one answer of the
/// right kind, so the fallback is a defensive invariant, not a recoverable
/// error.
fn answer_for<'a>(response: &'a DecisionResponse, id: &str) -> &'a Answer {
    response
        .answers()
        .iter()
        .find_map(|(key, answer)| (key.as_str() == id).then_some(answer))
        .unwrap_or_else(|| unreachable!("validation guarantees answer `{id}` is present"))
}

#[cfg(test)]
mod tests {
    use ownai_core::{ProjectedFile, ProjectedItem, SourceSpan};
    use ownai_decisions::fake::FakeProvider;
    use ownai_decisions::{ChoiceAnswer, DecisionError, NoulAnswer, QuestionKind, ScoreAnswer};

    use super::*;

    const TEST_LIMIT: usize = 64 * 1024;

    fn lens() -> ReviewLens {
        ReviewLens::new(TEST_LIMIT).expect("valid lens")
    }

    fn qid(value: &str) -> QuestionId {
        QuestionId::new(value).expect("valid question id")
    }

    fn prob(value: f64) -> Probability {
        Probability::new(value).expect("valid probability")
    }

    fn sample_path() -> RepoPath {
        RepoPath::new("src/auth.rs").expect("valid path")
    }

    fn modified_method() -> ItemDiff {
        ItemDiff {
            path: sample_path(),
            language: Language::Rust,
            mode: ProjectionMode::Signatures,
            change: ItemChangeKind::Modified,
            stable_key: "impl Session::refresh".to_owned(),
            item_kind: ItemKind::Method,
            name: "refresh".to_owned(),
            old_text: Some("fn refresh(&mut self, token: Token) -> Result<(), Error>;".to_owned()),
            new_text: Some(
                "async fn refresh(&mut self, token: RefreshToken) -> Result<Session, Error>;"
                    .to_owned(),
            ),
        }
    }

    fn concern_answer() -> Answer {
        let mut probabilities = vec![(CONCERN_OPTIONS[0].0.to_owned(), prob(0.55))];
        for (key, _) in &CONCERN_OPTIONS[1..] {
            probabilities.push(((*key).to_owned(), prob(0.05)));
        }
        Answer::Choice(
            ChoiceAnswer::new(CONCERN_OPTIONS[0].0, probabilities, prob(0.8))
                .expect("valid concern answer"),
        )
    }

    fn score_answer(value: f64) -> Answer {
        let probabilities = RISK_LEVELS
            .iter()
            .map(|(label, _)| ((*label).to_owned(), prob(0.2)))
            .collect::<Vec<_>>();
        Answer::Score(
            ScoreAnswer::new(value, probabilities, prob(0.7)).expect("valid score answer"),
        )
    }

    fn noul_answer(value: f64) -> Answer {
        Answer::Noul(NoulAnswer::new(prob(value)))
    }

    fn valid_answers() -> Vec<(QuestionId, Answer)> {
        vec![
            (qid(CONCERN_ID), concern_answer()),
            (qid(RISK_ID), score_answer(3.0)),
            (qid(LIKELY_BREAKING_ID), noul_answer(0.71)),
            (qid(NEEDS_TESTS_ID), noul_answer(0.91)),
            (qid(NEEDS_DOCS_ID), noul_answer(0.43)),
            (qid(NEEDS_MIGRATION_ID), noul_answer(0.66)),
            (qid(SECURITY_SENSITIVE_ID), noul_answer(0.86)),
        ]
    }

    fn valid_response() -> DecisionResponse {
        DecisionResponse::new(
            "fake",
            "fake-v1",
            valid_answers(),
            Some(Usage {
                input_tokens: 128,
                output_tokens: 64,
            }),
        )
        .expect("valid response")
    }

    /// A provider that returns one scripted response without validating it, so
    /// tests can exercise the lens's own post-evaluation validation.
    struct NonValidatingProvider {
        response: DecisionResponse,
    }

    impl DecisionProvider for NonValidatingProvider {
        fn evaluate(&self, _request: &DecisionRequest) -> Result<DecisionResponse, DecisionError> {
            Ok(self.response.clone())
        }
    }

    fn invalid_response_error(response: DecisionResponse) -> ReviewError {
        let provider = NonValidatingProvider { response };
        lens()
            .review_item(&provider, modified_method())
            .expect_err("the lens rejects it")
    }

    fn projected(items: Vec<ProjectedItem>) -> ProjectedFile {
        ProjectedFile::new(sample_path(), Language::Rust, items)
    }

    fn projected_item(stable_key: &str, parent_key: Option<&str>, text: &str) -> ProjectedItem {
        ProjectedItem {
            stable_key: stable_key.to_owned(),
            parent_key: parent_key.map(str::to_owned),
            kind: ItemKind::Function,
            name: stable_key.to_owned(),
            span: SourceSpan {
                start_byte: 0,
                end_byte: 0,
                start_line: 0,
                start_column: 0,
                end_line: 0,
                end_column: 0,
            },
            canonical_text: text.to_owned(),
        }
    }

    fn file_diff(old: Option<ProjectedFile>, new: Option<ProjectedFile>) -> FileDiff {
        FileDiff {
            path: sample_path(),
            mode: ProjectionMode::Signatures,
            old,
            new,
        }
    }

    #[test]
    fn modified_rust_method_state_matches_the_pinned_v1_bytes() {
        let state = review_state(&modified_method());
        let bytes = serde_json::to_vec(&state).expect("state serializes");
        assert_eq!(
            String::from_utf8(bytes).expect("state is UTF-8"),
            concat!(
                r#"{"after":"async fn refresh(&mut self, token: RefreshToken) -> Result<Session, Error>;","#,
                r#""before":"fn refresh(&mut self, token: Token) -> Result<(), Error>;","#,
                r#""change":"modified","#,
                r#""item":{"kind":"method","name":"refresh","stable_key":"impl Session::refresh"},"#,
                r#""language":"rust","#,
                r#""path":{"bytes_hex":"7372632f617574682e7273","display":"src/auth.rs"},"#,
                r#""projection_mode":"signatures","#,
                r#""schema":"ownai.review-unit.v1"}"#,
            )
        );
    }

    #[test]
    fn non_utf8_paths_keep_raw_identity_in_bytes_hex() {
        let mut item = modified_method();
        item.path = RepoPath::new(b"src/\xFF/lib.rs").expect("valid raw path");

        let state = review_state(&item);

        assert_eq!(
            state["path"]["bytes_hex"],
            json!("7372632fff2f6c69622e7273")
        );
        assert_eq!(state["path"]["display"], json!("src/\\xFF/lib.rs"));
    }

    #[test]
    fn added_items_have_null_before_and_removed_items_have_null_after() {
        let mut added = modified_method();
        added.change = ItemChangeKind::Added;
        added.old_text = None;
        let mut removed = modified_method();
        removed.change = ItemChangeKind::Removed;
        removed.new_text = None;

        let added_state = review_state(&added);
        assert_eq!(added_state["change"], json!("added"));
        assert_eq!(added_state["before"], Value::Null);
        assert!(added_state["after"].is_string());

        let removed_state = review_state(&removed);
        assert_eq!(removed_state["change"], json!("removed"));
        assert_eq!(removed_state["after"], Value::Null);
        assert!(removed_state["before"].is_string());
    }

    #[test]
    fn every_projection_mode_has_a_stable_key() {
        let cases = [
            (ProjectionMode::Types, "types"),
            (ProjectionMode::Signatures, "signatures"),
        ];
        for (mode, key) in cases {
            assert_eq!(projection_mode_key(mode), key);
        }
    }

    #[test]
    fn every_language_has_a_stable_key() {
        let cases = [
            (Language::Elm, "elm"),
            (Language::Haskell, "haskell"),
            (Language::Python, "python"),
            (Language::Rust, "rust"),
        ];
        for (language, key) in cases {
            assert_eq!(language_key(language), key);
        }
    }

    #[test]
    fn every_item_kind_has_a_stable_key() {
        let cases = [
            (ItemKind::Module, "module"),
            (ItemKind::Type, "type"),
            (ItemKind::TypeAlias, "type-alias"),
            (ItemKind::Constructor, "constructor"),
            (ItemKind::Field, "field"),
            (ItemKind::Variant, "variant"),
            (ItemKind::Trait, "trait"),
            (ItemKind::TraitImplementation, "trait-implementation"),
            (ItemKind::AssociatedType, "associated-type"),
            (ItemKind::TypeFamily, "type-family"),
            (ItemKind::PatternSynonym, "pattern-synonym"),
            (ItemKind::Function, "function"),
            (ItemKind::Method, "method"),
            (ItemKind::Value, "value"),
            (ItemKind::Constant, "constant"),
            (ItemKind::Static, "static"),
            (ItemKind::Port, "port"),
            (ItemKind::Operator, "operator"),
            (ItemKind::ForeignBlock, "foreign-block"),
        ];
        for (kind, key) in cases {
            assert_eq!(item_kind_key(kind), key);
        }
    }

    #[test]
    fn every_change_kind_has_a_stable_key() {
        let cases = [
            (ItemChangeKind::Added, "added"),
            (ItemChangeKind::Removed, "removed"),
            (ItemChangeKind::Modified, "modified"),
        ];
        for (change, key) in cases {
            assert_eq!(change_key(change), key);
        }
    }

    #[test]
    fn one_request_carries_the_seven_fixed_questions_in_order() {
        let provider = FakeProvider::new(vec![Ok(valid_response())]);

        lens()
            .review_item(&provider, modified_method())
            .expect("review succeeds");

        let requests = provider.requests();
        assert_eq!(requests.len(), 1);
        let questions = requests[0].questions();
        let ids: Vec<&str> = questions
            .iter()
            .map(|question| question.id().as_str())
            .collect();
        assert_eq!(
            ids,
            vec![
                CONCERN_ID,
                RISK_ID,
                LIKELY_BREAKING_ID,
                NEEDS_TESTS_ID,
                NEEDS_DOCS_ID,
                NEEDS_MIGRATION_ID,
                SECURITY_SENSITIVE_ID,
            ]
        );
        let kinds: Vec<QuestionKind> = questions.iter().map(Question::kind).collect();
        assert_eq!(
            kinds,
            vec![
                QuestionKind::Choice,
                QuestionKind::Score,
                QuestionKind::Noul,
                QuestionKind::Noul,
                QuestionKind::Noul,
                QuestionKind::Noul,
                QuestionKind::Noul,
            ]
        );
    }

    #[test]
    fn concern_options_and_risk_levels_have_stable_order_and_keys() {
        let provider = FakeProvider::new(vec![Ok(valid_response())]);
        lens()
            .review_item(&provider, modified_method())
            .expect("review succeeds");
        let requests = provider.requests();
        let questions = requests[0].questions();

        let Question::Choice(concern) = &questions[0] else {
            panic!("concern must be a choice question");
        };
        let keys: Vec<&str> = concern.options().iter().map(ChoiceOption::key).collect();
        assert_eq!(
            keys,
            vec![
                "api-contract",
                "data-model",
                "authentication-authorization",
                "storage",
                "networking-integration",
                "configuration-operations",
                "observability",
                "user-interface",
                "testing-tooling",
                "other",
            ]
        );
        assert!(
            concern
                .options()
                .iter()
                .all(|option| option.description().is_some())
        );

        let Question::Score(risk) = &questions[1] else {
            panic!("risk must be a score question");
        };
        let labels: Vec<&str> = risk.levels().iter().map(ScoreLevel::label).collect();
        assert_eq!(
            labels,
            vec!["routine", "low", "moderate", "high", "critical"]
        );
        assert!(
            risk.levels()
                .iter()
                .all(|level| level.description().is_some())
        );
    }

    #[test]
    fn all_seven_valid_answers_decode_into_one_judgment() {
        let provider = FakeProvider::new(vec![Ok(valid_response())]);

        let reviewed = lens()
            .review_item(&provider, modified_method())
            .expect("review succeeds");

        let judgment = &reviewed.judgment;
        assert_eq!(judgment.concern.selected, "api-contract");
        assert_eq!(judgment.concern.confidence, prob(0.8));
        assert_eq!(judgment.concern.probabilities.len(), CONCERN_OPTIONS.len());
        assert_eq!(judgment.risk.raw_index, 3.0);
        assert_eq!(judgment.risk.display_value, 4.0);
        assert_eq!(judgment.risk.confidence, prob(0.7));
        assert_eq!(judgment.risk.probabilities.len(), RISK_LEVELS.len());
        assert_eq!(judgment.likely_breaking, prob(0.71));
        assert_eq!(judgment.needs_tests, prob(0.91));
        assert_eq!(judgment.needs_docs, prob(0.43));
        assert_eq!(judgment.needs_migration, prob(0.66));
        assert_eq!(judgment.security_sensitive, prob(0.86));
    }

    #[test]
    fn risk_retains_raw_index_and_offsets_the_display_value() {
        for raw_index in [0.0, 1.5, 3.0, 4.0] {
            let mut answers = valid_answers();
            answers[1] = (qid(RISK_ID), score_answer(raw_index));
            let response =
                DecisionResponse::new("fake", "fake-v1", answers, None).expect("valid response");
            let provider = NonValidatingProvider { response };

            let reviewed = lens()
                .review_item(&provider, modified_method())
                .expect("review succeeds");

            assert_eq!(reviewed.judgment.risk.raw_index, raw_index);
            assert_eq!(reviewed.judgment.risk.display_value, raw_index + 1.0);
        }
    }

    #[test]
    fn provider_model_and_usage_are_retained() {
        let provider = FakeProvider::new(vec![Ok(valid_response())]);

        let reviewed = lens()
            .review_item(&provider, modified_method())
            .expect("review succeeds");

        assert_eq!(reviewed.provider, "fake");
        assert_eq!(reviewed.model_revision, "fake-v1");
        assert_eq!(
            reviewed.usage,
            Some(Usage {
                input_tokens: 128,
                output_tokens: 64,
            })
        );
        assert_eq!(reviewed.item, modified_method());
    }

    #[test]
    fn review_file_preserves_order_and_retains_both_outcomes() {
        let old = projected(vec![
            projected_item("function a", None, "fn a();"),
            projected_item("function b", None, "fn b();"),
        ]);
        let new = projected(vec![
            projected_item("function a", None, "fn a(u8);"),
            projected_item("function b", None, "fn b(u8);"),
        ]);
        let diff = file_diff(Some(old), Some(new));
        let provider = FakeProvider::new(vec![
            Ok(valid_response()),
            Err(DecisionError::Provider {
                message: "boom".to_owned(),
            }),
        ]);

        let outcomes = lens().review_file(&provider, &diff);

        assert_eq!(outcomes.len(), 2);
        match &outcomes[0] {
            ReviewOutcome::Reviewed(reviewed) => assert_eq!(reviewed.item.stable_key, "function a"),
            other => panic!("expected a reviewed outcome, got {other:?}"),
        }
        match &outcomes[1] {
            ReviewOutcome::Failed { item, error } => {
                assert_eq!(item.stable_key, "function b");
                assert!(matches!(error, ReviewError::Provider { .. }));
            }
            other => panic!("expected a failed outcome, got {other:?}"),
        }
        assert_eq!(provider.requests().len(), 2);
    }

    #[test]
    fn nested_items_do_not_produce_an_independent_request() {
        let old = projected(vec![
            projected_item("type User", None, "struct User { id: u8 }"),
            projected_item("type User::field id", Some("type User"), "id: u8"),
        ]);
        let new = projected(vec![
            projected_item("type User", None, "struct User { id: u16 }"),
            projected_item("type User::field id", Some("type User"), "id: u16"),
        ]);
        let diff = file_diff(Some(old), Some(new));
        let provider = FakeProvider::new(vec![Ok(valid_response())]);

        let outcomes = lens().review_file(&provider, &diff);

        assert_eq!(outcomes.len(), 1);
        match &outcomes[0] {
            ReviewOutcome::Reviewed(reviewed) => assert_eq!(reviewed.item.stable_key, "type User"),
            other => panic!("expected a reviewed outcome, got {other:?}"),
        }
        assert_eq!(provider.requests().len(), 1);
    }

    #[test]
    fn oversized_state_makes_no_provider_call() {
        let provider = FakeProvider::new(vec![]);

        let error = ReviewLens::new(1)
            .expect("non-zero limit is valid")
            .review_item(&provider, modified_method())
            .expect_err("state is too large");

        match error {
            ReviewError::StateTooLarge {
                path,
                stable_key,
                actual_bytes,
                limit_bytes,
            } => {
                assert_eq!(path, sample_path());
                assert_eq!(stable_key, "impl Session::refresh");
                assert!(actual_bytes > 1);
                assert_eq!(limit_bytes, 1);
            }
            other => panic!("expected StateTooLarge, got {other:?}"),
        }
        assert!(provider.requests().is_empty());
    }

    #[test]
    fn a_zero_state_limit_is_rejected_during_construction() {
        assert!(matches!(
            ReviewLens::new(0),
            Err(ReviewError::InvalidStateLimit { limit_bytes: 0 })
        ));
    }

    #[test]
    fn provider_failure_retains_item_context() {
        let provider = FakeProvider::new(vec![Err(DecisionError::Provider {
            message: "boom".to_owned(),
        })]);

        let error = lens()
            .review_item(&provider, modified_method())
            .expect_err("provider fails");

        match error {
            ReviewError::Provider {
                path,
                stable_key,
                source,
            } => {
                assert_eq!(path, sample_path());
                assert_eq!(stable_key, "impl Session::refresh");
                assert!(matches!(source, DecisionError::Provider { .. }));
            }
            other => panic!("expected Provider, got {other:?}"),
        }
    }

    #[test]
    fn a_fake_providers_mismatched_response_arrives_as_a_provider_error() {
        let provider = FakeProvider::new(vec![Ok(DecisionResponse::new(
            "fake",
            "fake-v1",
            vec![],
            None,
        )
        .expect("valid shape"))]);

        let error = lens()
            .review_item(&provider, modified_method())
            .expect_err("the fake rejects the response");

        match error {
            ReviewError::Provider { source, .. } => {
                assert!(matches!(source, DecisionError::InvalidResponse { .. }));
            }
            other => panic!("expected Provider, got {other:?}"),
        }
    }

    #[test]
    fn missing_answer_is_rejected_as_invalid_response() {
        let response = DecisionResponse::new("fake", "fake-v1", vec![], None).expect("valid shape");

        match invalid_response_error(response) {
            ReviewError::InvalidResponse {
                path,
                stable_key,
                source,
            } => {
                assert_eq!(path, sample_path());
                assert_eq!(stable_key, "impl Session::refresh");
                assert!(matches!(source, ValidationError::MissingAnswer { .. }));
            }
            other => panic!("expected InvalidResponse, got {other:?}"),
        }
    }

    #[test]
    fn extra_answer_is_rejected_as_invalid_response() {
        let mut answers = valid_answers();
        answers.push((qid("surprise"), noul_answer(0.5)));
        let response =
            DecisionResponse::new("fake", "fake-v1", answers, None).expect("valid shape");

        match invalid_response_error(response) {
            ReviewError::InvalidResponse { source, .. } => {
                assert!(matches!(source, ValidationError::UnexpectedAnswer { .. }));
            }
            other => panic!("expected InvalidResponse, got {other:?}"),
        }
    }

    #[test]
    fn wrong_answer_kind_is_rejected_as_invalid_response() {
        let mut answers = valid_answers();
        answers[0] = (qid(CONCERN_ID), noul_answer(0.5));
        let response =
            DecisionResponse::new("fake", "fake-v1", answers, None).expect("valid shape");

        match invalid_response_error(response) {
            ReviewError::InvalidResponse { source, .. } => {
                assert!(matches!(source, ValidationError::WrongAnswerType { .. }));
            }
            other => panic!("expected InvalidResponse, got {other:?}"),
        }
    }

    #[test]
    fn option_mismatch_is_rejected_as_invalid_response() {
        let mut answers = valid_answers();
        let mismatched = Answer::Choice(
            ChoiceAnswer::new(
                "different",
                vec![
                    ("different".to_owned(), prob(0.9)),
                    ("other".to_owned(), prob(0.1)),
                ],
                prob(0.9),
            )
            .expect("internally valid answer"),
        );
        answers[0] = (qid(CONCERN_ID), mismatched);
        let response =
            DecisionResponse::new("fake", "fake-v1", answers, None).expect("valid shape");

        match invalid_response_error(response) {
            ReviewError::InvalidResponse { source, .. } => {
                assert!(matches!(source, ValidationError::OptionMismatch { .. }));
            }
            other => panic!("expected InvalidResponse, got {other:?}"),
        }
    }

    #[test]
    fn out_of_range_score_is_rejected_as_invalid_response() {
        let mut answers = valid_answers();
        answers[1] = (qid(RISK_ID), score_answer(5.0));
        let response =
            DecisionResponse::new("fake", "fake-v1", answers, None).expect("valid shape");

        match invalid_response_error(response) {
            ReviewError::InvalidResponse { source, .. } => {
                assert!(matches!(source, ValidationError::ScoreOutOfRange { .. }));
            }
            other => panic!("expected InvalidResponse, got {other:?}"),
        }
    }
}
