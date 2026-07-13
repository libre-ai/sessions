//! Pure guest-join client state.
//!
//! The state machine is intentionally UI-agnostic and keeps the Biscuit join
//! token out of state entirely: the app may hold it in memory to open the first
//! POST/WS, but the machine only tracks session progress and server authority.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::protocol::{
    LeaderboardEntry, ParticipantId, PublicReveal, QuestionId, QuestionKind, QuestionPublic,
    ServerMessage, SessionPhasePublic, SessionSnapshot,
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
    Joined {
        participant_id: ParticipantId,
        participants_count: u32,
    },
    Snapshot {
        snapshot: SessionSnapshot,
    },
    QuestionOpened {
        question: QuestionPublic,
    },
    AnswersRevealed {
        correct_choices: Vec<u8>,
        leaderboard: Vec<LeaderboardEntry>,
        heatmap: BTreeMap<String, f32>,
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

impl GuestJoinEvent {
    pub fn from_server_message(message: ServerMessage) -> Option<Self> {
        match message {
            ServerMessage::Joined {
                participant_id,
                participants,
            } => Some(Self::Joined {
                participant_id,
                participants_count: participants,
            }),
            ServerMessage::Snapshot { snapshot } => Some(Self::Snapshot { snapshot }),
            ServerMessage::QuestionOpened { question } => Some(Self::QuestionOpened { question }),
            ServerMessage::AnswersRevealed {
                correct_choices,
                leaderboard,
                heatmap,
            } => Some(Self::AnswersRevealed {
                correct_choices,
                leaderboard,
                heatmap,
            }),
            ServerMessage::AnswerAccepted { question_id } => {
                Some(Self::AnswerAccepted { question_id })
            }
            ServerMessage::Error { reason } => Some(Self::Failed { reason }),
            ServerMessage::Pong
            | ServerMessage::AnswerReceived { .. }
            | ServerMessage::BreakoutOpened { .. }
            | ServerMessage::FlashcardsReady { .. } => None,
        }
    }
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        question: Option<QuestionPublic>,
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
                Self::Disconnected { resume } => Self::Disconnected {
                    resume: Box::new(resume.apply_event(GuestJoinEvent::NameEdited { name })),
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
                Self::Disconnected { resume } => Self::Disconnected {
                    resume: Box::new(resume.apply_event(GuestJoinEvent::JoinStarted)),
                },
                other => other,
            },
            GuestJoinEvent::JoinSucceeded {
                participant_id,
                participants_count,
            }
            | GuestJoinEvent::Joined {
                participant_id,
                participants_count,
            } => match self {
                Self::Joining {
                    session_id, name, ..
                }
                | Self::NameEntry {
                    session_id, name, ..
                } => Self::Lobby {
                    session_id,
                    participant_id,
                    name,
                    participants_count,
                },
                Self::Lobby {
                    session_id,
                    participant_id,
                    name,
                    participants_count: current,
                } => Self::Lobby {
                    session_id,
                    participant_id,
                    name,
                    participants_count: current.max(participants_count),
                },
                Self::Asking {
                    session_id,
                    participant_id,
                    name,
                    participants_count: current,
                    question,
                    selected,
                    submission,
                    answered,
                } => Self::Asking {
                    session_id,
                    participant_id,
                    name,
                    participants_count: current.max(participants_count),
                    question,
                    selected,
                    submission,
                    answered,
                },
                Self::Revealed {
                    session_id,
                    participant_id,
                    name,
                    participants_count: current,
                    question,
                    selected,
                    submission,
                    reveal,
                } => Self::Revealed {
                    session_id,
                    participant_id,
                    name,
                    participants_count: current.max(participants_count),
                    question,
                    selected,
                    submission,
                    reveal,
                },
                Self::Disconnected { resume } => Self::Disconnected {
                    resume: Box::new(resume.apply_event(GuestJoinEvent::Joined {
                        participant_id,
                        participants_count,
                    })),
                },
                other => other,
            },
            GuestJoinEvent::Snapshot { snapshot } => self.apply_snapshot(snapshot),
            GuestJoinEvent::QuestionOpened { question } => self.apply_question_opened(question),
            GuestJoinEvent::AnswersRevealed {
                correct_choices,
                leaderboard,
                heatmap,
            } => self.apply_answers_revealed(correct_choices, leaderboard, heatmap),
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
            Self::Disconnected { resume } => Self::Disconnected {
                resume: Box::new(resume.apply_snapshot(snapshot)),
            },
            Self::Lobby {
                session_id,
                participant_id,
                name,
                participants_count,
            } => match snapshot.phase {
                SessionPhasePublic::Lobby => Self::Lobby {
                    session_id,
                    participant_id,
                    name,
                    participants_count: participants_count.max(snapshot.participants_count),
                },
                SessionPhasePublic::Asking => {
                    let Some(question) = snapshot.question else {
                        return Self::Failed {
                            reason: "asking snapshot missing question".into(),
                        };
                    };
                    let submission = if snapshot.answered {
                        JoinSubmission::Accepted {
                            question_id: question.id.clone(),
                        }
                    } else {
                        JoinSubmission::Idle
                    };
                    Self::Asking {
                        session_id,
                        participant_id,
                        name,
                        participants_count: participants_count.max(snapshot.participants_count),
                        question,
                        selected: BTreeSet::new(),
                        submission,
                        answered: snapshot.answered,
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
                        participants_count: participants_count.max(snapshot.participants_count),
                        question: Some(question),
                        selected: BTreeSet::new(),
                        submission: JoinSubmission::Accepted {
                            question_id: reveal.question_id.clone(),
                        },
                        reveal,
                    }
                }
            },
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
                    participants_count: participants_count.max(snapshot.participants_count),
                    question,
                    selected,
                    submission,
                    answered,
                },
                SessionPhasePublic::Asking => {
                    let Some(snapshot_question) = snapshot.question else {
                        return Self::Failed {
                            reason: "asking snapshot missing question".into(),
                        };
                    };
                    let same_question = snapshot_question.id == question.id;
                    let submission = if snapshot.answered {
                        lock_submitted(if same_question {
                            submission
                        } else {
                            JoinSubmission::Accepted {
                                question_id: snapshot_question.id.clone(),
                            }
                        })
                    } else if same_question {
                        submission
                    } else {
                        JoinSubmission::Idle
                    };
                    Self::Asking {
                        session_id,
                        participant_id,
                        name,
                        participants_count: participants_count.max(snapshot.participants_count),
                        question: snapshot_question,
                        selected: if same_question {
                            selected
                        } else {
                            BTreeSet::new()
                        },
                        submission,
                        answered: answered || snapshot.answered,
                    }
                }
                SessionPhasePublic::Revealed => {
                    let Some(snapshot_question) = snapshot.question else {
                        return Self::Failed {
                            reason: "revealed snapshot missing question".into(),
                        };
                    };
                    let Some(reveal) = snapshot.reveal else {
                        return Self::Failed {
                            reason: "revealed snapshot missing reveal".into(),
                        };
                    };
                    let same_question = snapshot_question.id == question.id;
                    Self::Revealed {
                        session_id,
                        participant_id,
                        name,
                        participants_count: participants_count.max(snapshot.participants_count),
                        question: Some(snapshot_question),
                        selected: if same_question {
                            selected
                        } else {
                            BTreeSet::new()
                        },
                        submission: lock_submitted(submission),
                        reveal,
                    }
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
                SessionPhasePublic::Lobby | SessionPhasePublic::Asking => Self::Revealed {
                    session_id,
                    participant_id,
                    name,
                    participants_count: participants_count.max(snapshot.participants_count),
                    question,
                    selected,
                    submission,
                    reveal,
                },
                SessionPhasePublic::Revealed => {
                    let Some(snapshot_question) = snapshot.question else {
                        return Self::Failed {
                            reason: "revealed snapshot missing question".into(),
                        };
                    };
                    let Some(snapshot_reveal) = snapshot.reveal else {
                        return Self::Failed {
                            reason: "revealed snapshot missing reveal".into(),
                        };
                    };
                    let same_question = question
                        .as_ref()
                        .map(|current| current.id.as_str() == snapshot_question.id.as_str())
                        .unwrap_or(true);
                    Self::Revealed {
                        session_id,
                        participant_id,
                        name,
                        participants_count: participants_count.max(snapshot.participants_count),
                        question: Some(snapshot_question),
                        selected: if same_question {
                            selected
                        } else {
                            BTreeSet::new()
                        },
                        submission: lock_submitted(submission),
                        reveal: snapshot_reveal,
                    }
                }
            },
            other => match snapshot.phase {
                SessionPhasePublic::Lobby => other,
                SessionPhasePublic::Asking => other,
                SessionPhasePublic::Revealed => other,
            },
        }
    }

    fn apply_question_opened(self, question: QuestionPublic) -> Self {
        match self {
            Self::Lobby {
                session_id,
                participant_id,
                name,
                participants_count,
            } => Self::Asking {
                session_id,
                participant_id,
                name,
                participants_count,
                question,
                selected: BTreeSet::new(),
                submission: JoinSubmission::Idle,
                answered: false,
            },
            Self::Asking {
                session_id,
                participant_id,
                name,
                participants_count,
                question: current,
                selected,
                submission,
                answered,
            } => {
                if current.id == question.id {
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
                } else {
                    Self::Asking {
                        session_id,
                        participant_id,
                        name,
                        participants_count,
                        question,
                        selected: BTreeSet::new(),
                        submission: JoinSubmission::Idle,
                        answered: false,
                    }
                }
            }
            Self::Revealed {
                session_id,
                participant_id,
                name,
                participants_count,
                question: current_question,
                selected,
                submission,
                reveal,
            } => {
                let current_question_id = current_question.as_ref().map(|q| q.id.as_str());
                if current_question_id == Some(reveal.question_id.as_str())
                    || current_question_id.is_none()
                {
                    Self::Revealed {
                        session_id,
                        participant_id,
                        name,
                        participants_count,
                        question: Some(question),
                        selected,
                        submission,
                        reveal,
                    }
                } else {
                    Self::Asking {
                        session_id,
                        participant_id,
                        name,
                        participants_count,
                        question,
                        selected: BTreeSet::new(),
                        submission: JoinSubmission::Idle,
                        answered: false,
                    }
                }
            }
            Self::Disconnected { resume } => Self::Disconnected {
                resume: Box::new(resume.apply_question_opened(question)),
            },
            other => other,
        }
    }

    fn apply_answers_revealed(
        self,
        correct_choices: Vec<u8>,
        leaderboard: Vec<LeaderboardEntry>,
        heatmap: BTreeMap<String, f32>,
    ) -> Self {
        let reveal = PublicReveal {
            question_id: self
                .question()
                .map(|question| question.id.clone())
                .unwrap_or_else(|| "unknown".to_string()),
            correct_choices,
            leaderboard,
            heatmap,
        };
        match self {
            Self::Asking {
                session_id,
                participant_id,
                name,
                participants_count,
                question,
                selected,
                submission,
                ..
            } if question.id == reveal.question_id => Self::Revealed {
                session_id,
                participant_id,
                name,
                participants_count,
                question: Some(question),
                selected,
                submission: lock_submitted(submission),
                reveal,
            },
            Self::Revealed {
                session_id,
                participant_id,
                name,
                participants_count,
                question,
                selected,
                submission,
                reveal: current,
            } if current.question_id == reveal.question_id
                || question
                    .as_ref()
                    .map(|question| question.id.as_str() == reveal.question_id)
                    .unwrap_or(true) =>
            {
                Self::Revealed {
                    session_id,
                    participant_id,
                    name,
                    participants_count,
                    question,
                    selected,
                    submission: lock_submitted(submission),
                    reveal,
                }
            }
            Self::Lobby {
                session_id,
                participant_id,
                name,
                participants_count,
            } => Self::Revealed {
                session_id,
                participant_id,
                name,
                participants_count,
                question: None,
                selected: BTreeSet::new(),
                submission: JoinSubmission::Accepted {
                    question_id: reveal.question_id.clone(),
                },
                reveal,
            },
            Self::Disconnected { resume } => Self::Disconnected {
                resume: Box::new(resume.apply_answers_revealed(
                    reveal.correct_choices,
                    reveal.leaderboard,
                    reveal.heatmap,
                )),
            },
            other => other,
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
            Self::Disconnected { resume } => Self::Disconnected {
                resume: Box::new(resume.toggle_choice(choice)),
            },
            other => other,
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
            Self::Disconnected { resume } => Self::Disconnected {
                resume: Box::new(resume.submit_answer()),
            },
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
                || reveal.question_id == question_id
                || question
                    .as_ref()
                    .map(|question| question.id == question_id)
                    .unwrap_or(false) =>
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
            Self::Disconnected { resume } => Self::Disconnected {
                resume: Box::new(resume.answer_accepted(question_id)),
            },
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
        match self {
            Self::Asking { submission, .. } => submission.is_locked(),
            Self::Revealed { submission, .. } => submission.is_locked(),
            Self::Disconnected { resume } => resume.is_locked(),
            _ => false,
        }
    }

    pub fn selected_choices(&self) -> Vec<u8> {
        match self {
            Self::Asking { selected, .. } | Self::Revealed { selected, .. } => {
                selected.iter().copied().collect()
            }
            Self::Disconnected { resume } => resume.selected_choices(),
            _ => Vec::new(),
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
            Self::Asking { question, .. } => Some(question),
            Self::Revealed { question, .. } => question.as_ref(),
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
        assert_eq!(state.selected_choices(), vec![1]);
    }

    #[test]
    fn invalid_choice_rejects_out_of_range_answers() {
        let state =
            GuestJoinState::lobby("S", "p1", "Alice", 1).apply_snapshot(asking_snapshot(false));
        let state = state.toggle_choice(9);
        assert!(matches!(state, GuestJoinState::Failed { .. }));
    }

    #[test]
    fn joined_participant_count_never_regresses() {
        let state =
            GuestJoinState::lobby("S", "p1", "Alice", 1).apply_event(GuestJoinEvent::Joined {
                participant_id: "p2".into(),
                participants_count: 2,
            });
        let state = state.apply_event(GuestJoinEvent::Joined {
            participant_id: "p3".into(),
            participants_count: 1,
        });
        assert!(matches!(
            state,
            GuestJoinState::Lobby {
                participants_count: 2,
                ..
            }
        ));
    }

    #[test]
    fn stale_snapshots_do_not_regress_after_reveal() {
        let asking =
            GuestJoinState::lobby("S", "p1", "Alice", 1).apply_snapshot(asking_snapshot(false));
        let revealed = asking.clone().apply_snapshot(revealed_snapshot());
        assert!(matches!(revealed, GuestJoinState::Revealed { .. }));
        let stale_lobby = revealed.clone().apply_snapshot(
            SessionSnapshot::new(SessionPhasePublic::Lobby, vec![], 0, None, false, None).unwrap(),
        );
        assert!(matches!(stale_lobby, GuestJoinState::Revealed { .. }));
        let stale_asking = revealed.apply_snapshot(asking_snapshot(false));
        assert!(matches!(stale_asking, GuestJoinState::Revealed { .. }));
    }

    #[test]
    fn answer_accepted_locks_the_submission() {
        let asking =
            GuestJoinState::lobby("S", "p1", "Alice", 1).apply_snapshot(asking_snapshot(false));
        let asking = asking.toggle_choice(1).submit_answer();
        let accepted = asking.answer_accepted("q1");
        assert!(matches!(
            accepted,
            GuestJoinState::Asking {
                answered: true,
                submission: JoinSubmission::Accepted { .. },
                ..
            }
        ));
        assert!(accepted.is_locked());
    }

    #[test]
    fn legacy_answers_revealed_does_not_invent_a_fake_question() {
        let state = GuestJoinState::lobby("S", "p1", "Alice", 1).apply_event(
            GuestJoinEvent::AnswersRevealed {
                correct_choices: vec![1],
                leaderboard: vec![LeaderboardEntry {
                    participant_id: "p1".into(),
                    name: "Alice".into(),
                    score: 10,
                }],
                heatmap: BTreeMap::new(),
            },
        );
        assert!(matches!(
            state,
            GuestJoinState::Revealed { question: None, .. }
        ));
        let state = state.apply_event(GuestJoinEvent::QuestionOpened {
            question: QuestionPublic {
                id: "q1".into(),
                text: "2+2 ?".into(),
                kind: QuestionKind::Single,
                choices: vec!["3".into(), "4".into()],
                timer_sec: 30,
                grounding: Default::default(),
            },
        });
        assert!(matches!(
            state,
            GuestJoinState::Revealed {
                question: Some(_),
                ..
            }
        ));
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
