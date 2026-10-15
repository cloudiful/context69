//! Luna text Decisions transport and strict typed response validation (issue #89 P5a).
//!
//! Adapter-only seam: a real strongly typed `reqwest` POST to `{base}/decisions`
//! with bearer auth, plus independent strict validation of the typed answers.
//! Not yet reachable from the live worker; the P4 unavailable branch stays
//! untouched until the configured integration phase.
//!
//! Numeric policy is local validation policy, not a vendor claim: distribution
//! sums and score weighted averages must hold within
//! [`PROBABILITY_TOLERANCE`]; NaN or out-of-range values deny; low confidence
//! is still a valid success (no confidence cutoff).
//!
//! Error hygiene: response bodies may echo secrets, so non-2xx responses map
//! to status-only [`ProviderHttpError`] with an explicitly empty body at this
//! boundary, and JSON/validation failures use safe fixed wording without
//! request headers, bodies, or key material. The existing global classifier
//! applies unchanged.

use anyhow::Result;
use context69_contracts_extraction::{
    ChoiceAnswer, ChoiceQuestion, DecisionAnswer, DecisionDefinition, ExtractionQuestion,
    ScoreAnswer, ScoreQuestion,
};
use context69_llm_support::normalize_endpoint;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{ProviderHttpError, ProviderPayloadError};

/// First-adapter Luna model. Rejected before any HTTP when mismatched.
pub const DECISION_MODEL: &str = "gpt-6-luna";

/// Local validation tolerance for distribution sums and score weighted
/// averages. Precision is not vendor-guaranteed; this is Context69 policy.
const PROBABILITY_TOLERANCE: f64 = 1e-3;

/// Credential-bearing Luna configuration. Intentionally carries no `Debug`
/// derive so a key can never appear in debug output, logs, or error chains.
#[derive(Clone)]
pub struct DecisionProviderConfig {
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
}

/// Typed adapter input. `text` reuses the existing `user_content`
/// (title/summary/body/parameters) as the Decisions text input string.
#[derive(Debug, Clone, Copy)]
pub struct DecisionInput<'a> {
    pub text: &'a str,
    pub definition: &'a DecisionDefinition,
}

/// Canonical typed answers. Refusals never surface here as success; they fail
/// through the payload-error path.
pub type DecisionAnswers = Vec<DecisionAnswer>;

/// Typed Luna Decisions call.
///
/// Validates the definition locally before send, POSTs
/// `{model, input: text, questions}` to `{base}/decisions` with bearer auth,
/// ignores unrelated vendor envelope fields, and strictly validates the
/// canonical answers. Refusal, missing, malformed, or inconsistent answers
/// fail safely; HTTP 429/5xx keep the existing retry classification via the
/// status-only error below.
pub async fn decide(
    client: &reqwest::Client,
    config: &DecisionProviderConfig,
    input: &DecisionInput<'_>,
) -> Result<DecisionAnswers> {
    input.definition.validate().map_err(|error| {
        anyhow::Error::new(ProviderPayloadError(format!(
            "decision definition is invalid: {error:#}"
        )))
    })?;
    let url = decisions_url(&config.endpoint)?;
    require_model(&config.model)?;
    require_key(&config.api_key)?;
    let body = json!({
        "model": config.model,
        "input": input.text,
        "questions": input.definition.questions,
    });
    let response = client
        .post(url)
        .bearer_auth(&config.api_key)
        .json(&body)
        .send()
        .await
        .map_err(anyhow::Error::new)?;
    let status = response.status();
    if !status.is_success() {
        // Status-only mapping at this boundary: response bodies may echo
        // secrets, so they are explicitly dropped here. The existing global
        // classifier still sees the status for 429/5xx handling.
        return Err(anyhow::Error::new(ProviderHttpError {
            status: status.as_u16(),
            body: String::new(),
        }));
    }
    let text = response.text().await.map_err(anyhow::Error::new)?;
    let value: Value = serde_json::from_str(&text).map_err(|_| {
        anyhow::Error::new(ProviderPayloadError(
            "decision response is not JSON".to_string(),
        ))
    })?;
    let wire: WireDecisionResponse = serde_json::from_value(value).map_err(|_| {
        anyhow::Error::new(ProviderPayloadError(
            "decision response is malformed".to_string(),
        ))
    })?;
    validate_decision_answers(input.definition, &wire.answers)?;
    Ok(wire.answers)
}

fn decisions_url(endpoint: &str) -> Result<String> {
    if endpoint.trim().is_empty() {
        return Err(anyhow::Error::new(ProviderPayloadError(
            "decision endpoint is not configured".to_string(),
        )));
    }
    normalize_endpoint(Some(endpoint), "/decisions").map_err(|_| {
        anyhow::Error::new(ProviderPayloadError(
            "decision endpoint is not configured".to_string(),
        ))
    })
}

fn require_model(model: &str) -> Result<()> {
    if model.trim().is_empty() {
        return Err(anyhow::Error::new(ProviderPayloadError(
            "decision model is not configured".to_string(),
        )));
    }
    if model != DECISION_MODEL {
        return Err(anyhow::Error::new(ProviderPayloadError(
            "decision model must be gpt-6-luna".to_string(),
        )));
    }
    Ok(())
}

fn require_key(api_key: &str) -> Result<()> {
    if api_key.trim().is_empty() {
        return Err(anyhow::Error::new(ProviderPayloadError(
            "decision api_key is not configured".to_string(),
        )));
    }
    Ok(())
}

/// Vendor envelope: typed `answers` plus any unrelated vendor metadata, which
/// is ignored. No `deny_unknown_fields` here; public DTOs stay strict.
#[derive(Debug, Deserialize)]
struct WireDecisionResponse {
    answers: Vec<DecisionAnswer>,
}

fn validate_decision_answers(
    definition: &DecisionDefinition,
    answers: &[DecisionAnswer],
) -> Result<()> {
    if answers.len() != definition.questions.len() {
        return Err(anyhow::Error::new(ProviderPayloadError(format!(
            "decision answers must hold {} entries, got {}",
            definition.questions.len(),
            answers.len()
        ))));
    }
    let mut questions_by_name = std::collections::HashMap::new();
    for question in &definition.questions {
        let name = question_name(question);
        if questions_by_name.insert(name, question).is_some() {
            return Err(anyhow::Error::new(ProviderPayloadError(format!(
                "duplicate decision question name: {name}"
            ))));
        }
    }
    let mut seen_answers = std::collections::HashSet::new();
    for answer in answers {
        if matches!(answer, DecisionAnswer::Refusal(_)) {
            let name = answer_name(answer);
            return Err(anyhow::Error::new(ProviderPayloadError(format!(
                "decision refused for question: {name}"
            ))));
        }
        let name = answer_name(answer);
        if !seen_answers.insert(name) {
            return Err(anyhow::Error::new(ProviderPayloadError(format!(
                "duplicate decision answer name: {name}"
            ))));
        }
        let Some(question) = questions_by_name.get(name) else {
            return Err(anyhow::Error::new(ProviderPayloadError(format!(
                "unknown decision answer name: {name}"
            ))));
        };
        validate_answer_for_question(question, answer)?;
    }
    for question in &definition.questions {
        let name = question_name(question);
        if !seen_answers.contains(name) {
            return Err(anyhow::Error::new(ProviderPayloadError(format!(
                "missing decision answer name: {name}"
            ))));
        }
    }
    Ok(())
}

fn question_name(question: &ExtractionQuestion) -> &str {
    match question {
        ExtractionQuestion::Predicate(q) => q.name.as_str(),
        ExtractionQuestion::Choice(q) => q.name.as_str(),
        ExtractionQuestion::Score(q) => q.name.as_str(),
    }
}

fn answer_name(answer: &DecisionAnswer) -> &str {
    match answer {
        DecisionAnswer::Predicate(a) => a.name.as_str(),
        DecisionAnswer::Choice(a) => a.name.as_str(),
        DecisionAnswer::Score(a) => a.name.as_str(),
        DecisionAnswer::Refusal(a) => a.name.as_str(),
    }
}

fn validate_answer_for_question(
    question: &ExtractionQuestion,
    answer: &DecisionAnswer,
) -> Result<()> {
    match (question, answer) {
        (ExtractionQuestion::Predicate(_), DecisionAnswer::Predicate(a)) => {
            require_unit_probability(a.probability, "predicate probability")?;
            Ok(())
        }
        (ExtractionQuestion::Choice(q), DecisionAnswer::Choice(a)) => validate_choice_answer(q, a),
        (ExtractionQuestion::Score(q), DecisionAnswer::Score(a)) => validate_score_answer(q, a),
        (_, DecisionAnswer::Refusal(a)) => Err(anyhow::Error::new(ProviderPayloadError(format!(
            "decision refused for question: {}",
            a.name
        )))),
        _ => Err(anyhow::Error::new(ProviderPayloadError(format!(
            "wrong decision answer type for question: {}",
            answer_name(answer)
        )))),
    }
}

fn require_unit_probability(value: f64, what: &str) -> Result<()> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(anyhow::Error::new(ProviderPayloadError(format!(
            "{what} must be finite within 0..=1"
        ))));
    }
    Ok(())
}

fn validate_choice_answer(question: &ChoiceQuestion, answer: &ChoiceAnswer) -> Result<()> {
    let declared: std::collections::HashSet<&str> = question
        .choices
        .iter()
        .map(|choice| choice.value.as_str())
        .collect();
    if !declared.contains(answer.choice.as_str()) {
        return Err(anyhow::Error::new(ProviderPayloadError(format!(
            "unknown choice value for question: {}",
            answer.name
        ))));
    }
    if answer.probabilities.len() != question.choices.len() {
        return Err(anyhow::Error::new(ProviderPayloadError(format!(
            "choice probabilities must cover every option for question: {}",
            answer.name
        ))));
    }
    let mut seen = std::collections::HashSet::new();
    let mut sum = 0.0;
    for entry in &answer.probabilities {
        if !declared.contains(entry.value.as_str()) {
            return Err(anyhow::Error::new(ProviderPayloadError(format!(
                "unknown choice probability value for question: {}",
                answer.name
            ))));
        }
        if !seen.insert(entry.value.as_str()) {
            return Err(anyhow::Error::new(ProviderPayloadError(format!(
                "duplicate choice probability value for question: {}",
                answer.name
            ))));
        }
        require_unit_probability(entry.probability, "choice probability")?;
        sum += entry.probability;
    }
    if (sum - 1.0).abs() > PROBABILITY_TOLERANCE {
        return Err(anyhow::Error::new(ProviderPayloadError(format!(
            "choice probabilities must sum to 1 for question: {}",
            answer.name
        ))));
    }
    require_unit_probability(answer.confidence, "choice confidence")?;
    Ok(())
}

fn validate_score_answer(question: &ScoreQuestion, answer: &ScoreAnswer) -> Result<()> {
    let levels = question.levels.len();
    if answer.probabilities.len() != levels {
        return Err(anyhow::Error::new(ProviderPayloadError(format!(
            "score probabilities must cover every level for question: {}",
            answer.name
        ))));
    }
    if !answer.score.is_finite()
        || answer.score < 0.0
        || answer.score > (levels.saturating_sub(1)) as f64
    {
        return Err(anyhow::Error::new(ProviderPayloadError(format!(
            "score must be finite within 0..=levels-1 for question: {}",
            answer.name
        ))));
    }
    let mut seen = std::collections::HashSet::new();
    let mut sum = 0.0;
    let mut weighted = 0.0;
    for entry in &answer.probabilities {
        if entry.value < 0 || (entry.value as usize) >= levels {
            return Err(anyhow::Error::new(ProviderPayloadError(format!(
                "score probability index out of range for question: {}",
                answer.name
            ))));
        }
        if !seen.insert(entry.value) {
            return Err(anyhow::Error::new(ProviderPayloadError(format!(
                "duplicate score probability index for question: {}",
                answer.name
            ))));
        }
        let expected_label = question.levels[entry.value as usize].label.as_str();
        if entry.label != expected_label {
            return Err(anyhow::Error::new(ProviderPayloadError(format!(
                "score probability label mismatch for question: {}",
                answer.name
            ))));
        }
        require_unit_probability(entry.probability, "score probability")?;
        sum += entry.probability;
        weighted += f64::from(entry.value) * entry.probability;
    }
    if (sum - 1.0).abs() > PROBABILITY_TOLERANCE {
        return Err(anyhow::Error::new(ProviderPayloadError(format!(
            "score probabilities must sum to 1 for question: {}",
            answer.name
        ))));
    }
    if (answer.score - weighted).abs() > PROBABILITY_TOLERANCE {
        return Err(anyhow::Error::new(ProviderPayloadError(format!(
            "score must match the weighted index average for question: {}",
            answer.name
        ))));
    }
    require_unit_probability(answer.confidence, "score confidence")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use context69_contracts_extraction::{
        ChoiceOption, ChoiceProbability, PredicateQuestion, ScoreLevel,
    };
    use serde_json::json;

    fn predicate_definition() -> DecisionDefinition {
        DecisionDefinition {
            questions: vec![ExtractionQuestion::Predicate(PredicateQuestion {
                name: "relevant".to_string(),
                instructions: "Is it relevant?".to_string(),
            })],
        }
    }

    fn choice_definition() -> DecisionDefinition {
        DecisionDefinition {
            questions: vec![ExtractionQuestion::Choice(ChoiceQuestion {
                name: "direction".to_string(),
                instructions: "Pick one.".to_string(),
                choices: vec![
                    ChoiceOption {
                        value: "up".to_string(),
                        description: "Up.".to_string(),
                    },
                    ChoiceOption {
                        value: "down".to_string(),
                        description: "Down.".to_string(),
                    },
                ],
            })],
        }
    }

    fn score_definition() -> DecisionDefinition {
        DecisionDefinition {
            questions: vec![ExtractionQuestion::Score(ScoreQuestion {
                name: "quality".to_string(),
                instructions: "Rate it.".to_string(),
                levels: vec![
                    ScoreLevel {
                        label: "bad".to_string(),
                        description: "Bad.".to_string(),
                    },
                    ScoreLevel {
                        label: "good".to_string(),
                        description: "Good.".to_string(),
                    },
                ],
            })],
        }
    }

    fn mixed_definition() -> DecisionDefinition {
        DecisionDefinition {
            questions: vec![
                ExtractionQuestion::Predicate(PredicateQuestion {
                    name: "relevant".to_string(),
                    instructions: "Is it?".to_string(),
                }),
                ExtractionQuestion::Choice(ChoiceQuestion {
                    name: "direction".to_string(),
                    instructions: "Pick.".to_string(),
                    choices: vec![
                        ChoiceOption {
                            value: "up".to_string(),
                            description: "Up.".to_string(),
                        },
                        ChoiceOption {
                            value: "down".to_string(),
                            description: "Down.".to_string(),
                        },
                    ],
                }),
                ExtractionQuestion::Score(ScoreQuestion {
                    name: "quality".to_string(),
                    instructions: "Rate.".to_string(),
                    levels: vec![
                        ScoreLevel {
                            label: "bad".to_string(),
                            description: "Bad.".to_string(),
                        },
                        ScoreLevel {
                            label: "good".to_string(),
                            description: "Good.".to_string(),
                        },
                    ],
                }),
            ],
        }
    }

    #[test]
    fn valid_predicate_choice_score_dispatch() {
        let definition = predicate_definition();
        let answers: Vec<DecisionAnswer> = serde_json::from_value(json!([
            {"type": "predicate", "name": "relevant", "probability": 0.7}
        ]))
        .expect("predicate fixture");
        assert!(validate_decision_answers(&definition, &answers).is_ok());

        let definition = choice_definition();
        let answers: Vec<DecisionAnswer> = serde_json::from_value(json!([
            {"type": "choice", "name": "direction", "choice": "up",
             "probabilities": [
               {"value": "up", "probability": 0.8},
               {"value": "down", "probability": 0.2}
             ], "confidence": 0.8}
        ]))
        .expect("choice fixture");
        assert!(validate_decision_answers(&definition, &answers).is_ok());

        let definition = score_definition();
        let answers: Vec<DecisionAnswer> = serde_json::from_value(json!([
            {"type": "score", "name": "quality", "score": 0.9,
             "probabilities": [
               {"value": 0, "label": "bad", "probability": 0.1},
               {"value": 1, "label": "good", "probability": 0.9}
             ], "confidence": 0.9}
        ]))
        .expect("score fixture");
        assert!(validate_decision_answers(&definition, &answers).is_ok());
    }

    #[test]
    fn low_confidence_is_still_success() {
        let definition = choice_definition();
        let answers: Vec<DecisionAnswer> = serde_json::from_value(json!([
            {"type": "choice", "name": "direction", "choice": "up",
             "probabilities": [
               {"value": "up", "probability": 0.51},
               {"value": "down", "probability": 0.49}
             ], "confidence": 0.01}
        ]))
        .expect("low confidence fixture");
        assert!(validate_decision_answers(&definition, &answers).is_ok());
    }

    #[test]
    fn reversed_answer_order_is_valid() {
        let definition = mixed_definition();
        let answers: Vec<DecisionAnswer> = serde_json::from_value(json!([
            {"type": "score", "name": "quality", "score": 1.0,
             "probabilities": [
               {"value": 0, "label": "bad", "probability": 0.0},
               {"value": 1, "label": "good", "probability": 1.0}
             ], "confidence": 0.9},
            {"type": "predicate", "name": "relevant", "probability": 0.6},
            {"type": "choice", "name": "direction", "choice": "down",
             "probabilities": [
               {"value": "up", "probability": 0.3},
               {"value": "down", "probability": 0.7}
             ], "confidence": 0.7}
        ]))
        .expect("reversed fixture");
        assert!(validate_decision_answers(&definition, &answers).is_ok());
    }

    #[test]
    fn refusal_incomplete_duplicate_wrong_type_fail() {
        let definition = predicate_definition();
        let refusal: Vec<DecisionAnswer> = serde_json::from_value(json!([
            {"type": "refusal", "name": "relevant"}
        ]))
        .expect("refusal fixture");
        assert!(validate_decision_answers(&definition, &refusal).is_err());

        let incomplete: Vec<DecisionAnswer> =
            serde_json::from_value(json!([])).expect("empty fixture");
        assert!(validate_decision_answers(&definition, &incomplete).is_err());

        let duplicate: Vec<DecisionAnswer> = serde_json::from_value(json!([
            {"type": "predicate", "name": "relevant", "probability": 0.5},
            {"type": "predicate", "name": "relevant", "probability": 0.6}
        ]))
        .expect("duplicate fixture");
        assert!(validate_decision_answers(&definition, &duplicate).is_err());

        let unknown: Vec<DecisionAnswer> = serde_json::from_value(json!([
            {"type": "predicate", "name": "other", "probability": 0.5}
        ]))
        .expect("unknown fixture");
        assert!(validate_decision_answers(&definition, &unknown).is_err());

        let wrong_type: Vec<DecisionAnswer> = serde_json::from_value(json!([
            {"type": "choice", "name": "relevant", "choice": "up",
             "probabilities": [
               {"value": "up", "probability": 0.5},
               {"value": "down", "probability": 0.5}
             ], "confidence": 0.5}
        ]))
        .expect("wrong type fixture");
        assert!(validate_decision_answers(&definition, &wrong_type).is_err());
    }

    #[test]
    fn out_of_range_nonfinite_invalid_distributions_fail() {
        let definition = predicate_definition();
        for probability in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
            let answers = vec![DecisionAnswer::Predicate(
                context69_contracts_extraction::PredicateAnswer {
                    name: "relevant".to_string(),
                    probability,
                },
            )];
            assert!(
                validate_decision_answers(&definition, &answers).is_err(),
                "probability {probability} must fail"
            );
        }

        let definition = choice_definition();
        let bad_sum: Vec<DecisionAnswer> = serde_json::from_value(json!([
            {"type": "choice", "name": "direction", "choice": "up",
             "probabilities": [
               {"value": "up", "probability": 0.5},
               {"value": "down", "probability": 0.4}
             ], "confidence": 0.5}
        ]))
        .expect("bad sum fixture");
        assert!(validate_decision_answers(&definition, &bad_sum).is_err());

        let duplicate_option: Vec<DecisionAnswer> = serde_json::from_value(json!([
            {"type": "choice", "name": "direction", "choice": "up",
             "probabilities": [
               {"value": "up", "probability": 0.5},
               {"value": "up", "probability": 0.5}
             ], "confidence": 0.5}
        ]))
        .expect("duplicate option fixture");
        assert!(validate_decision_answers(&definition, &duplicate_option).is_err());

        let omitted_option: Vec<DecisionAnswer> = serde_json::from_value(json!([
            {"type": "choice", "name": "direction", "choice": "up",
             "probabilities": [{"value": "up", "probability": 1.0}],
             "confidence": 0.5}
        ]))
        .expect("omitted option fixture");
        assert!(validate_decision_answers(&definition, &omitted_option).is_err());

        let unknown_choice: Vec<DecisionAnswer> = serde_json::from_value(json!([
            {"type": "choice", "name": "direction", "choice": "sideways",
             "probabilities": [
               {"value": "up", "probability": 0.5},
               {"value": "down", "probability": 0.5}
             ], "confidence": 0.5}
        ]))
        .expect("unknown choice fixture");
        assert!(validate_decision_answers(&definition, &unknown_choice).is_err());
    }

    #[test]
    fn score_index_label_mismatch_and_weighted_average_fail() {
        let definition = score_definition();
        let label_mismatch: Vec<DecisionAnswer> = serde_json::from_value(json!([
            {"type": "score", "name": "quality", "score": 1.0,
             "probabilities": [
               {"value": 0, "label": "good", "probability": 0.0},
               {"value": 1, "label": "bad", "probability": 1.0}
             ], "confidence": 0.9}
        ]))
        .expect("label mismatch fixture");
        assert!(validate_decision_answers(&definition, &label_mismatch).is_err());

        let index_out_of_range: Vec<DecisionAnswer> = serde_json::from_value(json!([
            {"type": "score", "name": "quality", "score": 5.0,
             "probabilities": [
               {"value": 0, "label": "bad", "probability": 0.0},
               {"value": 1, "label": "good", "probability": 1.0}
             ], "confidence": 0.9}
        ]))
        .expect("index range fixture");
        assert!(validate_decision_answers(&definition, &index_out_of_range).is_err());

        let weighted_mismatch: Vec<DecisionAnswer> = serde_json::from_value(json!([
            {"type": "score", "name": "quality", "score": 0.0,
             "probabilities": [
               {"value": 0, "label": "bad", "probability": 0.1},
               {"value": 1, "label": "good", "probability": 0.9}
             ], "confidence": 0.9}
        ]))
        .expect("weighted mismatch fixture");
        assert!(validate_decision_answers(&definition, &weighted_mismatch).is_err());
    }

    #[test]
    fn extra_vendor_envelope_fields_normalize_safely() {
        let wire: WireDecisionResponse = serde_json::from_value(json!({
            "answers": [
                {"type": "predicate", "name": "relevant", "probability": 0.7}
            ],
            "usage": {"input_tokens": 10, "output_tokens": 5},
            "id": "dec_123",
            "model": "gpt-6-luna"
        }))
        .expect("vendor envelope with extras");
        assert_eq!(wire.answers.len(), 1);
        assert!(validate_decision_answers(&predicate_definition(), &wire.answers).is_ok());
    }

    #[test]
    fn endpoint_normalization_appends_decisions_once() {
        assert_eq!(
            decisions_url("https://api.openai.com/v1").expect("base"),
            "https://api.openai.com/v1/decisions"
        );
        assert_eq!(
            decisions_url("https://api.openai.com/v1/decisions").expect("full"),
            "https://api.openai.com/v1/decisions"
        );
        assert_eq!(
            decisions_url("https://api.openai.com/v1/").expect("trailing slash"),
            "https://api.openai.com/v1/decisions"
        );
        assert!(decisions_url("  ").is_err());
    }

    #[test]
    fn config_requires_luna_model_and_key() {
        assert!(require_model("gpt-6-luna").is_ok());
        assert!(require_model("").is_err());
        assert!(require_model("gpt-4o").is_err());
        assert!(require_key("dummy").is_ok());
        assert!(require_key("  ").is_err());
    }

    #[test]
    fn contract_limits_hold_thirty_two_questions() {
        let mut questions = Vec::new();
        for index in 0..33 {
            questions.push(ExtractionQuestion::Predicate(PredicateQuestion {
                name: format!("q-{index}"),
                instructions: "Is it?".to_string(),
            }));
        }
        let definition = DecisionDefinition { questions };
        assert!(definition.validate().is_err());
    }
}
