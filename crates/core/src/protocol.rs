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

/// A quiz question as the host/server knows it — including the correct answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub id: QuestionId,
    pub text: String,
    pub choices: Vec<String>,
    pub correct_choice: u8,
    /// Source sections this question is grounded in (for the confusion heatmap).
    #[serde(default)]
    pub source_section_ids: Vec<String>,
    #[serde(default = "default_timer")]
    pub timer_sec: u32,
}

impl Question {
    /// The participant-facing projection: no correct answer leaks.
    pub fn public(&self) -> QuestionPublic {
        QuestionPublic {
            id: self.id.clone(),
            text: self.text.clone(),
            choices: self.choices.clone(),
            timer_sec: self.timer_sec,
        }
    }
}

/// What participants see: a question without its answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionPublic {
    pub id: QuestionId,
    pub text: String,
    pub choices: Vec<String>,
    pub timer_sec: u32,
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
    SubmitAnswer {
        question_id: QuestionId,
        choice: u8,
        elapsed_ms: u32,
    },
    /// Host opens a question (host-only; enforced by the WS/Biscuit layer).
    PushQuestion {
        question: Question,
    },
    /// Host reveals answers and the leaderboard (host-only).
    Reveal,
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
        correct_choice: u8,
        leaderboard: Vec<LeaderboardEntry>,
        /// Per source-section confusion ratio in `[0.0, 1.0]`.
        heatmap: BTreeMap<String, f32>,
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
            choices: vec!["3".into(), "4".into(), "5".into()],
            correct_choice: 1,
            source_section_ids: vec!["doc1#s2".into()],
            timer_sec: 20,
        }
    }

    #[test]
    fn public_projection_hides_the_answer() {
        let pubq = sample_question().public();
        assert_eq!(pubq.id, "q1");
        assert_eq!(pubq.choices.len(), 3);
        assert_eq!(pubq.timer_sec, 20);
        // QuestionPublic has no `correct_choice` field at all.
        let json = serde_json::to_string(&pubq).unwrap();
        assert!(!json.contains("correct"));
    }

    #[test]
    fn client_message_roundtrips() {
        let msg = ClientMessage::SubmitAnswer {
            question_id: "q1".into(),
            choice: 1,
            elapsed_ms: 3500,
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
    fn question_uses_default_timer_when_absent() {
        let q: Question =
            serde_json::from_str(r#"{"id":"q","text":"t","choices":["a"],"correct_choice":0}"#)
                .unwrap();
        assert_eq!(q.timer_sec, 30);
        assert!(q.source_section_ids.is_empty());
    }
}
