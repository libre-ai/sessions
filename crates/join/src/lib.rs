#![allow(non_snake_case)]
#![cfg_attr(not(target_arch = "wasm32"), allow(unused_imports, dead_code))]

use std::cell::RefCell;
use std::collections::BTreeSet;

use dioxus::prelude::*;
use presto_core::api::ApiEnvelope;
use presto_core::protocol::{
    ClientMessage, LeaderboardEntry, ParticipantId, PublicReveal, QuestionKind, QuestionPublic,
    ServerMessage, SessionPhasePublic, SessionSnapshot,
};
use rumble_lm_ui::{
    AppSurface, Card, JoinLeaderboard, JoinQuestion, JoinReveal, JoinStatus, Toast,
};
use serde::Deserialize;
use serde_json::json;

pub const JOIN_STYLES: &str = include_str!("join.css");

#[cfg(target_arch = "wasm32")]
thread_local! {
    static JOIN_CONNECTION: RefCell<Option<JoinTransport>> = const { RefCell::new(None) };
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SubmissionLock {
    Idle,
    Pending,
    Accepted,
}

impl SubmissionLock {
    fn is_locked(&self) -> bool {
        !matches!(self, Self::Idle)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum JoinUiState {
    ReadingLink,
    Invalid {
        reason: String,
    },
    NameEntry {
        session_id: String,
        name: String,
    },
    Joining {
        session_id: String,
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
        submission: SubmissionLock,
        answered: bool,
    },
    Revealed {
        session_id: String,
        participant_id: ParticipantId,
        name: String,
        participants_count: u32,
        question: QuestionPublic,
        selected: BTreeSet<u8>,
        submission: SubmissionLock,
        reveal: PublicReveal,
    },
    Disconnected {
        resume: Box<JoinUiState>,
    },
    Expired {
        reason: String,
    },
    Failed {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct JoinCredentials {
    session_id: String,
    participant_token: String,
    participant_id: ParticipantId,
    name: String,
}

#[cfg(target_arch = "wasm32")]
struct JoinTransport {
    ws: web_sys::WebSocket,
    _onopen: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)>,
    _onmessage: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::MessageEvent)>,
    _onerror: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::ErrorEvent)>,
    _onclose: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::CloseEvent)>,
}

#[derive(Debug, Deserialize)]
struct JoinedSession {
    participant_id: ParticipantId,
    participant_token: String,
}

#[derive(Debug)]
enum JoinError {
    InvalidLink(String),
    Expired(String),
    Busy(String),
    Network(String),
}

#[cfg(target_arch = "wasm32")]
pub fn App() -> Element {
    let mut booted = use_signal(|| false);
    let mut state = use_signal(|| JoinUiState::ReadingLink);
    let mut draft_name = use_signal(String::new);
    let mut join_token = use_signal(|| None::<String>);
    let mut reconnect_attempts = use_signal(|| 0_u8);

    use_effect(move || {
        if *booted.read() {
            return;
        }
        booted.set(true);
        #[cfg(target_arch = "wasm32")]
        {
            match read_join_link() {
                Ok((session_id, token)) => {
                    join_token.set(Some(token));
                    state.set(JoinUiState::NameEntry {
                        session_id,
                        name: String::new(),
                    });
                    clear_join_fragment();
                }
                Err(reason) => state.set(JoinUiState::Invalid { reason }),
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            state.set(JoinUiState::Invalid {
                reason: "client join requires wasm32".into(),
            });
        }
    });

    let current = state.read().clone();
    let draft_value = draft_name.read().clone();
    let status_message = match &current {
        JoinUiState::ReadingLink => "Lecture du lien sécurisé…".to_string(),
        JoinUiState::Invalid { reason } => reason.clone(),
        JoinUiState::NameEntry { .. } => "Entrez votre nom pour rejoindre.".to_string(),
        JoinUiState::Joining { .. } => "Connexion en cours…".to_string(),
        JoinUiState::Lobby {
            participants_count, ..
        } => format!("Connecté — {participants_count} participant(s)"),
        JoinUiState::Asking { answered, .. } => {
            if *answered {
                "Réponse acceptée.".into()
            } else {
                "Question ouverte.".into()
            }
        }
        JoinUiState::Revealed { .. } => "Réponse révélée.".to_string(),
        JoinUiState::Disconnected { .. } => {
            let tries = *reconnect_attempts.read();
            if tries == 0 {
                "Connexion perdue, tentative de reprise…".into()
            } else {
                format!("Connexion perdue, reprise {tries}/5…")
            }
        }
        JoinUiState::Expired { reason } | JoinUiState::Failed { reason } => reason.clone(),
    };

    rsx! {
        AppSurface {
            style { "{JOIN_STYLES}" }
            main { class: "join-shell join-safe-area",
                header { class: "join-hero",
                    p { class: "join-kicker", "Rumble LM · participant" }
                    h1 { "Rejoindre une session" }
                    p { class: "join-lede", "Le token du lien reste uniquement en mémoire. Aucun stockage, cookie ou manifest n’est utilisé par ce client." }
                }
                JoinStatus { message: status_message }
                match current {
                    JoinUiState::ReadingLink => rsx! {
                        Card { title: "Chargement".to_string(), body: "Lecture du code et du fragment…".to_string() }
                    },
                    JoinUiState::Invalid { reason } => rsx! {
                        Card { title: "Lien invalide".to_string(), body: reason }
                    },
                    JoinUiState::NameEntry { session_id, name: _current_name } => {
                        let session_id_for_join = session_id.clone();
                        let on_join = move |_| {
                            let session_id = session_id_for_join.clone();
                            let name = draft_name.read().trim().to_string();
                            let token = join_token.read().clone();
                            if name.is_empty() {
                                state.set(JoinUiState::Invalid { reason: "un nom est requis".into() });
                                return;
                            }
                            let Some(token) = token else {
                                state.set(JoinUiState::Expired { reason: "lien de join expiré".into() });
                                return;
                            };
                            state.set(JoinUiState::Joining { session_id: session_id.clone(), name: name.clone() });
                            spawn(async move {
                                match redeem_join_link(&session_id, &token, &name).await {
                                    Ok(joined) => {
                                        join_token.set(None);
                                        let credentials = JoinCredentials {
                                            session_id: session_id.clone(),
                                            participant_token: joined.participant_token.clone(),
                                            participant_id: joined.participant_id.clone(),
                                            name: name.clone(),
                                        };
                                        state.set(JoinUiState::Lobby {
                                            session_id: session_id.clone(),
                                            participant_id: credentials.participant_id.clone(),
                                            name: name.clone(),
                                            participants_count: 1,
                                        });
                                        reconnect_attempts.set(0);
                                        connect_session(credentials, state, reconnect_attempts);
                                    }
                                    Err(err) => match err {
                                        JoinError::Expired(reason) => state.set(JoinUiState::Expired { reason }),
                                        JoinError::InvalidLink(reason) => state.set(JoinUiState::Invalid { reason }),
                                        JoinError::Busy(reason) | JoinError::Network(reason) => state.set(JoinUiState::Failed { reason }),
                                    },
                                }
                            });
                        };
                        rsx! {
                            section { class: "join-panel",
                                label { class: "presto-label", r#for: "join-name", "Nom" }
                                input {
                                    id: "join-name",
                                    class: "presto-input join-input",
                                    value: "{draft_value}",
                                    maxlength: "24",
                                    autocomplete: "nickname",
                                    autocapitalize: "words",
                                    placeholder: "Ton nom",
                                    oninput: move |event| draft_name.set(event.value()),
                                }
                                div { class: "join-actions",
                                    button {
                                        class: "presto-button presto-button--primary",
                                        disabled: draft_value.trim().is_empty(),
                                        onclick: on_join,
                                        "Rejoindre"
                                    }
                                }
                                p { class: "presto-help", "Session {session_id}" }
                            }
                        }
                    },
                    JoinUiState::Joining { session_id, name } => rsx! {
                        Card { title: "Connexion".to_string(), body: format!("{name} rejoint la session {session_id}.") }
                    },
                    JoinUiState::Lobby { session_id, participant_id, name, participants_count } => rsx! {
                        section { class: "join-stack",
                            Card { title: "Lobby".to_string(), body: format!("{name} · {participant_id} · {participants_count} participant(s)") }
                            p { class: "presto-help", "En attente d’une question…" }
                            p { class: "presto-help", "Code {session_id}" }
                        }
                    },
                    JoinUiState::Asking { question, selected, submission, answered, .. } => rsx! {
                        section { class: "join-stack",
                            JoinQuestion {
                                question: question.clone(),
                                selected: selected.iter().copied().collect::<Vec<_>>(),
                                locked: answered || submission.is_locked(),
                                on_toggle: move |choice| toggle_choice(state, choice),
                                on_submit: move |_| submit_answer(state),
                            }
                            if answered || submission.is_locked() {
                                Toast { message: "Réponse enregistrée".to_string() }
                            }
                        }
                    },
                    JoinUiState::Revealed { question, reveal, .. } => rsx! {
                        section { class: "join-stack",
                            JoinReveal { question: question.clone(), reveal: reveal.clone() }
                            JoinLeaderboard { entries: reveal.leaderboard.clone() }
                        }
                    },
                    JoinUiState::Disconnected { .. } => rsx! {
                        Card { title: "Déconnecté".to_string(), body: "Reconnexion automatique en cours…".to_string() }
                    },
                    JoinUiState::Expired { reason } => rsx! {
                        Card { title: "Lien expiré".to_string(), body: reason }
                    },
                    JoinUiState::Failed { reason } => rsx! {
                        Card { title: "Échec réseau".to_string(), body: reason }
                    },
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn App() -> Element {
    rsx! {
        AppSurface {
            Card { title: "Lien participant".to_string(), body: "Le client join est disponible uniquement en wasm32.".to_string() }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn read_join_link() -> Result<(String, String), String> {
    let window = web_sys::window().ok_or("window indisponible")?;
    let location = window.location();
    let pathname = location
        .pathname()
        .map_err(|_| "impossible de lire l’URL")?;
    let session_id = pathname
        .strip_prefix("/join/")
        .and_then(|rest| rest.split('/').next())
        .filter(|code| validate_session_code(code))
        .ok_or_else(|| "code de session invalide".to_string())?;
    let hash = location
        .hash()
        .map_err(|_| "impossible de lire le fragment")?;
    let token = hash
        .strip_prefix("#token=")
        .filter(|token| !token.is_empty())
        .ok_or_else(|| "fragment de token manquant".to_string())?;
    Ok((session_id.to_string(), token.to_string()))
}

#[cfg(target_arch = "wasm32")]
fn clear_join_fragment() {
    if let Some(window) = web_sys::window() {
        if let Ok(history) = window.history() {
            let location = window.location();
            let pathname = location.pathname().unwrap_or_default();
            let search = location.search().unwrap_or_default();
            let path = format!("{pathname}{search}");
            let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&path));
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn redeem_join_link(
    session_id: &str,
    redemption_token: &str,
    name: &str,
) -> Result<JoinedSession, JoinError> {
    use gloo_net::http::Request;

    let body = json!({ "name": name });
    let response = Request::post(&format!("/join/{session_id}/participants"))
        .header("Authorization", &format!("Bearer {redemption_token}"))
        .json(&body)
        .map_err(|_| JoinError::Network("impossible de préparer la requête".into()))?
        .send()
        .await
        .map_err(|_| JoinError::Network("réseau indisponible".into()))?;

    match response.status() {
        200 => response
            .json::<ApiEnvelope<JoinedSession>>()
            .await
            .map(|envelope| envelope.data)
            .map_err(|_| JoinError::Network("réponse join illisible".into())),
        401 | 403 | 410 => Err(JoinError::Expired("lien de join expiré ou refusé".into())),
        404 => Err(JoinError::InvalidLink("session introuvable".into())),
        429 | 503 => Err(JoinError::Busy("join temporairement indisponible".into())),
        _ => Err(JoinError::Network("join refusé".into())),
    }
}

#[cfg(target_arch = "wasm32")]
fn connect_session(
    credentials: JoinCredentials,
    mut state: Signal<JoinUiState>,
    mut reconnect_attempts: Signal<u8>,
) {
    use wasm_bindgen::{JsCast, closure::Closure};

    let url = websocket_url(
        &credentials.session_id,
        &credentials.participant_token,
        &credentials.name,
    );

    let ws = match web_sys::WebSocket::new(&url) {
        Ok(ws) => ws,
        Err(_) => {
            state.set(JoinUiState::Failed {
                reason: "impossible d’ouvrir la WebSocket".into(),
            });
            return;
        }
    };
    ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

    let onopen = Closure::<dyn FnMut(web_sys::Event)>::wrap(Box::new(move |_| {
        reconnect_attempts.set(0);
    }));

    let mut state_for_message = state;
    let onmessage = Closure::<dyn FnMut(web_sys::MessageEvent)>::wrap(Box::new(
        move |event: web_sys::MessageEvent| {
            let Some(text) = event.data().as_string() else {
                return;
            };
            let Ok(message) = serde_json::from_str::<ServerMessage>(&text) else {
                return;
            };
            let next = apply_server_message(state_for_message.read().clone(), message);
            state_for_message.set(next);
        },
    ));

    let mut state_for_error = state;
    let onerror = Closure::<dyn FnMut(web_sys::ErrorEvent)>::wrap(Box::new(move |_| {
        let current = state_for_error.read().clone();
        if !matches!(
            current,
            JoinUiState::Expired { .. } | JoinUiState::Failed { .. }
        ) {
            state_for_error.set(JoinUiState::Disconnected {
                resume: Box::new(current),
            });
        }
    }));

    let mut state_for_close = state;
    let mut attempts_for_close = reconnect_attempts;
    let credentials_for_close = credentials.clone();
    let onclose = Closure::<dyn FnMut(web_sys::CloseEvent)>::wrap(Box::new(move |_| {
        let current = state_for_close.read().clone();
        if matches!(
            current,
            JoinUiState::Expired { .. } | JoinUiState::Failed { .. }
        ) {
            return;
        }
        state_for_close.set(JoinUiState::Disconnected {
            resume: Box::new(current.clone()),
        });
        let next_attempt = attempts_for_close.read().saturating_add(1);
        attempts_for_close.set(next_attempt);
        if next_attempt > 5 {
            state_for_close.set(JoinUiState::Failed {
                reason: "reconnexion impossible".into(),
            });
            return;
        }
        let delay_ms = 250_u32
            .saturating_mul(1 << (next_attempt.saturating_sub(1) as u32))
            .min(4000);
        let credentials_for_retry = credentials_for_close.clone();
        let state_for_retry = state_for_close;
        let attempts_for_retry = attempts_for_close;
        spawn(async move {
            gloo_timers::future::TimeoutFuture::new(delay_ms).await;
            connect_session(credentials_for_retry, state_for_retry, attempts_for_retry);
        });
    }));

    let transport = JoinTransport {
        ws: ws.clone(),
        _onopen: onopen,
        _onmessage: onmessage,
        _onerror: onerror,
        _onclose: onclose,
    };
    let _ = ws.set_onopen(Some(transport._onopen.as_ref().unchecked_ref()));
    let _ = ws.set_onmessage(Some(transport._onmessage.as_ref().unchecked_ref()));
    let _ = ws.set_onerror(Some(transport._onerror.as_ref().unchecked_ref()));
    let _ = ws.set_onclose(Some(transport._onclose.as_ref().unchecked_ref()));

    JOIN_CONNECTION.with(|slot| *slot.borrow_mut() = Some(transport));
}

#[cfg(target_arch = "wasm32")]
fn apply_server_message(state: JoinUiState, message: ServerMessage) -> JoinUiState {
    match message {
        ServerMessage::Joined {
            participant_id,
            participants,
        } => match state {
            JoinUiState::Joining { session_id, name } => JoinUiState::Lobby {
                session_id,
                participant_id,
                name,
                participants_count: participants,
            },
            JoinUiState::Lobby {
                session_id, name, ..
            }
            | JoinUiState::Asking {
                session_id, name, ..
            }
            | JoinUiState::Revealed {
                session_id, name, ..
            } => JoinUiState::Lobby {
                session_id,
                participant_id,
                name,
                participants_count: participants,
            },
            other => other,
        },
        ServerMessage::Snapshot { snapshot } => apply_snapshot(state, snapshot),
        ServerMessage::QuestionOpened { question } => match state {
            JoinUiState::Lobby {
                session_id,
                participant_id,
                name,
                participants_count,
            } => JoinUiState::Asking {
                session_id,
                participant_id,
                name,
                participants_count,
                question,
                selected: BTreeSet::new(),
                submission: SubmissionLock::Idle,
                answered: false,
            },
            JoinUiState::Asking {
                session_id,
                participant_id,
                name,
                participants_count,
                question: current,
                selected,
                submission,
                answered,
            } if current.id == question.id => JoinUiState::Asking {
                session_id,
                participant_id,
                name,
                participants_count,
                question,
                selected,
                submission,
                answered,
            },
            other => other,
        },
        ServerMessage::AnswerAccepted { question_id } => accept_answer(state, &question_id),
        ServerMessage::AnswersRevealed {
            correct_choices,
            leaderboard,
            heatmap,
        } => reveal_answers(state, correct_choices, leaderboard, heatmap),
        ServerMessage::Error { reason } => JoinUiState::Failed { reason },
        ServerMessage::AnswerReceived { .. } => state,
        ServerMessage::Pong
        | ServerMessage::BreakoutOpened { .. }
        | ServerMessage::FlashcardsReady { .. } => state,
    }
}

#[cfg(target_arch = "wasm32")]
fn apply_snapshot(state: JoinUiState, snapshot: SessionSnapshot) -> JoinUiState {
    match snapshot.phase {
        SessionPhasePublic::Lobby => match state {
            JoinUiState::Joining { session_id, name } => JoinUiState::Lobby {
                session_id,
                participant_id: "unknown".into(),
                name,
                participants_count: snapshot.participants_count,
            },
            JoinUiState::Lobby {
                session_id,
                participant_id,
                name,
                ..
            } => JoinUiState::Lobby {
                session_id,
                participant_id,
                name,
                participants_count: snapshot.participants_count,
            },
            JoinUiState::Asking {
                session_id,
                participant_id,
                name,
                ..
            } => JoinUiState::Lobby {
                session_id,
                participant_id,
                name,
                participants_count: snapshot.participants_count,
            },
            JoinUiState::Revealed {
                session_id,
                participant_id,
                name,
                ..
            } => JoinUiState::Lobby {
                session_id,
                participant_id,
                name,
                participants_count: snapshot.participants_count,
            },
            other => other,
        },
        SessionPhasePublic::Asking => {
            let Some(question) = snapshot.question else {
                return JoinUiState::Failed {
                    reason: "snapshot asking sans question".into(),
                };
            };
            match state {
                JoinUiState::Lobby {
                    session_id,
                    participant_id,
                    name,
                    ..
                }
                | JoinUiState::Asking {
                    session_id,
                    participant_id,
                    name,
                    ..
                }
                | JoinUiState::Revealed {
                    session_id,
                    participant_id,
                    name,
                    ..
                } => JoinUiState::Asking {
                    session_id,
                    participant_id,
                    name,
                    participants_count: snapshot.participants_count,
                    question,
                    selected: BTreeSet::new(),
                    submission: if snapshot.answered {
                        SubmissionLock::Accepted
                    } else {
                        SubmissionLock::Idle
                    },
                    answered: snapshot.answered,
                },
                other => other,
            }
        }
        SessionPhasePublic::Revealed => {
            let Some(question) = snapshot.question else {
                return JoinUiState::Failed {
                    reason: "snapshot revealed sans question".into(),
                };
            };
            let Some(reveal) = snapshot.reveal else {
                return JoinUiState::Failed {
                    reason: "snapshot revealed sans reveal".into(),
                };
            };
            match state {
                JoinUiState::Lobby {
                    session_id,
                    participant_id,
                    name,
                    ..
                }
                | JoinUiState::Asking {
                    session_id,
                    participant_id,
                    name,
                    ..
                }
                | JoinUiState::Revealed {
                    session_id,
                    participant_id,
                    name,
                    ..
                } => JoinUiState::Revealed {
                    session_id,
                    participant_id,
                    name,
                    participants_count: snapshot.participants_count,
                    question,
                    selected: BTreeSet::new(),
                    submission: SubmissionLock::Accepted,
                    reveal,
                },
                other => other,
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn accept_answer(state: JoinUiState, question_id: &str) -> JoinUiState {
    match state {
        JoinUiState::Asking {
            session_id,
            participant_id,
            name,
            participants_count,
            question,
            selected,
            ..
        } if question.id == question_id => JoinUiState::Asking {
            session_id,
            participant_id,
            name,
            participants_count,
            question,
            selected,
            submission: SubmissionLock::Accepted,
            answered: true,
        },
        JoinUiState::Revealed {
            session_id,
            participant_id,
            name,
            participants_count,
            question,
            selected,
            reveal,
            ..
        } if reveal.question_id == question_id || question.id == question_id => {
            JoinUiState::Revealed {
                session_id,
                participant_id,
                name,
                participants_count,
                question,
                selected,
                submission: SubmissionLock::Accepted,
                reveal,
            }
        }
        other => other,
    }
}

#[cfg(target_arch = "wasm32")]
fn reveal_answers(
    state: JoinUiState,
    correct_choices: Vec<u8>,
    leaderboard: Vec<LeaderboardEntry>,
    heatmap: std::collections::BTreeMap<String, f32>,
) -> JoinUiState {
    match state {
        JoinUiState::Asking {
            session_id,
            participant_id,
            name,
            participants_count,
            question,
            selected,
            ..
        } => {
            let question_id = question.id.clone();
            JoinUiState::Revealed {
                session_id,
                participant_id,
                name,
                participants_count,
                question,
                selected,
                submission: SubmissionLock::Accepted,
                reveal: PublicReveal {
                    question_id,
                    correct_choices,
                    leaderboard,
                    heatmap,
                },
            }
        }
        JoinUiState::Lobby {
            session_id,
            participant_id,
            name,
            participants_count,
        } => JoinUiState::Revealed {
            session_id,
            participant_id,
            name,
            participants_count,
            question: QuestionPublic {
                id: "unknown".into(),
                text: "Révélation".into(),
                kind: QuestionKind::Single,
                choices: vec![],
                timer_sec: 0,
                grounding: Default::default(),
            },
            selected: BTreeSet::new(),
            submission: SubmissionLock::Accepted,
            reveal: PublicReveal {
                question_id: "unknown".into(),
                correct_choices,
                leaderboard,
                heatmap,
            },
        },
        other => other,
    }
}

#[cfg(target_arch = "wasm32")]
fn toggle_choice(mut state: Signal<JoinUiState>, choice: u8) {
    let next = match state.read().clone() {
        JoinUiState::Asking {
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
                JoinUiState::Failed {
                    reason: "choix hors plage".into(),
                }
            } else {
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
                JoinUiState::Asking {
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
        }
        other => other,
    };
    state.set(next);
}

#[cfg(target_arch = "wasm32")]
fn submit_answer(mut state: Signal<JoinUiState>) {
    let next = match state.read().clone() {
        JoinUiState::Asking {
            session_id,
            participant_id,
            name,
            participants_count,
            question,
            selected,
            submission,
            answered,
        } if !submission.is_locked() && !answered => {
            let choices: Vec<u8> = selected.iter().copied().collect();
            if choices.is_empty()
                || (matches!(question.kind, QuestionKind::Single) && choices.len() != 1)
            {
                JoinUiState::Failed {
                    reason: "sélection invalide".into(),
                }
            } else {
                send_join_message(&ClientMessage::SubmitAnswer {
                    question_id: question.id.clone(),
                    choices,
                });
                JoinUiState::Asking {
                    session_id,
                    participant_id,
                    name,
                    participants_count,
                    question,
                    selected,
                    submission: SubmissionLock::Pending,
                    answered: false,
                }
            }
        }
        other => other,
    };
    state.set(next);
}

#[cfg(target_arch = "wasm32")]
fn send_join_message(message: &ClientMessage) {
    JOIN_CONNECTION.with(|slot| {
        if let Some(connection) = slot.borrow().as_ref() {
            let _ = connection
                .ws
                .send_with_str(&serde_json::to_string(message).unwrap_or_default());
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn validate_session_code(session_id: &str) -> bool {
    matches!(session_id.len(), 6 | 12)
        && session_id
            .bytes()
            .all(|byte| b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789".contains(&byte))
}

#[cfg(target_arch = "wasm32")]
fn websocket_url(session_id: &str, token: &str, name: &str) -> String {
    let window = web_sys::window().expect("window");
    let location = window.location();
    let protocol = location.protocol().unwrap_or_default();
    let scheme = if protocol == "https:" { "wss" } else { "ws" };
    let host = location.host().unwrap_or_default();
    let token = urlencoding::encode(token);
    let name = urlencoding::encode(name);
    format!("{scheme}://{host}/ws/{session_id}?token={token}&name={name}")
}
