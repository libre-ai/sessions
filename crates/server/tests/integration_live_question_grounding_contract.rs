use presto_core::protocol::{CitationValidationStatus, ServerMessage};
use serde_json::Value;

#[test]
fn live_question_grounding_fixture_roundtrips_without_source_leakage() {
    let fixture_json =
        include_str!("../../../docs/contracts/live-question-grounding.v0.1.fixtures.json");
    let root: Value = serde_json::from_str(fixture_json).expect("fixture JSON is valid");
    assert_eq!(root["version"], "live-question-grounding.v0.1");

    for fixture in root["fixtures"].as_array().expect("fixtures array") {
        let message = fixture["message"].clone();
        let raw_message = serde_json::to_string(&message).expect("message serializes");
        for forbidden in fixture["forbidden_public_fields"]
            .as_array()
            .expect("forbidden fields array")
        {
            let field = forbidden.as_str().expect("forbidden field string");
            assert!(
                !raw_message.contains(field),
                "public question fixture must not expose {field}: {raw_message}"
            );
        }

        let decoded: ServerMessage = serde_json::from_value(message).expect("wire message shape");
        let ServerMessage::QuestionOpened { question } = decoded else {
            panic!("fixture must be a question_opened message");
        };
        assert!(!question.grounding.source_refs_exposed);

        match fixture["id"].as_str().expect("fixture id") {
            "fixture_question_opened" => {
                assert!(question.grounding.grounded);
                assert_eq!(question.grounding.citation_count, 1);
                assert_eq!(
                    question.grounding.validation_status,
                    CitationValidationStatus::Fixture
                );
            }
            "unvalidated_host_question" => {
                assert!(!question.grounding.grounded);
                assert_eq!(question.grounding.citation_count, 0);
                assert_eq!(
                    question.grounding.validation_status,
                    CitationValidationStatus::NotValidated
                );
            }
            other => panic!("unexpected fixture id {other}"),
        }
    }
}
