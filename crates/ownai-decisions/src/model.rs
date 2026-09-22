//! Validated provider-independent questions, answers, requests, and responses.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde_json::Value;

use crate::ValidationError;

const PROBABILITY_SUM_TOLERANCE: f64 = 0.000_1;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct QuestionId(String);

impl QuestionId {
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        non_empty("question id", &value)?;
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(ValidationError::InvalidQuestionId { value });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for QuestionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Probability(f64);

impl Probability {
    pub fn new(value: f64) -> Result<Self, ValidationError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(ValidationError::InvalidProbability {
                value: value.to_string(),
            });
        }
        Ok(Self(value))
    }

    pub fn get(self) -> f64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChoiceOption {
    key: String,
    description: Option<String>,
}

impl ChoiceOption {
    pub fn new(
        key: impl Into<String>,
        description: Option<String>,
    ) -> Result<Self, ValidationError> {
        let key = key.into();
        non_empty("choice option key", &key)?;
        if let Some(value) = &description {
            non_empty("choice option description", value)?;
        }
        Ok(Self { key, description })
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScoreLevel {
    label: String,
    description: Option<String>,
}

impl ScoreLevel {
    pub fn new(
        label: impl Into<String>,
        description: Option<String>,
    ) -> Result<Self, ValidationError> {
        let label = label.into();
        non_empty("score level label", &label)?;
        if let Some(value) = &description {
            non_empty("score level description", value)?;
        }
        Ok(Self { label, description })
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChoiceQuestion {
    id: QuestionId,
    instructions: String,
    options: Vec<ChoiceOption>,
}

impl ChoiceQuestion {
    pub fn new(
        id: QuestionId,
        instructions: impl Into<String>,
        options: Vec<ChoiceOption>,
    ) -> Result<Self, ValidationError> {
        let instructions = instructions.into();
        non_empty("choice instructions", &instructions)?;
        validate_unique_options(&id, options.iter().map(ChoiceOption::key), 2)?;
        Ok(Self {
            id,
            instructions,
            options,
        })
    }

    pub fn id(&self) -> &QuestionId {
        &self.id
    }

    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    pub fn options(&self) -> &[ChoiceOption] {
        &self.options
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScoreQuestion {
    id: QuestionId,
    instructions: String,
    levels: Vec<ScoreLevel>,
}

impl ScoreQuestion {
    pub fn new(
        id: QuestionId,
        instructions: impl Into<String>,
        levels: Vec<ScoreLevel>,
    ) -> Result<Self, ValidationError> {
        let instructions = instructions.into();
        non_empty("score instructions", &instructions)?;
        validate_unique_options(&id, levels.iter().map(ScoreLevel::label), 2)?;
        Ok(Self {
            id,
            instructions,
            levels,
        })
    }

    pub fn id(&self) -> &QuestionId {
        &self.id
    }

    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    pub fn levels(&self) -> &[ScoreLevel] {
        &self.levels
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoulQuestion {
    id: QuestionId,
    instructions: String,
}

impl NoulQuestion {
    pub fn new(id: QuestionId, instructions: impl Into<String>) -> Result<Self, ValidationError> {
        let instructions = instructions.into();
        non_empty("noul instructions", &instructions)?;
        Ok(Self { id, instructions })
    }

    pub fn id(&self) -> &QuestionId {
        &self.id
    }

    pub fn instructions(&self) -> &str {
        &self.instructions
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Question {
    Choice(ChoiceQuestion),
    Score(ScoreQuestion),
    Noul(NoulQuestion),
}

impl Question {
    pub fn id(&self) -> &QuestionId {
        match self {
            Self::Choice(question) => question.id(),
            Self::Score(question) => question.id(),
            Self::Noul(question) => question.id(),
        }
    }

    pub fn kind(&self) -> QuestionKind {
        match self {
            Self::Choice(_) => QuestionKind::Choice,
            Self::Score(_) => QuestionKind::Score,
            Self::Noul(_) => QuestionKind::Noul,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuestionKind {
    Choice,
    Score,
    Noul,
}

impl QuestionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Choice => "choice",
            Self::Score => "score",
            Self::Noul => "noul",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecisionRequest {
    state: Value,
    questions: Vec<Question>,
}

impl DecisionRequest {
    pub fn new(state: Value, questions: Vec<Question>) -> Result<Self, ValidationError> {
        if questions.is_empty() {
            return Err(ValidationError::Empty { field: "questions" });
        }
        let mut ids = BTreeSet::new();
        for question in &questions {
            if !ids.insert(question.id().as_str()) {
                return Err(ValidationError::DuplicateQuestion {
                    id: question.id().to_string(),
                });
            }
        }
        Ok(Self { state, questions })
    }

    pub fn state(&self) -> &Value {
        &self.state
    }

    pub fn questions(&self) -> &[Question] {
        &self.questions
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceAnswer {
    selected: String,
    probabilities: BTreeMap<String, Probability>,
    confidence: Probability,
}

impl ChoiceAnswer {
    pub fn new(
        selected: impl Into<String>,
        probabilities: Vec<(String, Probability)>,
        confidence: Probability,
    ) -> Result<Self, ValidationError> {
        let selected = selected.into();
        non_empty("selected choice", &selected)?;
        let probabilities = probability_map(probabilities)?;
        let Some(selected_probability) = probabilities.get(&selected) else {
            return Err(ValidationError::UnknownSelectedChoice { selected });
        };
        if probabilities
            .values()
            .any(|probability| probability.get() > selected_probability.get())
        {
            return Err(ValidationError::SelectedChoiceNotMaximum { selected });
        }
        Ok(Self {
            selected,
            probabilities,
            confidence,
        })
    }

    pub fn selected(&self) -> &str {
        &self.selected
    }

    pub fn probabilities(&self) -> &BTreeMap<String, Probability> {
        &self.probabilities
    }

    pub fn confidence(&self) -> Probability {
        self.confidence
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScoreAnswer {
    value: f64,
    probabilities: BTreeMap<String, Probability>,
    confidence: Probability,
}

impl ScoreAnswer {
    pub fn new(
        value: f64,
        probabilities: Vec<(String, Probability)>,
        confidence: Probability,
    ) -> Result<Self, ValidationError> {
        if !value.is_finite() {
            return Err(ValidationError::InvalidScore {
                value: value.to_string(),
            });
        }
        Ok(Self {
            value,
            probabilities: probability_map(probabilities)?,
            confidence,
        })
    }

    pub fn value(&self) -> f64 {
        self.value
    }

    pub fn probabilities(&self) -> &BTreeMap<String, Probability> {
        &self.probabilities
    }

    pub fn confidence(&self) -> Probability {
        self.confidence
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoulAnswer {
    probability: Probability,
}

impl NoulAnswer {
    pub fn new(probability: Probability) -> Self {
        Self { probability }
    }

    pub fn probability(self) -> Probability {
        self.probability
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    Choice(ChoiceAnswer),
    Score(ScoreAnswer),
    Noul(NoulAnswer),
}

impl Answer {
    pub fn kind(&self) -> AnswerKind {
        match self {
            Self::Choice(_) => AnswerKind::Choice,
            Self::Score(_) => AnswerKind::Score,
            Self::Noul(_) => AnswerKind::Noul,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnswerKind {
    Choice,
    Score,
    Noul,
}

impl AnswerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Choice => "choice",
            Self::Score => "score",
            Self::Noul => "noul",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecisionResponse {
    provider: String,
    model_revision: String,
    answers: BTreeMap<QuestionId, Answer>,
    usage: Option<Usage>,
}

impl DecisionResponse {
    pub fn new(
        provider: impl Into<String>,
        model_revision: impl Into<String>,
        answers: Vec<(QuestionId, Answer)>,
        usage: Option<Usage>,
    ) -> Result<Self, ValidationError> {
        let provider = provider.into();
        let model_revision = model_revision.into();
        non_empty("provider", &provider)?;
        non_empty("model revision", &model_revision)?;

        let mut answer_map = BTreeMap::new();
        for (id, answer) in answers {
            if answer_map.insert(id.clone(), answer).is_some() {
                return Err(ValidationError::DuplicateAnswer { id: id.to_string() });
            }
        }
        Ok(Self {
            provider,
            model_revision,
            answers: answer_map,
            usage,
        })
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn model_revision(&self) -> &str {
        &self.model_revision
    }

    pub fn answers(&self) -> &BTreeMap<QuestionId, Answer> {
        &self.answers
    }

    pub fn usage(&self) -> Option<Usage> {
        self.usage
    }

    /// Checks exact answer coverage, answer kinds, option identities, and score
    /// range against the request that produced this response.
    pub fn validate_for(&self, request: &DecisionRequest) -> Result<(), ValidationError> {
        for question in request.questions() {
            let id = question.id();
            let Some(answer) = self.answers.get(id) else {
                return Err(ValidationError::MissingAnswer { id: id.to_string() });
            };
            validate_answer(id, question, answer)?;
        }
        for id in self.answers.keys() {
            if !request
                .questions()
                .iter()
                .any(|question| question.id() == id)
            {
                return Err(ValidationError::UnexpectedAnswer { id: id.to_string() });
            }
        }
        Ok(())
    }
}

fn validate_answer(
    id: &QuestionId,
    question: &Question,
    answer: &Answer,
) -> Result<(), ValidationError> {
    match (question, answer) {
        (Question::Choice(question), Answer::Choice(answer)) => {
            let expected: BTreeSet<&str> =
                question.options().iter().map(ChoiceOption::key).collect();
            let actual: BTreeSet<&str> =
                answer.probabilities().keys().map(String::as_str).collect();
            if expected != actual {
                return Err(ValidationError::OptionMismatch { id: id.to_string() });
            }
        }
        (Question::Score(question), Answer::Score(answer)) => {
            let expected: Vec<&str> = question.levels().iter().map(ScoreLevel::label).collect();
            let actual: BTreeSet<&str> =
                answer.probabilities().keys().map(String::as_str).collect();
            if expected.iter().copied().collect::<BTreeSet<_>>() != actual {
                return Err(ValidationError::OptionMismatch { id: id.to_string() });
            }
            let maximum = expected.len() - 1;
            if !(0.0..=maximum as f64).contains(&answer.value()) {
                return Err(ValidationError::ScoreOutOfRange {
                    id: id.to_string(),
                    maximum,
                    value: answer.value().to_string(),
                });
            }
        }
        (Question::Noul(_), Answer::Noul(_)) => {}
        _ => {
            return Err(ValidationError::WrongAnswerType {
                id: id.to_string(),
                expected: question.kind().as_str(),
                actual: answer.kind().as_str(),
            });
        }
    }
    Ok(())
}

fn non_empty(field: &'static str, value: &str) -> Result<(), ValidationError> {
    if value.trim().is_empty() {
        Err(ValidationError::Empty { field })
    } else {
        Ok(())
    }
}

fn validate_unique_options<'a>(
    question: &QuestionId,
    options: impl Iterator<Item = &'a str>,
    minimum: usize,
) -> Result<(), ValidationError> {
    let mut count = 0;
    let mut keys = BTreeSet::new();
    for key in options {
        count += 1;
        if !keys.insert(key) {
            return Err(ValidationError::DuplicateOption {
                question: question.to_string(),
                key: key.to_owned(),
            });
        }
    }
    if count < minimum {
        return Err(ValidationError::TooFewOptions {
            question: question.to_string(),
            minimum,
        });
    }
    Ok(())
}

fn probability_map(
    probabilities: Vec<(String, Probability)>,
) -> Result<BTreeMap<String, Probability>, ValidationError> {
    if probabilities.is_empty() {
        return Err(ValidationError::EmptyDistribution);
    }
    let mut result = BTreeMap::new();
    for (key, probability) in probabilities {
        non_empty("probability key", &key)?;
        if result.insert(key.clone(), probability).is_some() {
            return Err(ValidationError::DuplicateProbability { key });
        }
    }
    let sum: f64 = result.values().map(|probability| probability.get()).sum();
    if (sum - 1.0).abs() > PROBABILITY_SUM_TOLERANCE {
        return Err(ValidationError::ProbabilitySum {
            sum: sum.to_string(),
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn id(value: &str) -> QuestionId {
        QuestionId::new(value).expect("valid id")
    }

    fn probability(value: f64) -> Probability {
        Probability::new(value).expect("valid probability")
    }

    fn option(key: &str) -> ChoiceOption {
        ChoiceOption::new(key, None).expect("valid option")
    }

    fn level(label: &str) -> ScoreLevel {
        ScoreLevel::new(label, None).expect("valid level")
    }

    fn choice_question() -> Question {
        Question::Choice(
            ChoiceQuestion::new(
                id("concern"),
                "Which concern?",
                vec![option("api"), option("other")],
            )
            .expect("valid question"),
        )
    }

    fn choice_answer() -> Answer {
        Answer::Choice(
            ChoiceAnswer::new(
                "api",
                vec![
                    ("api".to_owned(), probability(0.8)),
                    ("other".to_owned(), probability(0.2)),
                ],
                probability(0.7),
            )
            .expect("valid answer"),
        )
    }

    #[test]
    fn question_ids_are_restricted_to_portable_keys() {
        assert!(QuestionId::new("needs_tests-2").is_ok());
        assert!(matches!(
            QuestionId::new("needs tests"),
            Err(ValidationError::InvalidQuestionId { .. })
        ));
    }

    #[test]
    fn requests_reject_duplicate_question_ids() {
        let question = choice_question();
        let error = DecisionRequest::new(json!({}), vec![question.clone(), question])
            .expect_err("duplicate id");
        assert!(matches!(error, ValidationError::DuplicateQuestion { .. }));
    }

    #[test]
    fn choices_require_distinct_options() {
        let error = ChoiceQuestion::new(
            id("concern"),
            "Which concern?",
            vec![option("api"), option("api")],
        )
        .expect_err("duplicate option");
        assert!(matches!(error, ValidationError::DuplicateOption { .. }));
    }

    #[test]
    fn probability_rejects_non_finite_and_out_of_range_values() {
        for value in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
            assert!(matches!(
                Probability::new(value),
                Err(ValidationError::InvalidProbability { .. })
            ));
        }
    }

    #[test]
    fn choice_answers_validate_distribution_and_selection() {
        let wrong_sum = ChoiceAnswer::new(
            "api",
            vec![
                ("api".to_owned(), probability(0.4)),
                ("other".to_owned(), probability(0.4)),
            ],
            probability(0.5),
        )
        .expect_err("invalid sum");
        assert!(matches!(wrong_sum, ValidationError::ProbabilitySum { .. }));

        let not_maximum = ChoiceAnswer::new(
            "api",
            vec![
                ("api".to_owned(), probability(0.4)),
                ("other".to_owned(), probability(0.6)),
            ],
            probability(0.5),
        )
        .expect_err("selected choice is not maximum");
        assert!(matches!(
            not_maximum,
            ValidationError::SelectedChoiceNotMaximum { .. }
        ));
    }

    #[test]
    fn responses_require_exact_question_coverage_and_types() {
        let request = DecisionRequest::new(json!({}), vec![choice_question()]).unwrap();
        let missing = DecisionResponse::new("fake", "v1", vec![], None).unwrap();
        assert!(matches!(
            missing.validate_for(&request),
            Err(ValidationError::MissingAnswer { .. })
        ));

        let wrong = DecisionResponse::new(
            "fake",
            "v1",
            vec![(
                id("concern"),
                Answer::Noul(NoulAnswer::new(probability(0.5))),
            )],
            None,
        )
        .unwrap();
        assert!(matches!(
            wrong.validate_for(&request),
            Err(ValidationError::WrongAnswerType { .. })
        ));
    }

    #[test]
    fn responses_require_the_question_option_set() {
        let request = DecisionRequest::new(json!({}), vec![choice_question()]).unwrap();
        let answer = Answer::Choice(
            ChoiceAnswer::new(
                "api",
                vec![
                    ("api".to_owned(), probability(0.8)),
                    ("different".to_owned(), probability(0.2)),
                ],
                probability(0.7),
            )
            .unwrap(),
        );
        let response =
            DecisionResponse::new("fake", "v1", vec![(id("concern"), answer)], None).unwrap();
        assert!(matches!(
            response.validate_for(&request),
            Err(ValidationError::OptionMismatch { .. })
        ));
    }

    #[test]
    fn score_answers_are_checked_against_the_question_range() {
        let question = Question::Score(
            ScoreQuestion::new(
                id("risk"),
                "How risky?",
                vec![level("low"), level("medium"), level("high")],
            )
            .unwrap(),
        );
        let request = DecisionRequest::new(json!({}), vec![question]).unwrap();
        let answer = Answer::Score(
            ScoreAnswer::new(
                2.5,
                vec![
                    ("low".to_owned(), probability(0.1)),
                    ("medium".to_owned(), probability(0.2)),
                    ("high".to_owned(), probability(0.7)),
                ],
                probability(0.6),
            )
            .unwrap(),
        );
        let response = DecisionResponse::new("fake", "v1", vec![(id("risk"), answer)], None)
            .expect("response shape");
        assert!(matches!(
            response.validate_for(&request),
            Err(ValidationError::ScoreOutOfRange { .. })
        ));
    }

    #[test]
    fn a_matching_response_is_valid() {
        let request = DecisionRequest::new(json!({}), vec![choice_question()]).unwrap();
        let response = DecisionResponse::new(
            "fake",
            "fake-v1",
            vec![(id("concern"), choice_answer())],
            Some(Usage {
                input_tokens: 10,
                output_tokens: 2,
            }),
        )
        .unwrap();

        assert_eq!(response.validate_for(&request), Ok(()));
        assert_eq!(response.provider(), "fake");
        assert_eq!(response.model_revision(), "fake-v1");
    }
}
