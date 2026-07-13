#![allow(non_snake_case)]
#![cfg_attr(not(target_arch = "wasm32"), allow(unused_imports, dead_code))]

use std::cell::RefCell;

use dioxus::prelude::*;
use presto_core::api::ApiEnvelope;
use presto_core::guest_join::{GuestJoinEvent, GuestJoinState};
use presto_core::protocol::{ClientMessage, ParticipantId, ServerMessage};
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
struct JoinCredentials {
    session_id: String,
    participant_token: String,
    participant_id: ParticipantId,
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
    let mut state = use_signal(|| GuestJoinState::reading_link("", false));
    let mut draft_name = use_signal(String::new);
    let mut join_token = use_signal(|| None::<String>);
    let mut reconnect_attempts = use_signal(|| 0_u8);
    let connection_epoch = use_signal(|| 0_u64);
    let mut participant_credentials = use_signal(|| None::<JoinCredentials>);
    let initial_join = use_signal(read_join_link_and_scrub);

    use_effect(move || {
        if *booted.read() {
            return;
        }
        booted.set(true);
        match initial_join.read().clone() {
            Ok((session_id, token)) => {
                join_token.set(Some(token));
                state.set(GuestJoinState::name_entry(session_id, true, String::new()));
            }
            Err(reason) => state.set(GuestJoinState::invalid(reason)),
        }
    });

    use_effect(move || {
        use wasm_bindgen::JsCast;

        let Some(window) = web_sys::window() else {
            return;
        };
        let mut state_for_offline = state;
        let reconnect_attempts_for_offline = reconnect_attempts;
        let connection_epoch_for_offline = connection_epoch;
        let participant_credentials_for_offline = participant_credentials;
        let offline = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::wrap(Box::new(
            move |_| {
                if let Some(credentials) = participant_credentials_for_offline.read().clone() {
                    let current = state_for_offline.read().clone();
                    if matches!(
                        current,
                        GuestJoinState::Expired { .. } | GuestJoinState::Failed { .. }
                    ) {
                        return;
                    }
                    clear_join_connection();
                    state_for_offline.set(current.apply_event(GuestJoinEvent::Disconnected));
                    schedule_reconnect(
                        credentials,
                        state_for_offline,
                        reconnect_attempts_for_offline,
                        connection_epoch_for_offline,
                    );
                }
            },
        ));
        let _ =
            window.add_event_listener_with_callback("offline", offline.as_ref().unchecked_ref());
        std::mem::forget(offline);

        let poll = wasm_bindgen::closure::Closure::<dyn FnMut()>::wrap(Box::new(move || {
            let Some(window) = web_sys::window() else {
                return;
            };
            if window.navigator().on_line() {
                return;
            }
            if let Some(credentials) = participant_credentials.read().clone() {
                let current = state.read().clone();
                if matches!(
                    current,
                    GuestJoinState::Disconnected { .. }
                        | GuestJoinState::Expired { .. }
                        | GuestJoinState::Failed { .. }
                ) {
                    return;
                }
                clear_join_connection();
                state.set(current.apply_event(GuestJoinEvent::Disconnected));
                schedule_reconnect(credentials, state, reconnect_attempts, connection_epoch);
            }
        }));
        let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
            poll.as_ref().unchecked_ref(),
            250,
        );
        std::mem::forget(poll);
    });

    let current = state.read().clone();
    let draft_value = draft_name.read().clone();
    let status_message = join_status_message(&current, *reconnect_attempts.read());

    rsx! {
        AppSurface {
            style { "{JOIN_STYLES}" }
            div { class: "join-shell join-safe-area", aria_label: "Espace participant",
                header { class: "join-hero",
                    p { class: "join-kicker", "Rumble LM · participant" }
                    h1 { "Rejoindre une session" }
                    p { class: "join-lede", "Le token du lien reste uniquement en mémoire. Aucun stockage, cookie ou manifest n’est utilisé par ce client." }
                }
                JoinStatus { message: status_message }
                match current {
                    GuestJoinState::ReadingLink { .. } => rsx! {
                        Card { title: "Chargement".to_string(), body: "Lecture du code et du fragment…".to_string() }
                    },
                    GuestJoinState::Invalid { reason } => rsx! {
                        Card { title: "Lien invalide".to_string(), body: reason }
                    },
                    GuestJoinState::NameEntry { session_id, .. } => {
                        let session_id_for_join = session_id.clone();
                        let on_join = move |_| {
                            let session_id = session_id_for_join.clone();
                            let name = draft_name.read().trim().to_string();
                            let token = join_token.read().clone();
                            if name.is_empty() {
                                state.set(GuestJoinState::invalid("un nom est requis"));
                                return;
                            }
                            let Some(token) = token else {
                                state.set(GuestJoinState::Expired { reason: "lien de join expiré".into() });
                                return;
                            };
                            state.set(GuestJoinState::joining(session_id.clone(), true, name.clone()));
                            spawn(async move {
                                match redeem_join_link(&session_id, &token, &name).await {
                                    Ok(joined) => {
                                        join_token.set(None);
                                        let credentials = JoinCredentials {
                                            session_id: session_id.clone(),
                                            participant_token: joined.participant_token.clone(),
                                            participant_id: joined.participant_id.clone(),
                                        };
                                        participant_credentials.set(Some(credentials.clone()));
                                        state.set(GuestJoinState::lobby(
                                            session_id.clone(),
                                            credentials.participant_id.clone(),
                                            name.clone(),
                                            1,
                                        ));
                                        reconnect_attempts.set(0);
                                        let epoch = next_connection_epoch(connection_epoch);
                                        connect_session(
                                            credentials,
                                            state,
                                            reconnect_attempts,
                                            connection_epoch,
                                            epoch,
                                        );
                                    }
                                    Err(err) => match err {
                                        JoinError::Expired(reason) => state.set(GuestJoinState::Expired { reason }),
                                        JoinError::InvalidLink(reason) => state.set(GuestJoinState::invalid(reason)),
                                        JoinError::Busy(reason) | JoinError::Network(reason) => {
                                            state.set(GuestJoinState::Failed { reason })
                                        }
                                    },
                                }
                            });
                        };
                        rsx! {
                            section { class: "join-panel", aria_label: "Connexion" ,
                                label { class: "presto-label", r#for: "join-name", "Nom" }
                                input {
                                    id: "join-name",
                                    class: "presto-input join-input",
                                    value: "{draft_value}",
                                    maxlength: "24",
                                    autocomplete: "nickname",
                                    autocapitalize: "words",
                                    placeholder: "Ton nom",
                                    oninput: move |event| {
                                        let name = event.value();
                                        draft_name.set(name.clone());
                                        let next = state.read().clone().apply_event(GuestJoinEvent::NameEdited { name });
                                        state.set(next);
                                    },
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
                    GuestJoinState::Joining { session_id, name, .. } => rsx! {
                        Card { title: "Connexion".to_string(), body: format!("{name} rejoint la session {session_id}.") }
                    },
                    GuestJoinState::Lobby {
                        session_id,
                        participant_id,
                        name,
                        participants_count,
                    } => rsx! {
                        section { class: "join-stack",
                            Card { title: "Lobby".to_string(), body: format!("{name} · {participant_id} · {participants_count} participant(s)") }
                            p { class: "presto-help", "En attente d’une question…" }
                            p { class: "presto-help", "Code {session_id}" }
                        }
                    },
                    GuestJoinState::Asking {
                        ref question,
                        submission: _,
                        ..
                    } => rsx! {
                        section { class: "join-stack",
                            JoinQuestion {
                                question: question.clone(),
                                selected: current.selected_choices(),
                                locked: current.is_locked(),
                                on_toggle: move |choice| {
                                    let next = state.read().clone().apply_event(GuestJoinEvent::ToggleChoice { choice });
                                    state.set(next);
                                },
                                on_submit: move |_| submit_answer(state),
                            }
                            if current.is_locked() {
                                Toast { message: "Réponse enregistrée".to_string() }
                            }
                        }
                    },
                    GuestJoinState::Revealed { question, reveal, .. } => rsx! {
                        section { class: "join-stack",
                            match question {
                                Some(question) => rsx! { JoinReveal { question: question.clone(), reveal: reveal.clone() } },
                                None => rsx! { Card { title: "Révélation".to_string(), body: "Réponse révélée.".to_string() } },
                            }
                            JoinLeaderboard { entries: reveal.leaderboard.clone() }
                        }
                    },
                    GuestJoinState::Disconnected { .. } => rsx! {
                        Card { title: "Déconnecté".to_string(), body: "Reconnexion automatique en cours…".to_string() }
                    },
                    GuestJoinState::Expired { reason } => rsx! {
                        Card { title: "Lien expiré".to_string(), body: reason }
                    },
                    GuestJoinState::Failed { reason } => rsx! {
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
fn next_connection_epoch(mut connection_epoch: Signal<u64>) -> u64 {
    let next = connection_epoch.read().wrapping_add(1);
    connection_epoch.set(next);
    next
}

#[cfg(target_arch = "wasm32")]
fn join_status_message(state: &GuestJoinState, reconnect_attempts: u8) -> String {
    match state {
        GuestJoinState::ReadingLink { .. } => "Lecture du lien sécurisé…".to_string(),
        GuestJoinState::Invalid { reason } => reason.clone(),
        GuestJoinState::NameEntry { .. } => "Entrez votre nom pour rejoindre.".to_string(),
        GuestJoinState::Joining { .. } => "Connexion en cours…".to_string(),
        GuestJoinState::Lobby {
            participants_count, ..
        } => format!("Connecté — {participants_count} participant(s)"),
        GuestJoinState::Asking { answered, .. } => {
            if *answered || state.is_locked() {
                "Réponse acceptée.".into()
            } else {
                "Question ouverte.".into()
            }
        }
        GuestJoinState::Revealed { .. } => "Réponse révélée.".to_string(),
        GuestJoinState::Disconnected { .. } => {
            if reconnect_attempts == 0 {
                "Connexion perdue, tentative de reprise…".into()
            } else {
                format!("Connexion perdue, reprise {reconnect_attempts}/5…")
            }
        }
        GuestJoinState::Expired { reason } | GuestJoinState::Failed { reason } => reason.clone(),
    }
}

#[cfg(target_arch = "wasm32")]
fn read_join_link_and_scrub() -> Result<(String, String), String> {
    let window = web_sys::window().ok_or("window indisponible")?;
    let location = window.location();
    let pathname = location
        .pathname()
        .map_err(|_| "impossible de lire l’URL")?;
    let search = location.search().map_err(|_| "impossible de lire l’URL")?;
    let mut hash = location
        .hash()
        .map_err(|_| "impossible de lire le fragment")?;
    if hash.is_empty() {
        if let Ok(fragment) = js_sys::Reflect::get(
            &window,
            &wasm_bindgen::JsValue::from_str("__PRESTO_JOIN_FRAGMENT__"),
        ) {
            if let Some(fragment) = fragment.as_string() {
                hash = fragment;
            }
        }
    }
    clear_join_fragment(&window, &pathname, &search);

    let session_id = pathname
        .strip_prefix("/join/")
        .and_then(|rest| rest.split('/').next())
        .filter(|code| validate_session_code(code))
        .ok_or_else(|| "code de session invalide".to_string())?;
    let token = hash
        .strip_prefix("#token=")
        .filter(|token| !token.is_empty())
        .ok_or_else(|| "fragment de token manquant".to_string())?;
    Ok((session_id.to_string(), token.to_string()))
}

#[cfg(target_arch = "wasm32")]
fn clear_join_fragment(window: &web_sys::Window, pathname: &str, search: &str) {
    let path = format!("{pathname}{search}");
    if let Ok(history) = window.history() {
        let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&path));
    }
    let _ = window.location().set_hash("");
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
async fn validate_join_resume(session_id: &str, participant_token: &str) -> Result<(), JoinError> {
    use gloo_net::http::Request;

    let response = Request::get(&format!("/sessions/{session_id}/participants/resume"))
        .header("Authorization", &format!("Bearer {participant_token}"))
        .send()
        .await
        .map_err(|_| JoinError::Network("réseau indisponible".into()))?;

    match response.status() {
        204 => Ok(()),
        401 | 403 | 410 => Err(JoinError::Expired("reconnexion expirée".into())),
        404 => Err(JoinError::InvalidLink("session introuvable".into())),
        429 | 503 => Err(JoinError::Busy(
            "reconnexion temporairement indisponible".into(),
        )),
        _ => Err(JoinError::Network("reconnexion refusée".into())),
    }
}

#[cfg(target_arch = "wasm32")]
fn connect_session(
    credentials: JoinCredentials,
    mut state: Signal<GuestJoinState>,
    mut reconnect_attempts: Signal<u8>,
    connection_epoch: Signal<u64>,
    epoch: u64,
) {
    use wasm_bindgen::{JsCast, closure::Closure};

    clear_join_connection();
    if *connection_epoch.read() != epoch {
        return;
    }

    let url = websocket_url(&credentials.session_id, &credentials.participant_token);
    let ws = match web_sys::WebSocket::new(&url) {
        Ok(ws) => ws,
        Err(_) => {
            state.set(GuestJoinState::Failed {
                reason: "impossible d’ouvrir la WebSocket".into(),
            });
            return;
        }
    };
    ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

    let epoch_for_open = epoch;
    let onopen = Closure::<dyn FnMut(web_sys::Event)>::wrap(Box::new(move |_| {
        if *connection_epoch.read() == epoch_for_open {
            reconnect_attempts.set(0);
        }
    }));

    let mut state_for_message = state;
    let epoch_for_message = epoch;
    let onmessage = Closure::<dyn FnMut(web_sys::MessageEvent)>::wrap(Box::new(
        move |event: web_sys::MessageEvent| {
            if *connection_epoch.read() != epoch_for_message {
                return;
            }
            let Some(text) = event.data().as_string() else {
                return;
            };
            let Ok(message) = serde_json::from_str::<ServerMessage>(&text) else {
                return;
            };
            if let Some(event) = GuestJoinEvent::from_server_message(message) {
                let current = state_for_message.read().clone();
                let mut next = current.clone().apply_event(event.clone());
                if matches!(current, GuestJoinState::Disconnected { .. })
                    && matches!(event, GuestJoinEvent::Snapshot { .. })
                {
                    next = next.apply_event(GuestJoinEvent::Reconnected);
                }
                state_for_message.set(next);
            }
        },
    ));

    let mut state_for_error = state;
    let attempts_for_error = reconnect_attempts;
    let epoch_for_error = connection_epoch;
    let credentials_for_error = credentials.clone();
    let onerror = Closure::<dyn FnMut(web_sys::ErrorEvent)>::wrap(Box::new(move |_| {
        if *epoch_for_error.read() != epoch {
            return;
        }
        let current = state_for_error.read().clone();
        if matches!(
            current,
            GuestJoinState::Expired { .. } | GuestJoinState::Failed { .. }
        ) {
            return;
        }
        state_for_error.set(current.clone().apply_event(GuestJoinEvent::Disconnected));
        schedule_reconnect(
            credentials_for_error.clone(),
            state_for_error,
            attempts_for_error,
            epoch_for_error,
        );
    }));

    let mut state_for_close = state;
    let attempts_for_close = reconnect_attempts;
    let epoch_for_close = connection_epoch;
    let credentials_for_close = credentials.clone();
    let onclose = Closure::<dyn FnMut(web_sys::CloseEvent)>::wrap(Box::new(move |_| {
        if *epoch_for_close.read() != epoch {
            return;
        }
        let current = state_for_close.read().clone();
        if matches!(
            current,
            GuestJoinState::Expired { .. } | GuestJoinState::Failed { .. }
        ) {
            return;
        }
        state_for_close.set(current.clone().apply_event(GuestJoinEvent::Disconnected));
        schedule_reconnect(
            credentials_for_close.clone(),
            state_for_close,
            attempts_for_close,
            epoch_for_close,
        );
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
fn schedule_reconnect(
    credentials: JoinCredentials,
    mut state: Signal<GuestJoinState>,
    mut reconnect_attempts: Signal<u8>,
    connection_epoch: Signal<u64>,
) {
    let attempt = reconnect_attempts.read().saturating_add(1);
    reconnect_attempts.set(attempt);
    if attempt > 5 {
        state.set(GuestJoinState::Failed {
            reason: "reconnexion impossible".into(),
        });
        return;
    }
    let delay_ms = 250_u32
        .saturating_mul(1 << (attempt.saturating_sub(1) as u32))
        .min(4_000);
    let epoch = next_connection_epoch(connection_epoch);
    spawn(async move {
        gloo_timers::future::TimeoutFuture::new(delay_ms).await;
        if *connection_epoch.read() != epoch {
            return;
        }
        match validate_join_resume(&credentials.session_id, &credentials.participant_token).await {
            Ok(()) => {
                if *connection_epoch.read() == epoch {
                    connect_session(
                        credentials,
                        state,
                        reconnect_attempts,
                        connection_epoch,
                        epoch,
                    );
                }
            }
            Err(JoinError::Expired(reason)) => state.set(GuestJoinState::Expired { reason }),
            Err(JoinError::InvalidLink(reason)) => state.set(GuestJoinState::invalid(reason)),
            Err(JoinError::Busy(_)) | Err(JoinError::Network(_)) => {
                schedule_reconnect(credentials, state, reconnect_attempts, connection_epoch)
            }
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn clear_join_connection() {
    JOIN_CONNECTION.with(|slot| {
        if let Some(transport) = slot.borrow_mut().take() {
            let _ = transport.ws.set_onopen(None);
            let _ = transport.ws.set_onmessage(None);
            let _ = transport.ws.set_onerror(None);
            let _ = transport.ws.set_onclose(None);
            let _ = transport.ws.close();
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn submit_answer(mut state: Signal<GuestJoinState>) {
    let next = state
        .read()
        .clone()
        .apply_event(GuestJoinEvent::SubmitAnswer);
    if matches!(next, GuestJoinState::Failed { .. }) {
        state.set(next);
        return;
    }
    let Some(question_id) = next.question().map(|question| question.id.clone()) else {
        state.set(next);
        return;
    };
    let choices = next.selected_choices();
    if choices.is_empty() {
        state.set(GuestJoinState::Failed {
            reason: "sélection invalide".into(),
        });
        return;
    }
    let sent = send_join_message(&ClientMessage::SubmitAnswer {
        question_id,
        choices,
    });
    if sent {
        state.set(next);
    } else {
        state.set(GuestJoinState::Failed {
            reason: "réseau indisponible".into(),
        });
    }
}

#[cfg(target_arch = "wasm32")]
fn send_join_message(message: &ClientMessage) -> bool {
    let payload = match serde_json::to_string(message) {
        Ok(payload) => payload,
        Err(_) => return false,
    };
    let mut sent = false;
    JOIN_CONNECTION.with(|slot| {
        if let Some(connection) = slot.borrow().as_ref() {
            sent = connection.ws.send_with_str(&payload).is_ok();
        }
    });
    sent
}

#[cfg(target_arch = "wasm32")]
fn validate_session_code(session_id: &str) -> bool {
    matches!(session_id.len(), 6 | 12)
        && session_id
            .bytes()
            .all(|byte| b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789".contains(&byte))
}

#[cfg(target_arch = "wasm32")]
fn websocket_url(session_id: &str, token: &str) -> String {
    let window = web_sys::window().expect("window");
    let location = window.location();
    let protocol = location.protocol().unwrap_or_default();
    let scheme = if protocol == "https:" { "wss" } else { "ws" };
    let host = location.host().unwrap_or_default();
    let token = urlencoding::encode(token);
    format!("{scheme}://{host}/ws/{session_id}?token={token}")
}
