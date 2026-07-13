//! Pure guest-join client state.
//!
//! The state machine is intentionally UI-agnostic and keeps the Biscuit join
//! token out of state entirely: the app may hold it in memory to open the first
//! POST/WS, but the machine only tracks session progress and server authority.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::protocol::{
    ParticipantId, PublicReveal, QuestionId, QuestionKind, QuestionPublic, SessionPhasePublic,
    SessionSnapshot,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GuestJoinEvent {
    ReadLink {
        session_id: String,
        token_present: bool,
    },
    LinkInvalid {
        reason: String,
    },
    NameEdited {
        name: String,
    },
    JoinStarted,
    JoinSucceeded {
        participant_id: ParticipantId,
        participants_count: u32,
    },
    Snapshot {
        snapshot: SessionSnapshot,
    },
    ToggleChoice {
        choice: u8,
    },
    SubmitAnswer,
    AnswerAccepted {
        question_id: QuestionId,
    },
    Disconnected,
    Reconnected,
    Expired {
        reason: String,
    },
    Failed {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "submission", rename_all = "snake_case")]
pub enum JoinSubmission {
    Idle,
    Pending {
        question_id: QuestionId,
    },
    Accepted {
        question_id: QuestionId,
    },
    Rejected {
        question_id: QuestionId,
        reason: String,
    },
}

impl JoinSubmission {
    fn is_locked(&self) -> bool {
        !matches!(self, Self::Idle)
    }

    fn question_id(&self) -> Option<&str> {
        match self {
            Self::Idle => None,
            Self::Pending { question_id }
            | Self::Accepted { question_id }
            | Self::Rejected { question_id, .. } => Some(question_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GuestJoinState {
    ReadingLink {
        session_id: String,
        token_present: bool,
    },
    Invalid {
        reason: String,
    },
    NameEntry {
        session_id: String,
        token_present: bool,
        name: String,
    },
    Joining {
        session_id: String,
        token_present: bool,
        name: String,
    },
    Lobby {
        session_id: String,
        participant_id: ParticipantId,
        name: String,
        participants_count: u32,
    },
    Asking {
        session_id: String,
        participant_id: ParticipantId,
        name: String,
        participants_count: u32,
        question: QuestionPublic,
        selected: BTreeSet<u8>,
        submission: JoinSubmission,
        answered: bool,
    },
    Revealed {
        session_id: String,
        participant_id: ParticipantId,
        name: String,
        participants_count: u32,
        question: QuestionPublic,
        selected: BTreeSet<u8>,
        submission: JoinSubmission,
        reveal: PublicReveal,
    },
    Disconnected {
        resume: Box<GuestJoinState>,
    },
    Expired {
        reason: String,
    },
    Failed {
        reason: String,
    },
}

impl GuestJoinState {
    pub fn reading_link(session_id: impl Into<String>, token_present: bool) -> Self {
        Self::ReadingLink {
            session_id: session_id.into(),
            token_present,
        }
    }

    pub fn invalid(reason: impl Into<String>) -> Self {
        Self::Invalid {
            reason: reason.into(),
        }
    }

    pub fn name_entry(
        session_id: impl Into<String>,
        token_present: bool,
        name: impl Into<String>,
    ) -> Self {
        Self::NameEntry {
            session_id: session_id.into(),
            token_present,
            name: name.into(),
        }
    }

    pub fn joining(
        session_id: impl Into<String>,
        token_present: bool,
        name: impl Into<String>,
    ) -> Self {
        Self::Joining {
            session_id: session_id.into(),
            token_present,
            name: name.into(),
        }
    }

    pub fn lobby(
        session_id: impl Into<String>,
        participant_id: impl Into<String>,
        name: impl Into<String>,
        participants_count: u32,
    ) -> Self {
        Self::Lobby {
            session_id: session_id.into(),
            participant_id: participant_id.into(),
            name: name.into(),
            participants_count,
        }
    }

    pub fn apply_event(self, event: GuestJoinEvent) -> Self {
        match event {
            GuestJoinEvent::ReadLink {
                session_id,
                token_present,
            } => Self::reading_link(session_id, token_present),
            GuestJoinEvent::LinkInvalid { reason } => Self::invalid(reason),
            GuestJoinEvent::NameEdited { name } => match self {
                Self::ReadingLink {
                    session_id,
                    token_present,
                }
                | Self::NameEntry {
                    session_id,
                    token_present,
                    ..
                } => Self::NameEntry {
                    session_id,
                    token_present,
                    name,
                },
                Self::Joining {
                    session_id,
                    token_present,
                    ..
                } => Self::Joining {
                    session_id,
                    token_present,
                    name,
                },
                other => other,
            },
            GuestJoinEvent::JoinStarted => match self {
                Self::NameEntry {
                    session_id,
                    token_present,
                    name,
                }
                | Self::Joining {
                    session_id,
                    token_present,
                    name,
                } => Self::Joining {
                    session_id,
                    token_present,
                    name,
                },
                other => other,
            },
            GuestJoinEvent::JoinSucceeded {
                participant_id,
                participants_count,
            } => match self {
                Self::Joining {
                    session_id, name, ..
                }
                | Self::NameEntry {
                    session_id, name, ..
                } => Self::lobby(session_id, participant_id, name, participants_count),
                other => other,
            },
            GuestJoinEvent::Snapshot { snapshot } => self.apply_snapshot(snapshot),
            GuestJoinEvent::ToggleChoice { choice } => self.toggle_choice(choice),
            GuestJoinEvent::SubmitAnswer => self.submit_answer(),
            GuestJoinEvent::AnswerAccepted { question_id } => self.answer_accepted(question_id),
            GuestJoinEvent::Disconnected => Self::Disconnected {
                resume: Box::new(self),
            },
            GuestJoinEvent::Reconnected => match self {
                Self::Disconnected { resume } => *resume,
                other => other,
            },
            GuestJoinEvent::Expired { reason } => Self::Expired { reason },
            GuestJoinEvent::Failed { reason } => Self::Failed { reason },
        }
    }

    pub fn apply_snapshot(self, snapshot: SessionSnapshot) -> Self {
        match self {
            Self::Lobby {
                session_id,
                participant_id,
                name,
                participants_count,
            } => Self::project_snapshot(
                session_id,
                participant_id,
                name,
                participants_count,
                snapshot,
                BTreeSet::new(),
                JoinSubmission::Idle,
            ),
            Self::Asking {
                session_id,
                participant_id,
                name,
                participants_count,
                question,
                selected,
                submission,
                answered,
            } => match snapshot.phase {
                SessionPhasePublic::Lobby => Self::Asking {
                    session_id,
                    participant_id,
                    name,
                    participants_count,
                    question,
                    selected,
                    submission,
                    answered,
                },
                SessionPhasePublic::Asking | SessionPhasePublic::Revealed => {
                    Self::project_snapshot(
                        session_id,
                        participant_id,
                        name,
                        participants_count,
                        snapshot,
                        BTreeSet::new(),
                        JoinSubmission::Idle,
                    )
                }
            },
            Self::Revealed {
                session_id,
                participant_id,
                name,
                participants_count,
                question,
                selected,
                submission,
                reveal,
            } => match snapshot.phase {
                SessionPhasePublic::Revealed => Self::project_snapshot(
                    session_id,
                    participant_id,
                    name,
                    participants_count,
                    snapshot,
                    BTreeSet::new(),
                    JoinSubmission::Idle,
                ),
                SessionPhasePublic::Lobby | SessionPhasePublic::Asking => Self::Revealed {
                    session_id,
                    participant_id,
                    name,
                    participants_count,
                    question,
                    selected,
                    submission,
                    reveal,
                },
            },
            Self::Disconnected { resume } => Self::Disconnected {
                resume: Box::new(resume.apply_snapshot(snapshot)),
            },
            other => match snapshot.phase {
                SessionPhasePublic::Lobby => other,
                SessionPhasePublic::Asking => other,
                SessionPhasePublic::Revealed => other,
            },
        }
    }

    fn project_snapshot(
        session_id: String,
        participant_id: ParticipantId,
        name: String,
        participants_count: u32,
        snapshot: SessionSnapshot,
        selected: BTreeSet<u8>,
        submission: JoinSubmission,
    ) -> Self {
        match snapshot.phase {
            SessionPhasePublic::Lobby => Self::Lobby {
                session_id,
                participant_id,
                name,
                participants_count: snapshot.participants_count.max(participants_count),
            },
            SessionPhasePublic::Asking => {
                let Some(question) = snapshot.question else {
                    return Self::Failed {
                        reason: "asking snapshot missing question".into(),
                    };
                };
                let answered = snapshot.answered || submission.is_locked();
                let (selected, submission) = if matches!(
                    submission,
                    JoinSubmission::Pending { .. } | JoinSubmission::Accepted { .. }
                ) {
                    (selected, submission)
                } else {
                    (BTreeSet::new(), JoinSubmission::Idle)
                };
                Self::Asking {
                    session_id,
                    participant_id,
                    name,
                    participants_count: snapshot.participants_count.max(participants_count),
                    question,
                    selected,
                    submission: if answered {
                        lock_submitted(submission)
                    } else {
                        submission
                    },
                    answered,
                }
            }
            SessionPhasePublic::Revealed => {
                let Some(question) = snapshot.question else {
                    return Self::Failed {
                        reason: "revealed snapshot missing question".into(),
                    };
                };
                let Some(reveal) = snapshot.reveal else {
                    return Self::Failed {
                        reason: "revealed snapshot missing reveal".into(),
                    };
                };
                Self::Revealed {
                    session_id,
                    participant_id,
                    name,
                    participants_count: snapshot.participants_count.max(participants_count),
                    question,
                    selected,
                    submission: lock_submitted(submission),
                    reveal,
                }
            }
        }
    }

    pub fn toggle_choice(self, choice: u8) -> Self {
        match self {
            Self::Asking {
                session_id,
                participant_id,
                name,
                participants_count,
                question,
                mut selected,
                submission,
                answered,
            } if !submission.is_locked() && !answered => {
                if usize::from(choice) >= question.choices.len() {
                    return Self::Failed {
                        reason: "choice out of range".into(),
                    };
                }
                match question.kind {
                    QuestionKind::Single => {
                        selected.clear();
                        selected.insert(choice);
                    }
                    QuestionKind::Multi => {
                        if !selected.insert(choice) {
                            selected.remove(&choice);
                        }
                    }
                }
                Self::Asking {
                    session_id,
                    participant_id,
                    name,
                    participants_count,
                    question,
                    selected,
                    submission,
                    answered,
                }
            }
            Self::Revealed { .. }
            | Self::Lobby { .. }
            | Self::ReadingLink { .. }
            | Self::Invalid { .. }
            | Self::NameEntry { .. }
            | Self::Joining { .. }
            | Self::Disconnected { .. }
            | Self::Expired { .. }
            | Self::Failed { .. }
            | Self::Asking { .. } => self,
        }
    }

    pub fn submit_answer(self) -> Self {
        match self {
            Self::Asking {
                session_id,
                participant_id,
                name,
                participants_count,
                question,
                selected,
                submission,
                answered,
            } if !submission.is_locked() && !answered => {
                if !selected_choices_are_valid(&question, &selected) {
                    return Self::Failed {
                        reason: "answer selection invalid".into(),
                    };
                }
                Self::Asking {
                    session_id,
                    participant_id,
                    name,
                    participants_count,
                    question: question.clone(),
                    selected,
                    submission: JoinSubmission::Pending {
                        question_id: question.id,
                    },
                    answered: false,
                }
            }
            other => other,
        }
    }

    pub fn answer_accepted(self, question_id: impl Into<QuestionId>) -> Self {
        let question_id = question_id.into();
        match self {
            Self::Asking {
                session_id,
                participant_id,
                name,
                participants_count,
                question,
                selected,
                submission,
                answered: _,
            } if submission.question_id() == Some(question_id.as_str())
                || question.id == question_id =>
            {
                Self::Asking {
                    session_id,
                    participant_id,
                    name,
                    participants_count,
                    question: question.clone(),
                    selected,
                    submission: JoinSubmission::Accepted { question_id },
                    answered: true,
                }
            }
            Self::Revealed {
                session_id,
                participant_id,
                name,
                participants_count,
                question,
                selected,
                submission,
                reveal,
            } if submission.question_id() == Some(question_id.as_str())
                || reveal.question_id == question_id =>
            {
                Self::Revealed {
                    session_id,
                    participant_id,
                    name,
                    participants_count,
                    question,
                    selected,
                    submission: JoinSubmission::Accepted { question_id },
                    reveal,
                }
            }
            other => other,
        }
    }

    pub fn disconnected(self) -> Self {
        Self::Disconnected {
            resume: Box::new(self),
        }
    }

    pub fn reconnected(self) -> Self {
        match self {
            Self::Disconnected { resume } => *resume,
            other => other,
        }
    }

    pub fn expire(self, reason: impl Into<String>) -> Self {
        Self::Expired {
            reason: reason.into(),
        }
    }

    pub fn fail(self, reason: impl Into<String>) -> Self {
        Self::Failed {
            reason: reason.into(),
        }
    }

    pub fn is_locked(&self) -> bool {
        matches!(
            self,
            Self::Asking {
                submission,
                answered: true,
                ..
            } if submission.is_locked()
        ) || matches!(
            self,
            Self::Revealed {
                submission,
                ..
            } if submission.is_locked()
        )
    }

    pub fn selected_choices(&self) -> &[u8] {
        match self {
            Self::Asking { .. } => &[],
            Self::Revealed { .. } => &[],
            _ => &[],
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Self::ReadingLink { .. }
            | Self::Invalid { .. }
            | Self::Expired { .. }
            | Self::Failed { .. } => None,
            Self::NameEntry { name, .. }
            | Self::Joining { name, .. }
            | Self::Lobby { name, .. }
            | Self::Asking { name, .. }
            | Self::Revealed { name, .. } => Some(name),
            Self::Disconnected { resume } => resume.name(),
        }
    }

    pub fn question(&self) -> Option<&QuestionPublic> {
        match self {
            Self::Asking { question, .. } | Self::Revealed { question, .. } => Some(question),
            Self::Disconnected { resume } => resume.question(),
            _ => None,
        }
    }
}

fn lock_submitted(submission: JoinSubmission) -> JoinSubmission {
    match submission {
        JoinSubmission::Pending { question_id }
        | JoinSubmission::Accepted { question_id }
        | JoinSubmission::Rejected { question_id, .. } => JoinSubmission::Accepted { question_id },
        JoinSubmission::Idle => JoinSubmission::Idle,
    }
}

fn selected_choices_are_valid(question: &QuestionPublic, selected: &BTreeSet<u8>) -> bool {
    (match question.kind {
        QuestionKind::Single => selected.len() == 1,
        QuestionKind::Multi => !selected.is_empty(),
    }) && selected
        .iter()
        .copied()
        .all(|choice| usize::from(choice) < question.choices.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{LeaderboardEntry, PublicReveal};

    fn asking_snapshot(answered: bool) -> SessionSnapshot {
        SessionSnapshot::new(
            SessionPhasePublic::Asking,
            vec![],
            0,
            Some(QuestionPublic {
                id: "q1".into(),
                text: "2+2 ?".into(),
                kind: QuestionKind::Single,
                choices: vec!["3".into(), "4".into()],
                timer_sec: 30,
                grounding: Default::default(),
            }),
            answered,
            None,
        )
        .unwrap()
    }

    fn revealed_snapshot() -> SessionSnapshot {
        SessionSnapshot::new(
            SessionPhasePublic::Revealed,
            vec![],
            0,
            Some(QuestionPublic {
                id: "q1".into(),
                text: "2+2 ?".into(),
                kind: QuestionKind::Single,
                choices: vec!["3".into(), "4".into()],
                timer_sec: 30,
                grounding: Default::default(),
            }),
            true,
            Some(PublicReveal {
                question_id: "q1".into(),
                correct_choices: vec![1],
                leaderboard: vec![LeaderboardEntry {
                    participant_id: "p1".into(),
                    name: "Alice".into(),
                    score: 10,
                }],
                heatmap: Default::default(),
            }),
        )
        .unwrap()
    }

    #[test]
    fn selection_is_unique_and_locked_after_submit() {
        let state =
            GuestJoinState::lobby("S", "p1", "Alice", 1).apply_snapshot(asking_snapshot(false));
        let state = state.toggle_choice(1);
        let state = state.submit_answer();
        assert!(matches!(
            state,
            GuestJoinState::Asking {
                submission: JoinSubmission::Pending { .. },
                answered: false,
                ..
            }
        ));
        assert!(state.toggle_choice(0).question().is_some());
    }

    #[test]
    fn invalid_choice_rejects_out_of_range_answers() {
        let state =
            GuestJoinState::lobby("S", "p1", "Alice", 1).apply_snapshot(asking_snapshot(false));
        let state = state.toggle_choice(9);
        assert!(matches!(state, GuestJoinState::Failed { .. }));
    }

    #[test]
    fn snapshot_does_not_regress_and_reveal_is_authority_for_correctness() {
        let asking =
            GuestJoinState::lobby("S", "p1", "Alice", 1).apply_snapshot(asking_snapshot(false));
        let accepted = asking.clone().answer_accepted("q1");
        assert!(accepted.is_locked());
        let revealed = accepted.apply_snapshot(revealed_snapshot());
        assert!(matches!(revealed, GuestJoinState::Revealed { .. }));
        let regress = revealed.apply_snapshot(asking_snapshot(false));
        assert!(
            matches!(regress, GuestJoinState::Revealed { .. }),
            "snapshot must not regress"
        );
    }

    #[test]
    fn invalid_question_payloads_are_refused() {
        assert!(
            serde_json::from_str::<SessionSnapshot>(
                r#"{"phase":"asking","participants":[],"participants_count":0,"answered":false}"#
            )
            .is_err()
        );
    }

    #[test]
    fn disconnected_state_roundtrips_resume() {
        let state = GuestJoinState::reading_link("ABCDEF", true)
            .apply_event(GuestJoinEvent::NameEdited { name: "Ada".into() });
        let disconnected = state.clone().disconnected();
        assert!(matches!(disconnected, GuestJoinState::Disconnected { .. }));
        let restored = disconnected.reconnected();
        assert_eq!(restored, state);
    }
}
