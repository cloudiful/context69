//! Typed decision question/answer contracts (issue #89 P4).
//!
//! Decision templates carry an explicit list of independently evaluated
//! questions; a decision result is a typed answer list, never generative
//! JSON. The bounds below are Context69 application limits, not vendor
//! claims: questions 1..=32 per template, choice options 2..=64, score
//! levels 2..=32. Template-kind history is snapshotted per result row, so
//! these shapes only ever validate the definition under registration and
//! decode the snapshot a row carries.

use anyhow::{Result, anyhow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Minimum/maximum questions per decision template (application limit).
pub const DECISION_QUESTIONS_MIN: usize = 1;
/// Maximum questions per decision template (application limit).
pub const DECISION_QUESTIONS_MAX: usize = 32;
/// Minimum options per choice question (application limit).
pub const CHOICE_OPTIONS_MIN: usize = 2;
/// Maximum options per choice question (application limit).
pub const CHOICE_OPTIONS_MAX: usize = 64;
/// Minimum levels per score question (application limit).
pub const SCORE_LEVELS_MIN: usize = 2;
/// Maximum levels per score question (application limit).
pub const SCORE_LEVELS_MAX: usize = 32;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PredicateQuestion {
    pub name: String,
    pub instructions: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChoiceOption {
    pub value: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChoiceQuestion {
    pub name: String,
    pub instructions: String,
    pub choices: Vec<ChoiceOption>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScoreLevel {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScoreQuestion {
    pub name: String,
    pub instructions: String,
    pub levels: Vec<ScoreLevel>,
}

/// One independently evaluated decision question, tagged by `type`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExtractionQuestion {
    Predicate(PredicateQuestion),
    Choice(ChoiceQuestion),
    Score(ScoreQuestion),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DecisionDefinition {
    pub questions: Vec<ExtractionQuestion>,
}

impl DecisionDefinition {
    /// Validate the application limits: question count, nonempty unique
    /// names, nonempty instructions, and per-kind option/level bounds with
    /// unique nonempty values/labels and nonempty descriptions.
    /// Question-answer matching and probability ranges belong to the real
    /// decision adapter (later phase), not to registration.
    pub fn validate(&self) -> Result<()> {
        if self.questions.len() < DECISION_QUESTIONS_MIN
            || self.questions.len() > DECISION_QUESTIONS_MAX
        {
            return Err(anyhow!(
                "decision questions must hold {}..={} entries, got {}",
                DECISION_QUESTIONS_MIN,
                DECISION_QUESTIONS_MAX,
                self.questions.len()
            ));
        }
        let mut names = std::collections::HashSet::new();
        for question in &self.questions {
            let (name, instructions) = match question {
                ExtractionQuestion::Predicate(q) => (q.name.as_str(), q.instructions.as_str()),
                ExtractionQuestion::Choice(q) => (q.name.as_str(), q.instructions.as_str()),
                ExtractionQuestion::Score(q) => (q.name.as_str(), q.instructions.as_str()),
            };
            if name.trim().is_empty() {
                return Err(anyhow!("decision question names must not be empty"));
            }
            if !names.insert(name) {
                return Err(anyhow!("duplicate decision question name: {name}"));
            }
            if instructions.trim().is_empty() {
                return Err(anyhow!(
                    "decision question instructions must not be empty: {name}"
                ));
            }
            match question {
                ExtractionQuestion::Predicate(_) => {}
                ExtractionQuestion::Choice(q) => validate_choice_options(name, &q.choices)?,
                ExtractionQuestion::Score(q) => validate_score_levels(name, &q.levels)?,
            }
        }
        Ok(())
    }
}

fn validate_choice_options(name: &str, choices: &[ChoiceOption]) -> Result<()> {
    if choices.len() < CHOICE_OPTIONS_MIN || choices.len() > CHOICE_OPTIONS_MAX {
        return Err(anyhow!(
            "choice question '{name}' must hold {}..={} options, got {}",
            CHOICE_OPTIONS_MIN,
            CHOICE_OPTIONS_MAX,
            choices.len()
        ));
    }
    let mut values = std::collections::HashSet::new();
    for choice in choices {
        if choice.value.trim().is_empty() {
            return Err(anyhow!(
                "choice question '{name}' option values must not be empty"
            ));
        }
        if !values.insert(choice.value.as_str()) {
            return Err(anyhow!(
                "duplicate choice value '{}' in question '{name}'",
                choice.value
            ));
        }
        if choice.description.trim().is_empty() {
            return Err(anyhow!(
                "choice question '{name}' option descriptions must not be empty"
            ));
        }
    }
    Ok(())
}

fn validate_score_levels(name: &str, levels: &[ScoreLevel]) -> Result<()> {
    if levels.len() < SCORE_LEVELS_MIN || levels.len() > SCORE_LEVELS_MAX {
        return Err(anyhow!(
            "score question '{name}' must hold {}..={} levels, got {}",
            SCORE_LEVELS_MIN,
            SCORE_LEVELS_MAX,
            levels.len()
        ));
    }
    let mut labels = std::collections::HashSet::new();
    for level in levels {
        if level.label.trim().is_empty() {
            return Err(anyhow!(
                "score question '{name}' level labels must not be empty"
            ));
        }
        if !labels.insert(level.label.as_str()) {
            return Err(anyhow!(
                "duplicate score label '{}' in question '{name}'",
                level.label
            ));
        }
        if level.description.trim().is_empty() {
            return Err(anyhow!(
                "score question '{name}' level descriptions must not be empty"
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PredicateAnswer {
    pub name: String,
    pub probability: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChoiceProbability {
    pub value: String,
    pub probability: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChoiceAnswer {
    pub name: String,
    pub choice: String,
    pub probabilities: Vec<ChoiceProbability>,
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScoreProbability {
    /// Zero-based index into the question's levels.
    pub value: i32,
    pub label: String,
    pub probability: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScoreAnswer {
    pub name: String,
    pub score: f64,
    pub probabilities: Vec<ScoreProbability>,
    pub confidence: f64,
}

/// A refusal is parsed failure evidence, never a successfully persisted
/// decision: execution treats it as failure (later adapter phase).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RefusalAnswer {
    pub name: String,
}

/// One typed decision answer, tagged by `type`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionAnswer {
    Predicate(PredicateAnswer),
    Choice(ChoiceAnswer),
    Score(ScoreAnswer),
    Refusal(RefusalAnswer),
}
