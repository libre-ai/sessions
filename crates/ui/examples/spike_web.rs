//! Spike: Dioxus web shell for rumble-lm.
//!
//! This is a minimal 3-screen app demonstrating Dioxus 0.7 component composition,
//! mocked data, and form handling for rumble-lm use cases:
//! - Session list (summary view)
//! - Live session (real-time state)
//! - Result export (recap + export)
//!
//! All data is mocked (hardcoded). No backend, no persistence.
//! Target: wasm32-unknown-unknown for web/PWA evaluation.
//!
//! To build for wasm: cargo check --example spike_web --target wasm32-unknown-unknown

#![allow(non_snake_case)]

use dioxus::prelude::*;

/// Top-level app navigation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    SessionList,
    LiveSession,
    ResultExport,
}

impl Screen {
    pub fn label(&self) -> &'static str {
        match self {
            Self::SessionList => "Sessions",
            Self::LiveSession => "Live",
            Self::ResultExport => "Export",
        }
    }
}

/// Mocked session summary.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub state: SessionState,
    pub participant_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Draft,
    Live,
    Archived,
}

impl SessionState {
    pub fn badge_class(&self) -> &'static str {
        match self {
            Self::Draft => "badge badge--draft",
            Self::Live => "badge badge--live",
            Self::Archived => "badge badge--archived",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Draft => "Draft",
            Self::Live => "Live",
            Self::Archived => "Archived",
        }
    }
}

/// Mocked live session state.
#[derive(Debug, Clone)]
pub struct LiveSession {
    pub id: String,
    pub title: String,
    pub current_question: String,
    pub answers: Vec<AggregatedAnswer>,
    pub active_participants: usize,
    pub total_participants: usize,
}

#[derive(Debug, Clone)]
pub struct AggregatedAnswer {
    pub text: String,
    pub count: usize,
}

/// Mocked result export data.
#[derive(Debug, Clone)]
pub struct SessionResult {
    pub id: String,
    pub title: String,
    pub participant_count: usize,
    pub question_count: usize,
    pub export_timestamp: String,
}

/// Mocked data providers (pure functions for testability).
pub fn mock_sessions() -> Vec<SessionSummary> {
    vec![
        SessionSummary {
            id: "sess-001".to_string(),
            title: "Introduction to Rust Ownership".to_string(),
            state: SessionState::Live,
            participant_count: 24,
        },
        SessionSummary {
            id: "sess-002".to_string(),
            title: "Advanced TypeScript Patterns".to_string(),
            state: SessionState::Archived,
            participant_count: 18,
        },
        SessionSummary {
            id: "sess-003".to_string(),
            title: "Building Scalable Systems".to_string(),
            state: SessionState::Draft,
            participant_count: 0,
        },
    ]
}

pub fn mock_live_session() -> LiveSession {
    LiveSession {
        id: "sess-001".to_string(),
        title: "Introduction to Rust Ownership".to_string(),
        current_question: "What does the borrow checker prevent?".to_string(),
        answers: vec![
            AggregatedAnswer {
                text: "Double-free errors".to_string(),
                count: 14,
            },
            AggregatedAnswer {
                text: "Use-after-free bugs".to_string(),
                count: 10,
            },
        ],
        active_participants: 22,
        total_participants: 24,
    }
}

pub fn mock_result() -> SessionResult {
    SessionResult {
        id: "sess-001".to_string(),
        title: "Introduction to Rust Ownership".to_string(),
        participant_count: 24,
        question_count: 5,
        export_timestamp: "2026-07-02T11:45:00Z".to_string(),
    }
}

/// Session list screen: shows a list of all sessions.
#[component]
pub fn SessionListScreen() -> Element {
    let sessions = mock_sessions();
    rsx! {
        div { class: "spike-container",
            h1 { "Sessions" }
            div { class: "spike-session-list",
                for session in sessions {
                    div { class: "spike-session-card",
                        div { class: "spike-session-card__header",
                            h2 { class: "spike-session-card__title", "{session.title}" }
                            span { class: session.state.badge_class(), "{session.state.label()}" }
                        }
                        p { class: "spike-session-card__meta",
                            "{session.participant_count} participants"
                        }
                    }
                }
            }
        }
    }
}

/// Live session screen: shows current question, answers, and participation.
#[component]
pub fn LiveSessionScreen() -> Element {
    let session = mock_live_session();
    rsx! {
        div { class: "spike-container",
            h1 { "{session.title}" }
            div { class: "spike-live-stats",
                span { class: "spike-stat",
                    strong { "{session.active_participants}/{session.total_participants}" }
                    " active"
                }
                span { class: "spike-indicator spike-indicator--active", "● Live" }
            }
            div { class: "spike-question-box",
                p { class: "spike-question-label", "Current question:" }
                h2 { class: "spike-question-text", "{session.current_question}" }
            }
            div { class: "spike-answers",
                h3 { "Responses (aggregated)" }
                for answer in session.answers {
                    div { class: "spike-answer-row",
                        span { class: "spike-answer-text", "{answer.text}" }
                        span { class: "spike-answer-count", "{answer.count}" }
                    }
                }
            }
        }
    }
}

/// Result export screen: shows recap and export options.
#[component]
pub fn ResultExportScreen() -> Element {
    let result = mock_result();

    rsx! {
        div { class: "spike-container",
            h1 { "Export Results" }
            div { class: "spike-result-recap",
                h2 { "{result.title}" }
                dl { class: "spike-result-fields",
                    dt { "Participants" }
                    dd { "{result.participant_count}" }
                    dt { "Questions" }
                    dd { "{result.question_count}" }
                    dt { "Exported" }
                    dd { "{result.export_timestamp}" }
                }
            }
            div { class: "spike-export-options",
                h3 { "Export as:" }
                button {
                    class: "spike-export-btn",
                    "📥 JSON"
                }
                button {
                    class: "spike-export-btn",
                    "📄 CSV"
                }
            }
            div { class: "spike-export-status",
                p { "Export ready (mock)" }
            }
        }
    }
}

/// Bottom navigation for screen switching.
#[component]
pub fn BottomNavigation(current_screen: Screen, on_navigate: EventHandler<Screen>) -> Element {
    rsx! {
        nav { class: "spike-bottom-nav",
            for screen in [Screen::SessionList, Screen::LiveSession, Screen::ResultExport] {
                button {
                    class: if screen == current_screen { "spike-nav-btn spike-nav-btn--active" } else { "spike-nav-btn" },
                    onclick: move |_| on_navigate.call(screen),
                    "{screen.label()}"
                }
            }
        }
    }
}

/// Root app component - stateless, used for SSR and testing.
#[component]
pub fn AppDemo(#[props(default = Screen::SessionList)] current_screen: Screen) -> Element {
    rsx! {
        style { "{SPIKE_STYLES}" }
        div { class: "spike-app",
            match current_screen {
                Screen::SessionList => rsx! { SessionListScreen {} },
                Screen::LiveSession => rsx! { LiveSessionScreen {} },
                Screen::ResultExport => rsx! { ResultExportScreen {} },
            }
            BottomNavigation {
                current_screen,
                on_navigate: move |_screen| {
                    // State management would happen here in a real app.
                    // For this spike, we're demonstrating component composition
                    // without stateful wasm machinery.
                },
            }
        }
    }
}

/// Inline CSS for this spike (minimal, demo-purpose).
pub const SPIKE_STYLES: &str = r#"
* {
    box-sizing: border-box;
}

body {
    font-family: system-ui, -apple-system, sans-serif;
    margin: 0;
    padding: 0;
    background: #fafafa;
}

.spike-app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    max-width: 480px;
    margin: 0 auto;
    background: white;
}

.spike-container {
    flex: 1;
    overflow-y: auto;
    padding: 1rem;
}

.spike-container h1 {
    margin: 0 0 1.5rem;
    font-size: 1.5rem;
    color: #333;
}

.spike-session-list {
    display: flex;
    flex-direction: column;
    gap: 1rem;
}

.spike-session-card {
    border: 1px solid #ddd;
    border-radius: 8px;
    padding: 1rem;
    background: #fafafa;
}

.spike-session-card__header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 0.5rem;
}

.spike-session-card__title {
    margin: 0;
    font-size: 1rem;
    color: #333;
}

.badge {
    display: inline-block;
    padding: 0.25rem 0.75rem;
    border-radius: 12px;
    font-size: 0.75rem;
    font-weight: 600;
}

.badge--live {
    background: #d4edda;
    color: #155724;
}

.badge--draft {
    background: #fff3cd;
    color: #856404;
}

.badge--archived {
    background: #e2e3e5;
    color: #383d41;
}

.spike-session-card__meta {
    margin: 0;
    font-size: 0.875rem;
    color: #666;
}

.spike-live-stats {
    display: flex;
    gap: 1rem;
    margin-bottom: 1.5rem;
    font-size: 0.875rem;
    color: #666;
}

.spike-stat {
    display: flex;
    align-items: center;
    gap: 0.25rem;
}

.spike-indicator {
    display: inline-block;
    padding: 0.25rem 0.5rem;
    border-radius: 4px;
    font-weight: 600;
    font-size: 0.75rem;
}

.spike-indicator--active {
    color: #27ae60;
}

.spike-question-box {
    background: #ecf0f1;
    border-left: 4px solid #3498db;
    padding: 1rem;
    margin-bottom: 1.5rem;
    border-radius: 4px;
}

.spike-question-label {
    margin: 0 0 0.5rem;
    font-size: 0.875rem;
    color: #666;
    text-transform: uppercase;
}

.spike-question-text {
    margin: 0;
    font-size: 1.125rem;
    color: #333;
}

.spike-answers {
    margin-bottom: 2rem;
}

.spike-answers h3 {
    margin: 0 0 1rem;
    font-size: 1rem;
    color: #333;
}

.spike-answer-row {
    display: flex;
    justify-content: space-between;
    padding: 0.75rem;
    border-bottom: 1px solid #eee;
    align-items: center;
}

.spike-answer-text {
    color: #333;
    flex: 1;
}

.spike-answer-count {
    display: inline-block;
    min-width: 2rem;
    text-align: right;
    padding: 0.25rem 0.5rem;
    background: #3498db;
    color: white;
    border-radius: 4px;
    font-size: 0.875rem;
    font-weight: 600;
}

.spike-result-recap {
    background: #ecf0f1;
    padding: 1rem;
    border-radius: 8px;
    margin-bottom: 1.5rem;
}

.spike-result-recap h2 {
    margin: 0 0 1rem;
    font-size: 1.125rem;
    color: #333;
}

.spike-result-fields {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 0.75rem;
    margin: 0;
}

.spike-result-fields dt {
    font-weight: 600;
    color: #333;
}

.spike-result-fields dd {
    margin: 0;
    color: #666;
}

.spike-export-options {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    margin-bottom: 2rem;
}

.spike-export-options h3 {
    margin: 0;
    font-size: 0.875rem;
    color: #333;
    text-transform: uppercase;
}

.spike-export-btn {
    padding: 0.75rem 1rem;
    border: 1px solid #3498db;
    background: white;
    color: #3498db;
    border-radius: 4px;
    cursor: pointer;
    font-weight: 600;
    transition: all 0.2s;
}

.spike-export-btn:hover {
    background: #3498db;
    color: white;
}

.spike-export-status {
    padding: 1rem;
    background: #d4edda;
    border: 1px solid #c3e6cb;
    border-radius: 4px;
    color: #155724;
    margin-top: 1rem;
}

.spike-bottom-nav {
    display: flex;
    border-top: 1px solid #ddd;
    background: white;
    height: 60px;
}

.spike-nav-btn {
    flex: 1;
    border: none;
    background: white;
    color: #666;
    cursor: pointer;
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    transition: all 0.2s;
    border-bottom: 3px solid transparent;
}

.spike-nav-btn:hover {
    background: #f5f5f5;
}

.spike-nav-btn--active {
    color: #3498db;
    border-bottom-color: #3498db;
    background: #f9f9f9;
}
"#;

/// Example main: demonstrates all 3 screens using SSR (server-side rendering).
fn main() {
    use dioxus_ssr::render_element;

    println!("=== Dioxus Web Shell Spike ===\n");

    println!("Screen 1: Session List");
    let html = render_element(rsx! { AppDemo { current_screen: Screen::SessionList } });
    println!("HTML length: {} bytes\n", html.len());

    println!("Screen 2: Live Session");
    let html = render_element(rsx! { AppDemo { current_screen: Screen::LiveSession } });
    println!("HTML length: {} bytes\n", html.len());

    println!("Screen 3: Result Export");
    let html = render_element(rsx! { AppDemo { current_screen: Screen::ResultExport } });
    println!("HTML length: {} bytes\n", html.len());

    println!("All screens compiled successfully for wasm32-unknown-unknown target.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_state_badge_class() {
        assert_eq!(SessionState::Live.badge_class(), "badge badge--live");
        assert_eq!(SessionState::Draft.badge_class(), "badge badge--draft");
        assert_eq!(
            SessionState::Archived.badge_class(),
            "badge badge--archived"
        );
    }

    #[test]
    fn screen_label() {
        assert_eq!(Screen::SessionList.label(), "Sessions");
        assert_eq!(Screen::LiveSession.label(), "Live");
        assert_eq!(Screen::ResultExport.label(), "Export");
    }

    #[test]
    fn mock_sessions_populated() {
        let sessions = mock_sessions();
        assert_eq!(sessions.len(), 3);
        assert_eq!(sessions[0].participant_count, 24);
        assert_eq!(sessions[0].state, SessionState::Live);
    }

    #[test]
    fn mock_live_session_has_answers() {
        let session = mock_live_session();
        assert_eq!(session.answers.len(), 2);
        assert_eq!(session.active_participants, 22);
        assert!(session.active_participants <= session.total_participants);
    }

    #[test]
    fn mock_result_consistent() {
        let result = mock_result();
        assert!(!result.title.is_empty());
        assert!(result.participant_count > 0);
    }

    #[test]
    fn app_demo_renders_session_list_by_default() {
        use dioxus_ssr::render_element;
        let html = render_element(rsx! { AppDemo {} });
        assert!(html.contains("Sessions"));
        assert!(html.contains("Introduction to Rust Ownership"));
    }

    #[test]
    fn app_demo_renders_live_session_screen() {
        use dioxus_ssr::render_element;
        let html = render_element(rsx! { AppDemo { current_screen: Screen::LiveSession } });
        assert!(html.contains("Current question:"));
        assert!(html.contains("What does the borrow checker prevent?"));
    }

    #[test]
    fn app_demo_renders_export_screen() {
        use dioxus_ssr::render_element;
        let html = render_element(rsx! { AppDemo { current_screen: Screen::ResultExport } });
        assert!(html.contains("Export Results"));
        assert!(html.contains("Participants"));
    }
}
