//! The live-session wire protocol, shared by the server and all clients.
//!
//! JSON over a WebSocket, one message per frame. The server holds the full
//! [`Question`] (with the correct answer); participants only ever receive the
//! [`QuestionPublic`] projection.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub type SessionId = String;
pub type ParticipantId = String;
pub type QuestionId = String;

fn default_timer() -> u32 {
    30
}

/// Public citation-validation state for a live question.
///
/// `Verified` is only set by the RAG path after the grounding verifier accepts
/// the generated question. `Fixture` is for deterministic demo content and must
/// not be presented as product-grade provenance. `NotValidated` is the default
/// for facilitator-pushed questions that carry no server-side proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CitationValidationStatus {
    #[default]
    NotValidated,
    Fixture,
    Verified,
}

impl CitationValidationStatus {
    fn is_publicly_grounded(self) -> bool {
        matches!(self, Self::Fixture | Self::Verified)
    }
}

/// Server-side validation marker attached to a question before it is projected
/// to participants. It intentionally contains no source text and no raw verifier
/// reasoning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitationValidation {
    pub status: CitationValidationStatus,
    pub validator: String,
    pub citation_count: usize,
}

impl CitationValidation {
    pub fn fixture(citation_count: usize) -> Self {
        Self {
            status: CitationValidationStatus::Fixture,
            validator: "fixture".into(),
            citation_count,
        }
    }

    pub fn verified(citation_count: usize) -> Self {
        Self {
            status: CitationValidationStatus::Verified,
            validator: "grounding_verifier".into(),
            citation_count,
        }
    }
}

/// Participant-facing grounding summary. Source refs stay server-side; the live
/// question proves whether citation validation happened without exposing corpus
/// handles or source text in `question_opened`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PublicGrounding {
    pub grounded: bool,
    pub citation_count: usize,
    pub validation_status: CitationValidationStatus,
    pub source_refs_exposed: bool,
}

/// How a question is answered: one correct choice (radio) or several (checkboxes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionKind {
    #[default]
    Single,
    Multi,
}

/// A quiz question as the host/server knows it — including the correct answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub id: QuestionId,
    pub text: String,
    #[serde(default)]
    pub kind: QuestionKind,
    pub choices: Vec<String>,
    /// The correct choice indices: exactly one for `Single`, one or more for `Multi`.
    pub correct_choices: Vec<u8>,
    /// Source sections this question is grounded in (server-side only: scoring,
    /// heatmap, and host-mediated breakouts). They are not included in
    /// [`QuestionPublic`].
    #[serde(default)]
    pub source_section_ids: Vec<String>,
    /// Server-side citation validation. Missing validation keeps the public
    /// projection honest (`grounded=false`) even when a host-supplied question
    /// includes source section ids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation_validation: Option<CitationValidation>,
    #[serde(default = "default_timer")]
    pub timer_sec: u32,
}

impl Question {
    /// The participant-facing projection: no correct answer, source text, or raw
    /// source-section ids leak.
    pub fn public(&self) -> QuestionPublic {
        let validation_status = self
            .citation_validation
            .as_ref()
            .map(|validation| validation.status)
            .unwrap_or_default();
        let citation_count = self
            .citation_validation
            .as_ref()
            .map(|validation| validation.citation_count)
            .unwrap_or_default()
            .min(self.source_section_ids.len());
        let grounded = validation_status.is_publicly_grounded() && citation_count > 0;
        QuestionPublic {
            id: self.id.clone(),
            text: self.text.clone(),
            kind: self.kind,
            choices: self.choices.clone(),
            timer_sec: self.timer_sec,
            grounding: PublicGrounding {
                grounded,
                citation_count: if grounded { citation_count } else { 0 },
                validation_status,
                source_refs_exposed: false,
            },
        }
    }
}

/// What participants see: a question without its answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionPublic {
    pub id: QuestionId,
    pub text: String,
    pub kind: QuestionKind,
    pub choices: Vec<String>,
    pub timer_sec: u32,
    #[serde(default)]
    pub grounding: PublicGrounding,
}

/// A spaced-repetition flashcard generated from a confused source section,
/// carrying an initial SM-2 state for an external SRS scheduler.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Flashcard {
    pub section_id: String,
    pub front: String,
    pub back: String,
    pub ease_factor: f32,
    pub interval_days: u32,
}

/// One row of the live leaderboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaderboardEntry {
    pub participant_id: ParticipantId,
    pub name: String,
    pub score: u32,
}

/// Messages a client sends to the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Join {
        name: String,
    },
    /// Answer the open question with the selected choice indices (one for a
    /// single-choice question, several for multi-select). `question_id` must
    /// match the exact open question; the server times the answer — clients do
    /// not supply elapsed time.
    SubmitAnswer {
        question_id: QuestionId,
        choices: Vec<u8>,
    },
    /// Host opens a question (host-only; enforced by the WS/Biscuit layer).
    PushQuestion {
        question: Question,
    },
    /// Host opens a question generated from the corpus for `query`
    /// (retrieve → generate → verify; host-only).
    GenerateQuestion {
        query: String,
    },
    /// Host reveals answers and the leaderboard (host-only).
    Reveal,
    /// Host opens a grounded clarification for a confused source section
    /// (retrieve → clarify; host-only).
    Breakout {
        section_id: String,
    },
    /// The requester asks for their own post-session spaced-repetition deck,
    /// generated from the sections they struggled with.
    Flashcards,
    Ping,
}

/// Messages the server broadcasts/sends to clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Joined {
        participant_id: ParticipantId,
        participants: u32,
    },
    QuestionOpened {
        question: QuestionPublic,
    },
    AnswerReceived {
        participant_id: ParticipantId,
    },
    AnswersRevealed {
        correct_choices: Vec<u8>,
        leaderboard: Vec<LeaderboardEntry>,
        /// Per source-section confusion ratio in `[0.0, 1.0]`.
        heatmap: BTreeMap<String, f32>,
    },
    /// A grounded clarification of a confused source section (the breakout).
    BreakoutOpened {
        section_id: String,
        explanation: String,
    },
    /// The requester's spaced-repetition deck for their weak sections.
    FlashcardsReady {
        cards: Vec<Flashcard>,
    },
    Error {
        reason: String,
    },
    Pong,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_question() -> Question {
        Question {
            id: "q1".into(),
            text: "2 + 2 ?".into(),
            kind: QuestionKind::Single,
            choices: vec!["3".into(), "4".into(), "5".into()],
            correct_choices: vec![1],
            source_section_ids: vec!["doc1#s2".into()],
            citation_validation: Some(CitationValidation::verified(1)),
            timer_sec: 20,
        }
    }

    #[test]
    fn public_projection_hides_the_answer() {
        let pubq = sample_question().public();
        assert_eq!(pubq.id, "q1");
        assert_eq!(pubq.choices.len(), 3);
        assert_eq!(pubq.timer_sec, 20);
        assert!(pubq.grounding.grounded);
        assert_eq!(pubq.grounding.citation_count, 1);
        assert_eq!(
            pubq.grounding.validation_status,
            CitationValidationStatus::Verified
        );
        assert!(!pubq.grounding.source_refs_exposed);
        // QuestionPublic has no `correct_choice` or raw source-section field at all.
        let json = serde_json::to_string(&pubq).unwrap();
        assert!(!json.contains("correct"));
        assert!(!json.contains("source_section_ids"));
    }

    #[test]
    fn client_message_roundtrips() {
        let msg = ClientMessage::SubmitAnswer {
            question_id: "q1".into(),
            choices: vec![1],
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"submit_answer\""));
        let back: ClientMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn server_message_roundtrips() {
        let msg = ServerMessage::QuestionOpened {
            question: sample_question().public(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"question_opened\""));
        let back: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn question_uses_safe_defaults_when_optional_grounding_is_absent() {
        let q: Question =
            serde_json::from_str(r#"{"id":"q","text":"t","choices":["a"],"correct_choices":[0]}"#)
                .unwrap();
        assert_eq!(q.timer_sec, 30);
        assert_eq!(q.kind, QuestionKind::Single);
        assert!(q.source_section_ids.is_empty());
        assert!(q.citation_validation.is_none());
        assert!(!q.public().grounding.grounded);
    }

    #[test]
    fn source_refs_without_validation_do_not_project_as_grounded() {
        let mut q = sample_question();
        q.citation_validation = None;
        let public = q.public();
        assert!(!public.grounding.grounded);
        assert_eq!(public.grounding.citation_count, 0);
        assert_eq!(
            public.grounding.validation_status,
            CitationValidationStatus::NotValidated
        );
    }
}
