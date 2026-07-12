//! Minimal Dioxus owner shell for Rumble LM.
//!
//! The server-owned OIDC redirect, notebook API and logout are wired without
//! exposing the HttpOnly session to WASM. Corpus operations remain a later
//! server-owned increment.

#![allow(non_snake_case)]

use dioxus::prelude::*;
#[cfg(target_arch = "wasm32")]
use presto_core::api::ApiEnvelope;
use presto_core::api::{CurrentSpace, RagQueryRequest, RagQueryResponse};
use presto_core::client::RagQueryState;
use rumble_lm_ui::{AppSurface, BottomNav, Card, NavItem, SourceCard, ThemeStyles};

pub const OWNER_STYLES: &str = include_str!("owner.css");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Home,
    Notebook,
    Corpus,
    Settings,
}

impl Screen {
    const fn href(self) -> &'static str {
        match self {
            Self::Home => "/app",
            Self::Notebook => "/app/notebook",
            Self::Corpus => "/app/corpus",
            Self::Settings => "/app/settings",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Routable)]
pub enum Route {
    #[route("/")]
    Home {},
    #[route("/login")]
    Login {},
    #[route("/notebook")]
    Notebook {},
    #[route("/corpus")]
    Corpus {},
    #[route("/settings")]
    Settings {},
}

#[component]
pub fn App() -> Element {
    rsx! { Router::<Route> {} }
}

fn session_notice() -> &'static str {
    "La session owner reste dans un cookie HttpOnly et les droits sont vérifiés par le serveur."
}

fn navigation(current: Screen) -> Vec<NavItem> {
    [
        (Screen::Home, "Accueil"),
        (Screen::Notebook, "Chat RAG"),
        (Screen::Corpus, "Corpus"),
        (Screen::Settings, "Réglages"),
    ]
    .into_iter()
    .map(|(screen, label)| {
        let item = NavItem::new(label, screen.href());
        if screen == current {
            item.current()
        } else {
            item
        }
    })
    .collect()
}

#[component]
fn OwnerFrame(current: Screen, children: Element) -> Element {
    rsx! {
        AppSurface {
            ThemeStyles {}
            style { "{OWNER_STYLES}" }
            div { class: "owner-shell",
                header { class: "owner-header",
                    div {
                        p { class: "owner-eyebrow", "Rumble LM" }
                        p { class: "owner-brand", "Espace owner" }
                    }
                    span { class: "owner-status", "Shell local" }
                }
                div { class: "owner-content", {children} }
                BottomNav {
                    items: navigation(current),
                    label: "Navigation owner".to_string(),
                }
            }
        }
    }
}

#[component]
pub fn Home() -> Element {
    rsx! {
        OwnerFrame { current: Screen::Home,
            section { class: "owner-page", aria_labelledby: "home-title",
                div { class: "owner-hero",
                    p { class: "owner-kicker", "Notebook personnel" }
                    h1 { id: "home-title", "Travaillez depuis vos propres sources." }
                    p { class: "owner-lede", "Connectez-vous avec l’IdP souverain pour retrouver l’espace personnel créé par le serveur." }
                }
                aside { class: "owner-session-note", role: "status", aria_live: "polite",
                    "{session_notice()}"
                }
                div { class: "owner-grid",
                    Card {
                        title: "Poser une question".to_string(),
                        body: "L’interface RAG est visible, mais aucun appel ni résultat n’est produit dans ce lot.".to_string(),
                        a { class: "owner-card-link", href: Screen::Notebook.href(), "Ouvrir le Chat RAG" }
                    }
                    Card {
                        title: "Préparer le corpus".to_string(),
                        body: "Le listage et l’ajout de documents arriveront avec l’API owner isolée.".to_string(),
                        a { class: "owner-card-link", href: Screen::Corpus.href(), "Voir le Corpus" }
                    }
                }
                a { class: "presto-button presto-button--primary owner-login-link", href: "/app/login", "Accéder à la connexion" }
            }
        }
    }
}

#[component]
pub fn Login() -> Element {
    rsx! {
        AppSurface {
            ThemeStyles {}
            style { "{OWNER_STYLES}" }
            section { class: "owner-login", aria_labelledby: "login-title",
                p { class: "owner-eyebrow", "Rumble LM · espace owner" }
                h1 { id: "login-title", "Connexion" }
                p { class: "owner-lede", "La connexion utilise OIDC Authorization Code + PKCE. Aucun jeton ni secret n’est lisible par cette application." }
                a { class: "presto-button presto-button--primary", href: "/auth/login", "Continuer vers la connexion" }
                p { class: "presto-help", "Après validation, le serveur revient ici avec une session HttpOnly révocable." }
                a { class: "owner-text-link", href: "/app", "Revenir à l’espace owner" }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum NotebookSession {
    Loading,
    Ready(CurrentSpace),
    Expired,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NetworkFailure {
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    SessionExpired,
    Unavailable,
}

#[cfg(target_arch = "wasm32")]
async fn load_current_space() -> Result<CurrentSpace, NetworkFailure> {
    use gloo_net::http::Request;

    let response = Request::get("/api/spaces/current")
        .send()
        .await
        .map_err(|_| NetworkFailure::Unavailable)?;
    if response.status() == 401 {
        return Err(NetworkFailure::SessionExpired);
    }
    if !response.ok() {
        return Err(NetworkFailure::Unavailable);
    }
    response
        .json::<ApiEnvelope<CurrentSpace>>()
        .await
        .map(|envelope| envelope.data)
        .map_err(|_| NetworkFailure::Unavailable)
}

#[cfg(not(target_arch = "wasm32"))]
async fn load_current_space() -> Result<CurrentSpace, NetworkFailure> {
    Err(NetworkFailure::Unavailable)
}

#[cfg(target_arch = "wasm32")]
async fn submit_rag_query(request: &RagQueryRequest) -> Result<RagQueryResponse, NetworkFailure> {
    use gloo_net::http::Request;

    let request = Request::post("/api/rag/query")
        .json(request)
        .map_err(|_| NetworkFailure::Unavailable)?;
    let response = request
        .send()
        .await
        .map_err(|_| NetworkFailure::Unavailable)?;
    if response.status() == 401 {
        return Err(NetworkFailure::SessionExpired);
    }
    if !response.ok() {
        return Err(NetworkFailure::Unavailable);
    }
    response
        .json::<ApiEnvelope<RagQueryResponse>>()
        .await
        .map(|envelope| envelope.data)
        .map_err(|_| NetworkFailure::Unavailable)
}

#[cfg(not(target_arch = "wasm32"))]
async fn submit_rag_query(_request: &RagQueryRequest) -> Result<RagQueryResponse, NetworkFailure> {
    Err(NetworkFailure::Unavailable)
}

#[component]
pub fn Notebook() -> Element {
    let mut session = use_signal(|| NotebookSession::Loading);
    let mut query = use_signal(String::new);
    let mut rag_state = use_signal(|| RagQueryState::Idle);
    let mut reload_space = use_signal(|| 0_u32);

    use_effect(move || {
        let _reload_generation = *reload_space.read();
        spawn(async move {
            session.set(match load_current_space().await {
                Ok(space) => NotebookSession::Ready(space),
                Err(NetworkFailure::SessionExpired) => NotebookSession::Expired,
                Err(NetworkFailure::Unavailable) => NotebookSession::Failed,
            });
        });
    });

    let current_session = session.read().clone();
    let current_rag_state = rag_state.read().clone();
    let can_edit =
        matches!(current_session, NotebookSession::Ready(_)) && !current_rag_state.is_loading();
    let can_submit = can_edit && !query.read().trim().is_empty();

    rsx! {
        OwnerFrame { current: Screen::Notebook,
            section { class: "owner-page owner-page--chat", aria_labelledby: "notebook-title",
                div {
                    p { class: "owner-kicker", "Chat RAG" }
                    h1 { id: "notebook-title", "Interroger votre corpus" }
                    p { class: "owner-lede", "Les réponses publiées proviennent uniquement du registre serveur de claims approuvés pour votre espace et votre clearance." }
                }
                div { class: "owner-conversation", aria_live: "polite",
                    match current_session {
                        NotebookSession::Loading => rsx! {
                            div { class: "owner-empty", role: "status", h2 { "Chargement de votre espace…" } }
                        },
                        NotebookSession::Expired => rsx! {
                            div { class: "owner-empty owner-result--failure", role: "alert",
                                h2 { "Session expirée" }
                                p { "Reconnectez-vous pour interroger votre espace personnel." }
                                a { class: "owner-text-link", href: "/app/login", "Se reconnecter" }
                            }
                        },
                        NotebookSession::Failed => rsx! {
                            div { class: "owner-empty owner-result--failure", role: "alert",
                                h2 { "Espace indisponible" }
                                p { "Le service est temporairement indisponible." }
                                button {
                                    class: "presto-button presto-button--secondary",
                                    r#type: "button",
                                    onclick: move |_| {
                                        session.set(NotebookSession::Loading);
                                        reload_space += 1;
                                    },
                                    "Réessayer le chargement"
                                }
                            }
                        },
                        NotebookSession::Ready(_) => match current_rag_state {
                            RagQueryState::Idle | RagQueryState::Draft { .. } => rsx! {
                                div { class: "owner-empty", role: "status",
                                    h2 { "Prêt à interroger les claims approuvés" }
                                    p { "Essayez : « Quelle est la capitale de la France ? »" }
                                }
                            },
                            RagQueryState::Loading { .. } => rsx! {
                                div { class: "owner-empty", role: "status", h2 { "Recherche en cours…" } }
                            },
                            RagQueryState::Grounded { answer, citations, .. } => rsx! {
                                article { class: "owner-result owner-result--grounded",
                                    p { class: "owner-kicker", "Claim approuvé" }
                                    h2 { "Réponse" }
                                    p { class: "owner-answer", "{answer}" }
                                    section { class: "owner-citations", aria_label: "Citations approuvées",
                                        h3 { "Sources" }
                                        for citation in citations {
                                            SourceCard { citation }
                                        }
                                    }
                                }
                            },
                            RagQueryState::Rejected { .. } => rsx! {
                                div { class: "owner-empty owner-result--rejected", role: "status",
                                    h2 { "Réponse rejetée" }
                                    p { "Aucun claim approuvé ne correspond exactement à cette question dans votre espace." }
                                }
                            },
                            RagQueryState::Failed { .. } => rsx! {
                                div { class: "owner-empty owner-result--failure", role: "alert",
                                    h2 { "Requête impossible" }
                                    p { "Le service est temporairement indisponible. Réessayez plus tard." }
                                }
                            },
                        },
                    }
                }
                form {
                    class: "owner-query",
                    onsubmit: move |event| {
                        event.prevent_default();
                        let NotebookSession::Ready(current) = session.read().clone() else {
                            return;
                        };
                        let Ok(loading) = RagQueryState::submit(query.read().as_str()) else {
                            rag_state.set(RagQueryState::Failed {
                                query: String::new(),
                                message: "invalid_query".to_string(),
                            });
                            return;
                        };
                        let submitted_query = loading.query().unwrap_or_default().to_string();
                        rag_state.set(loading);
                        spawn(async move {
                            let request = RagQueryRequest {
                                space_id: current.space.id,
                                query: submitted_query,
                                max_sources: Some(3),
                            };
                            match submit_rag_query(&request).await {
                                Ok(response) => {
                                    let state = rag_state.read().clone().apply_response(response);
                                    rag_state.set(state);
                                }
                                Err(NetworkFailure::SessionExpired) => {
                                    session.set(NotebookSession::Expired);
                                    rag_state.set(RagQueryState::Idle);
                                }
                                Err(NetworkFailure::Unavailable) => {
                                    let state = rag_state
                                        .read()
                                        .clone()
                                        .fail("service_unavailable");
                                    rag_state.set(state);
                                }
                            }
                        });
                    },
                    label { class: "presto-label", r#for: "owner-query", "Question au corpus" }
                    textarea {
                        class: "presto-input owner-query__input",
                        id: "owner-query",
                        name: "query",
                        rows: "2",
                        maxlength: "4096",
                        disabled: !can_edit,
                        aria_describedby: "owner-query-help",
                        placeholder: "Quelle est la capitale de la France ?",
                        value: "{query}",
                        oninput: move |event| {
                            let value = event.value();
                            query.set(value.clone());
                            rag_state.set(RagQueryState::edit(value));
                        },
                    }
                    p { class: "presto-help", id: "owner-query-help", "4 096 caractères maximum. L’espace et la clearance sont imposés par le serveur." }
                    button { class: "presto-button presto-button--primary", r#type: "submit", disabled: !can_submit, "Envoyer" }
                }
            }
        }
    }
}

#[component]
pub fn Corpus() -> Element {
    rsx! {
        OwnerFrame { current: Screen::Corpus,
            section { class: "owner-page", aria_labelledby: "corpus-title",
                div {
                    p { class: "owner-kicker", "Sources" }
                    h1 { id: "corpus-title", "Corpus" }
                    p { class: "owner-lede", "La liste et l’ajout de documents owner ne sont pas implémentés dans ce shell." }
                }
                div { class: "owner-empty", role: "status",
                    h2 { "Aucun corpus chargé" }
                    p { "Cet état signifie “non chargé sans authentification”, pas “votre corpus est vide”." }
                }
                button { class: "presto-button presto-button--secondary", disabled: true, "Ajouter un document" }
            }
        }
    }
}

#[component]
pub fn Settings() -> Element {
    rsx! {
        OwnerFrame { current: Screen::Settings,
            section { class: "owner-page", aria_labelledby: "settings-title",
                div {
                    p { class: "owner-kicker", "Préférences" }
                    h1 { id: "settings-title", "Réglages" }
                    p { class: "owner-lede", "Aucun réglage n’est persisté dans le navigateur par ce shell." }
                }
                Card {
                    title: "Session".to_string(),
                    body: "La session est gérée côté serveur ; aucun token n’est exposé au client Rust/WASM.".to_string(),
                    form { method: "post", action: "/auth/logout",
                        button { class: "owner-card-link", r#type: "submit", "Se déconnecter" }
                    }
                }
                Card {
                    title: "Stockage local".to_string(),
                    body: "Désactivé : ni localStorage, ni sessionStorage, ni service worker.".to_string()
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(element: Element) -> String {
        dioxus_ssr::render_element(element)
    }

    #[test]
    fn all_owner_screens_render_honest_placeholder_states() {
        let home = render(rsx! { Home {} });
        let login = render(rsx! { Login {} });
        let notebook = render(rsx! { Notebook {} });
        let corpus = render(rsx! { Corpus {} });
        let settings = render(rsx! { Settings {} });

        assert!(home.contains("cookie HttpOnly"));
        assert!(login.contains("Authorization Code + PKCE"));
        assert!(notebook.contains("Chargement de votre espace"));
        assert!(notebook.contains("4 096 caractères maximum"));
        assert!(corpus.contains("non chargé sans authentification"));
        assert!(settings.contains("Aucun réglage n’est persisté"));
        assert!(settings.contains("action=\"/auth/logout\""));
        assert!(!notebook.contains("Paris est la capitale"));
    }

    #[test]
    fn mobile_navigation_marks_the_current_route_and_uses_app_prefix() {
        let html = render(rsx! { Corpus {} });
        assert!(html.contains("aria-label=\"Navigation owner\""));
        assert!(html.contains("href=\"/app/notebook\""));
        assert!(html.contains("href=\"/app/corpus\" aria-current=\"page\""));
        assert!(html.contains("href=\"/app/settings\""));
    }

    #[test]
    fn query_input_is_labelled_disabled_and_sticky() {
        let html = render(rsx! { Notebook {} });
        assert!(html.contains("for=\"owner-query\""));
        assert!(html.contains("aria-describedby=\"owner-query-help\""));
        assert!(html.contains("maxlength=\"4096\""));
        assert!(html.contains(" disabled"));
        assert!(OWNER_STYLES.contains("position: sticky"));
    }

    #[test]
    fn app_styles_use_portal_tokens_and_no_raw_colors() {
        assert!(OWNER_STYLES.contains("var(--presto-color-bg)"));
        assert!(!OWNER_STYLES.contains('#'));
    }
}
