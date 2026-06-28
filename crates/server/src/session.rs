//! The authoritative live-session engine — a pure state machine (no async, no
//! WebSocket, no auth). The WS layer and the trait seams (SessionStore, Fanout,
//! RateLimiter) wrap this in later slices; keeping the rules pure makes them
//! exhaustively unit-testable.

use std::collections::BTreeMap;

use presto_core::protocol::{LeaderboardEntry, ParticipantId, Question, QuestionPublic};

/// Grace added to a question's timer before the server closes it to answers, to
/// allow for network latency on an answer sent just before the deadline.
pub const ANSWER_GRACE_MS: u64 = 1500;

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

#[derive(Debug, Clone)]
pub struct Answer {
    /// The selected choice indices (one for single-choice, several for multi).
    pub choices: Vec<u8>,
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
    /// The question's timer (plus grace) has elapsed; answers are closed.
    Closed,
}

/// The outcome of a reveal: the correct choice(s), the sorted leaderboard, and a
/// per-source-section confusion heatmap.
#[derive(Debug, Clone, PartialEq)]
pub struct RevealResult {
    pub correct_choices: Vec<u8>,
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
    /// Epoch-millis at which `current` was opened (server clock); the basis for
    /// answer timing and the close deadline.
    pub opened_at_ms: Option<u64>,
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
            opened_at_ms: None,
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

    /// Open a new question at `opened_at_ms` (server clock): clears prior answers
    /// and enters `Asking`.
    pub fn push_question(&mut self, question: Question, opened_at_ms: u64) {
        self.current = Some(question);
        self.opened_at_ms = Some(opened_at_ms);
        self.answers.clear();
        self.phase = Phase::Asking;
    }

    /// The currently open question's public projection, if one is open (for a
    /// participant joining or reconnecting mid-question).
    pub fn open_question(&self) -> Option<QuestionPublic> {
        if self.phase == Phase::Asking {
            self.current.as_ref().map(Question::public)
        } else {
            None
        }
    }

    /// Record a participant's answer (once, while `Asking`, before the deadline).
    /// `now_ms` is the server clock; elapsed time is computed here, never trusted
    /// from the client.
    pub fn submit_answer(
        &mut self,
        participant_id: &str,
        choices: Vec<u8>,
        now_ms: u64,
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
        let opened = self.opened_at_ms.ok_or(SessionError::NotAsking)?;
        let timer_ms = self
            .current
            .as_ref()
            .map_or(0, |q| u64::from(q.timer_sec) * 1000);
        if now_ms > opened + timer_ms + ANSWER_GRACE_MS {
            return Err(SessionError::Closed);
        }
        let elapsed_ms = u32::try_from(now_ms.saturating_sub(opened)).unwrap_or(u32::MAX);
        self.answers.insert(
            participant_id.to_string(),
            Answer {
                choices,
                elapsed_ms,
            },
        );
        Ok(())
    }

    /// Score the round, build the leaderboard + heatmap, enter `Revealed`.
    pub fn reveal(&mut self) -> Result<RevealResult, SessionError> {
        // Extract what we need from the question before mutating participants,
        // so the immutable borrow of `self.current` is released first.
        let (correct, sections) = {
            let q = self.current.as_ref().ok_or(SessionError::NoQuestion)?;
            (q.correct_choices.clone(), q.source_section_ids.clone())
        };

        for (pid, answer) in &self.answers {
            if is_correct(&answer.choices, &correct)
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
                .filter(|a| !is_correct(&a.choices, &correct))
                .count() as f32
                / answered
        } else {
            0.0
        };
        let heatmap = sections.into_iter().map(|s| (s, confusion)).collect();

        self.phase = Phase::Revealed;
        Ok(RevealResult {
            correct_choices: correct,
            leaderboard,
            heatmap,
        })
    }
}

/// Whether a submitted set of choice indices exactly matches the correct set
/// (order- and duplicate-insensitive). Works for single- and multi-select.
pub fn is_correct(submitted: &[u8], correct: &[u8]) -> bool {
    let norm = |v: &[u8]| {
        let mut s: Vec<u8> = v.to_vec();
        s.sort_unstable();
        s.dedup();
        s
    };
    !correct.is_empty() && norm(submitted) == norm(correct)
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
    use presto_core::protocol::QuestionKind;

    fn question() -> Question {
        Question {
            id: "q1".into(),
            text: "?".into(),
            kind: QuestionKind::Single,
            choices: vec!["a".into(), "b".into(), "c".into()],
            correct_choices: vec![1],
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
        assert_eq!(
            s.submit_answer("p1", vec![1], 100),
            Err(SessionError::NotAsking)
        );
    }

    #[test]
    fn answer_validation() {
        let mut s = Session::new("s1", "host");
        s.join("p1", "Alice");
        s.push_question(question(), 0);
        assert!(s.submit_answer("p1", vec![1], 100).is_ok());
        assert_eq!(
            s.submit_answer("p1", vec![0], 200),
            Err(SessionError::AlreadyAnswered)
        );
        assert_eq!(
            s.submit_answer("ghost", vec![1], 100),
            Err(SessionError::UnknownParticipant)
        );
    }

    #[test]
    fn reveal_without_question_errors() {
        let mut s = Session::new("s1", "host");
        assert_eq!(s.reveal().unwrap_err(), SessionError::NoQuestion);
    }

    #[test]
    fn answers_after_the_deadline_are_closed() {
        let mut s = Session::new("s1", "host");
        s.join("p1", "Alice");
        s.join("p2", "Bob");
        s.push_question(question(), 0); // timer_sec = 30 → close at 30_000 + grace
        // Within the timer + grace window: accepted, server-timed.
        assert!(s.submit_answer("p1", vec![1], 31_000).is_ok());
        assert_eq!(s.answers["p1"].elapsed_ms, 31_000);
        // Past timer + grace (30_000 + 1_500): rejected.
        assert_eq!(
            s.submit_answer("p2", vec![1], 31_501),
            Err(SessionError::Closed)
        );
    }

    #[test]
    fn open_question_tracks_the_asking_phase() {
        let mut s = Session::new("s1", "host");
        assert!(s.open_question().is_none()); // Lobby
        s.push_question(question(), 0);
        assert_eq!(s.open_question().unwrap().id, "q1"); // Asking → public question
        s.reveal().unwrap();
        assert!(s.open_question().is_none()); // Revealed
    }

    #[test]
    fn full_round_scores_ranks_and_builds_heatmap() {
        let mut s = Session::new("s1", "host");
        s.join("p1", "Alice");
        s.join("p2", "Bob");
        s.join("p3", "Carol");
        s.push_question(question(), 0);

        s.submit_answer("p1", vec![1], 1_000).unwrap(); // correct, fast
        s.submit_answer("p2", vec![1], 20_000).unwrap(); // correct, slow
        s.submit_answer("p3", vec![0], 2_000).unwrap(); // wrong

        let result = s.reveal().unwrap();
        assert_eq!(s.phase, Phase::Revealed);
        assert_eq!(result.correct_choices, vec![1]);

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
        s.push_question(question(), 0);
        assert_eq!(s.phase, Phase::Asking);
        assert!(s.answers.is_empty());
    }

    #[test]
    fn is_correct_compares_as_sets() {
        assert!(is_correct(&[2, 0], &[0, 2])); // order-insensitive
        assert!(is_correct(&[1], &[1]));
        assert!(!is_correct(&[0], &[0, 2])); // incomplete
        assert!(!is_correct(&[0, 2, 3], &[0, 2])); // extra
        assert!(!is_correct(&[], &[])); // empty correct never matches
    }

    #[test]
    fn multi_select_scores_exact_set_only() {
        let mut s = Session::new("s1", "host");
        s.join("p1", "Alice");
        s.join("p2", "Bob");
        let q = Question {
            id: "m".into(),
            text: "?".into(),
            kind: QuestionKind::Multi,
            choices: vec!["a".into(), "b".into(), "c".into(), "d".into()],
            correct_choices: vec![0, 2],
            source_section_ids: vec!["sec".into()],
            timer_sec: 30,
        };
        s.push_question(q, 0);
        s.submit_answer("p1", vec![2, 0], 1_000).unwrap(); // correct (set match)
        s.submit_answer("p2", vec![0], 1_000).unwrap(); // wrong (incomplete)
        let r = s.reveal().unwrap();
        assert_eq!(r.correct_choices, vec![0, 2]);
        let p1 = r
            .leaderboard
            .iter()
            .find(|e| e.participant_id == "p1")
            .unwrap();
        let p2 = r
            .leaderboard
            .iter()
            .find(|e| e.participant_id == "p2")
            .unwrap();
        assert!(p1.score >= 500);
        assert_eq!(p2.score, 0);
    }
}
