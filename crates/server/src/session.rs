//! The authoritative live-session engine — a pure state machine (no async, no
//! WebSocket, no auth). The WS layer and the trait seams (SessionStore, Fanout,
//! RateLimiter) wrap this in later slices; keeping the rules pure makes them
//! exhaustively unit-testable.

use std::collections::BTreeMap;

use presto_core::protocol::{LeaderboardEntry, ParticipantId, Question};

/// Where a session is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Lobby,
    Asking,
    Revealed,
}

#[derive(Debug, Clone)]
pub struct Participant {
    pub name: String,
    pub score: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Answer {
    pub choice: u8,
    pub elapsed_ms: u32,
}

/// Why an action was rejected. Host-vs-participant authorization is enforced by
/// the WS/Biscuit layer, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionError {
    NotAsking,
    AlreadyAnswered,
    UnknownParticipant,
    NoQuestion,
}

/// The outcome of a reveal: the correct choice, the sorted leaderboard, and a
/// per-source-section confusion heatmap.
#[derive(Debug, Clone, PartialEq)]
pub struct RevealResult {
    pub correct_choice: u8,
    pub leaderboard: Vec<LeaderboardEntry>,
    pub heatmap: BTreeMap<String, f32>,
}

/// One live quiz session, authoritative in memory.
#[derive(Debug)]
pub struct Session {
    pub id: String,
    pub host_id: String,
    pub phase: Phase,
    pub current: Option<Question>,
    pub participants: BTreeMap<ParticipantId, Participant>,
    pub answers: BTreeMap<ParticipantId, Answer>,
}

impl Session {
    pub fn new(id: impl Into<String>, host_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            host_id: host_id.into(),
            phase: Phase::Lobby,
            current: None,
            participants: BTreeMap::new(),
            answers: BTreeMap::new(),
        }
    }

    /// Join (or rejoin — idempotent, preserving score). Returns the participant count.
    pub fn join(&mut self, participant_id: impl Into<String>, name: impl Into<String>) -> u32 {
        let name = name.into();
        self.participants
            .entry(participant_id.into())
            .or_insert(Participant { name, score: 0 });
        self.participants.len() as u32
    }

    /// Open a new question: clears prior answers and enters `Asking`.
    pub fn push_question(&mut self, question: Question) {
        self.current = Some(question);
        self.answers.clear();
        self.phase = Phase::Asking;
    }

    /// Record a participant's answer (once, while `Asking`).
    pub fn submit_answer(
        &mut self,
        participant_id: &str,
        choice: u8,
        elapsed_ms: u32,
    ) -> Result<(), SessionError> {
        if self.phase != Phase::Asking {
            return Err(SessionError::NotAsking);
        }
        if !self.participants.contains_key(participant_id) {
            return Err(SessionError::UnknownParticipant);
        }
        if self.answers.contains_key(participant_id) {
            return Err(SessionError::AlreadyAnswered);
        }
        self.answers
            .insert(participant_id.to_string(), Answer { choice, elapsed_ms });
        Ok(())
    }

    /// Score the round, build the leaderboard + heatmap, enter `Revealed`.
    pub fn reveal(&mut self) -> Result<RevealResult, SessionError> {
        // Extract what we need from the question before mutating participants,
        // so the immutable borrow of `self.current` is released first.
        let (correct, sections) = {
            let q = self.current.as_ref().ok_or(SessionError::NoQuestion)?;
            (q.correct_choice, q.source_section_ids.clone())
        };

        for (pid, answer) in &self.answers {
            if answer.choice == correct
                && let Some(p) = self.participants.get_mut(pid)
            {
                p.score += score(true, answer.elapsed_ms);
            }
        }

        let mut leaderboard: Vec<LeaderboardEntry> = self
            .participants
            .iter()
            .map(|(pid, p)| LeaderboardEntry {
                participant_id: pid.clone(),
                name: p.name.clone(),
                score: p.score,
            })
            .collect();
        // Highest score first; ties broken by id for determinism.
        leaderboard.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.participant_id.cmp(&b.participant_id))
        });

        let answered = self.answers.len() as f32;
        let confusion = if answered > 0.0 {
            self.answers
                .values()
                .filter(|a| a.choice != correct)
                .count() as f32
                / answered
        } else {
            0.0
        };
        let heatmap = sections.into_iter().map(|s| (s, confusion)).collect();

        self.phase = Phase::Revealed;
        Ok(RevealResult {
            correct_choice: correct,
            leaderboard,
            heatmap,
        })
    }
}

/// Round score: 500 for a correct answer plus a speed bonus (capped at 100),
/// decaying over 30 s. Wrong answers score 0.
pub fn score(correct: bool, elapsed_ms: u32) -> u32 {
    if !correct {
        return 0;
    }
    let speed_bonus = (30_000u32.saturating_sub(elapsed_ms) / 300).min(100);
    500 + speed_bonus
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question() -> Question {
        Question {
            id: "q1".into(),
            text: "?".into(),
            choices: vec!["a".into(), "b".into(), "c".into()],
            correct_choice: 1,
            source_section_ids: vec!["doc1#s2".into()],
            timer_sec: 30,
        }
    }

    #[test]
    fn scoring_rewards_correctness_and_speed() {
        assert_eq!(score(false, 0), 0);
        assert_eq!(score(true, 0), 600); // full speed bonus
        assert_eq!(score(true, 30_000), 500); // no bonus after 30s
        assert_eq!(score(true, 60_000), 500); // saturates, never below 500
        assert_eq!(score(true, 15_000), 550); // half bonus
    }

    #[test]
    fn join_is_idempotent_and_counts() {
        let mut s = Session::new("s1", "host");
        assert_eq!(s.join("p1", "Alice"), 1);
        assert_eq!(s.join("p2", "Bob"), 2);
        assert_eq!(s.join("p1", "Alice again"), 2); // rejoin: no new participant
        assert_eq!(s.participants["p1"].name, "Alice"); // original name preserved
    }

    #[test]
    fn cannot_answer_before_a_question_is_pushed() {
        let mut s = Session::new("s1", "host");
        s.join("p1", "Alice");
        assert_eq!(s.submit_answer("p1", 1, 100), Err(SessionError::NotAsking));
    }

    #[test]
    fn answer_validation() {
        let mut s = Session::new("s1", "host");
        s.join("p1", "Alice");
        s.push_question(question());
        assert!(s.submit_answer("p1", 1, 100).is_ok());
        assert_eq!(
            s.submit_answer("p1", 0, 200),
            Err(SessionError::AlreadyAnswered)
        );
        assert_eq!(
            s.submit_answer("ghost", 1, 100),
            Err(SessionError::UnknownParticipant)
        );
    }

    #[test]
    fn reveal_without_question_errors() {
        let mut s = Session::new("s1", "host");
        assert_eq!(s.reveal().unwrap_err(), SessionError::NoQuestion);
    }

    #[test]
    fn full_round_scores_ranks_and_builds_heatmap() {
        let mut s = Session::new("s1", "host");
        s.join("p1", "Alice");
        s.join("p2", "Bob");
        s.join("p3", "Carol");
        s.push_question(question());

        s.submit_answer("p1", 1, 1_000).unwrap(); // correct, fast
        s.submit_answer("p2", 1, 20_000).unwrap(); // correct, slow
        s.submit_answer("p3", 0, 2_000).unwrap(); // wrong

        let result = s.reveal().unwrap();
        assert_eq!(s.phase, Phase::Revealed);
        assert_eq!(result.correct_choice, 1);

        // p1 fastest-correct leads, then p2, then p3 (0).
        assert_eq!(result.leaderboard[0].participant_id, "p1");
        assert_eq!(result.leaderboard[1].participant_id, "p2");
        assert_eq!(result.leaderboard[2].participant_id, "p3");
        assert_eq!(result.leaderboard[0].score, score(true, 1_000));
        assert_eq!(result.leaderboard[2].score, 0);

        // One of three answers was wrong → confusion ≈ 0.333 on the source section.
        let confusion = result.heatmap["doc1#s2"];
        assert!((confusion - 1.0 / 3.0).abs() < 1e-6);

        // A new question resets answers and re-enters Asking.
        s.push_question(question());
        assert_eq!(s.phase, Phase::Asking);
        assert!(s.answers.is_empty());
    }
}
