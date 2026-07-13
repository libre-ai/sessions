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

/// Upper bound on the public participant roster shipped in a reconnect snapshot.
pub const MAX_SESSION_SNAPSHOT_PARTICIPANTS: usize = 32;

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

/// A public participant row embedded in a reconnect snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParticipantPublic {
    pub participant_id: ParticipantId,
    pub name: String,
}

/// The session phase projected to public reconnect snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhasePublic {
    Lobby,
    Asking,
    Revealed,
}

/// The public reveal payload (no private source state).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublicReveal {
    pub question_id: QuestionId,
    pub correct_choices: Vec<u8>,
    pub leaderboard: Vec<LeaderboardEntry>,
    pub heatmap: BTreeMap<String, f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "SessionSnapshotRepr")]
pub struct SessionSnapshot {
    pub phase: SessionPhasePublic,
    pub participants: Vec<ParticipantPublic>,
    pub participants_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<QuestionPublic>,
    pub answered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reveal: Option<PublicReveal>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionSnapshotRepr {
    phase: SessionPhasePublic,
    participants: Vec<ParticipantPublic>,
    participants_count: u32,
    #[serde(default)]
    question: Option<QuestionPublic>,
    answered: bool,
    #[serde(default)]
    reveal: Option<PublicReveal>,
}

impl TryFrom<SessionSnapshotRepr> for SessionSnapshot {
    type Error = String;

    fn try_from(value: SessionSnapshotRepr) -> Result<Self, Self::Error> {
        if value.participants.len() > MAX_SESSION_SNAPSHOT_PARTICIPANTS {
            return Err("participants roster exceeds public snapshot limit".into());
        }
        if value.participants_count < value.participants.len() as u32 {
            return Err("participants_count is smaller than the public roster".into());
        }
        match value.phase {
            SessionPhasePublic::Lobby => {
                if value.question.is_some() || value.reveal.is_some() || value.answered {
                    return Err("lobby snapshot must stay empty and unanswered".into());
                }
            }
            SessionPhasePublic::Asking => {
                if value.question.is_none() || value.reveal.is_some() {
                    return Err("asking snapshot must expose a question and hide reveal".into());
                }
            }
            SessionPhasePublic::Revealed => {
                if value.question.is_none() || value.reveal.is_none() {
                    return Err("revealed snapshot must include question and reveal".into());
                }
            }
        }
        Ok(Self {
            phase: value.phase,
            participants: value.participants,
            participants_count: value.participants_count,
            question: value.question,
            answered: value.answered,
            reveal: value.reveal,
        })
    }
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
    Snapshot {
        snapshot: SessionSnapshot,
    },
    QuestionOpened {
        question: QuestionPublic,
    },
    AnswerReceived {
        participant_id: ParticipantId,
    },
    AnswerAccepted {
        question_id: QuestionId,
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

    fn sample_snapshot(phase: SessionPhasePublic) -> SessionSnapshot {
        let participants = vec![ParticipantPublic {
            participant_id: "p1".into(),
            name: "Alice".into(),
        }];
        let question = Some(sample_question().public());
        let reveal = Some(PublicReveal {
            question_id: "q1".into(),
            correct_choices: vec![1],
            leaderboard: vec![LeaderboardEntry {
                participant_id: "p1".into(),
                name: "Alice".into(),
                score: 600,
            }],
            heatmap: BTreeMap::from([(String::from("doc1#s2"), 0.0)]),
        });
        match phase {
            SessionPhasePublic::Lobby => SessionSnapshot {
                phase,
                participants,
                participants_count: 1,
                question: None,
                answered: false,
                reveal: None,
            },
            SessionPhasePublic::Asking => SessionSnapshot {
                phase,
                participants,
                participants_count: 1,
                question,
                answered: false,
                reveal: None,
            },
            SessionPhasePublic::Revealed => SessionSnapshot {
                phase,
                participants,
                participants_count: 1,
                question,
                answered: true,
                reveal,
            },
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

        let accepted = ServerMessage::AnswerAccepted {
            question_id: "q1".into(),
        };
        let json = serde_json::to_string(&accepted).unwrap();
        assert!(json.contains("\"type\":\"answer_accepted\""));
        let back: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, accepted);

        let snapshot = ServerMessage::Snapshot {
            snapshot: sample_snapshot(SessionPhasePublic::Asking),
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("\"type\":\"snapshot\""));
        let back: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snapshot);
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

    #[test]
    fn snapshot_secret_data_is_phase_gated() {
        let lobby = serde_json::to_string(&sample_snapshot(SessionPhasePublic::Lobby)).unwrap();
        let asking = serde_json::to_string(&sample_snapshot(SessionPhasePublic::Asking)).unwrap();
        let revealed =
            serde_json::to_string(&sample_snapshot(SessionPhasePublic::Revealed)).unwrap();

        assert!(!lobby.contains("correct_choices"));
        assert!(!asking.contains("correct_choices"));
        assert!(revealed.contains("correct_choices"));
        assert!(!revealed.contains("answer_accepted"));
    }

    #[test]
    fn snapshot_invariants_are_rejected_on_deserialize() {
        assert!(serde_json::from_str::<SessionSnapshot>(
            r#"{"phase":"lobby","participants":[],"participants_count":0,"answered":false,"reveal":{"question_id":"q1","correct_choices":[],"leaderboard":[],"heatmap":{}}}"#
        )
        .is_err());
        assert!(serde_json::from_str::<SessionSnapshot>(
            r#"{"phase":"asking","participants":[],"participants_count":0,"question":{"id":"q1","text":"t","kind":"single","choices":["a"],"timer_sec":30,"grounding":{"grounded":false,"citation_count":0,"validation_status":"not_validated","source_refs_exposed":false}},"answered":false,"reveal":{"question_id":"q1","correct_choices":[],"leaderboard":[],"heatmap":{}}}"#
        )
        .is_err());
    }
}
