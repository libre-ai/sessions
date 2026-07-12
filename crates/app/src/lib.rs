//! Minimal Dioxus owner shell for Rumble LM.
//!
//! This crate renders navigation and honest unavailable states only. Authentication,
//! corpus operations, and grounded RAG remain server-owned follow-up increments.

#![allow(non_snake_case)]

use dioxus::prelude::*;
use presto_core::client::AuthSessionState;
use rumble_lm_ui::{AppSurface, BottomNav, Card, NavItem, ThemeStyles};

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

fn anonymous_notice() -> &'static str {
    let auth = AuthSessionState::Anonymous;
    match auth {
        AuthSessionState::Anonymous => {
            "Vous consultez un shell sans session. Aucune donnée owner n’est chargée."
        }
        AuthSessionState::Unknown
        | AuthSessionState::Authenticated { .. }
        | AuthSessionState::Expired { .. } => unreachable!("the #31 shell starts anonymous"),
    }
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
                    p { class: "owner-lede", "Ce shell prépare le parcours mobile owner sans simuler de compte, de document ou de réponse grounded." }
                }
                aside { class: "owner-session-note", role: "status", aria_live: "polite",
                    "{anonymous_notice()}"
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
                p { class: "owner-lede", "Aucune session durable n’est créée par ce shell. Le flux OIDC et son cookie HttpOnly seront livrés séparément." }
                a { class: "presto-button presto-button--primary", href: "/auth/login", "Continuer vers la connexion" }
                p { class: "presto-help", "Le point d’entrée /auth/login est une couture non fonctionnelle réservée au lot d’authentification." }
                a { class: "owner-text-link", href: "/app", "Revenir au shell sans session" }
            }
        }
    }
}

#[component]
pub fn Notebook() -> Element {
    rsx! {
        OwnerFrame { current: Screen::Notebook,
            section { class: "owner-page owner-page--chat", aria_labelledby: "notebook-title",
                div {
                    p { class: "owner-kicker", "Chat RAG" }
                    h1 { id: "notebook-title", "Interroger votre corpus" }
                    p { class: "owner-lede", "Aucune réponse de démonstration n’est injectée : le verifier et les citations seront branchés dans un lot ultérieur." }
                }
                div { class: "owner-empty", role: "status",
                    h2 { "Conversation indisponible sans session" }
                    p { "Connectez-vous lorsque l’authentification sera disponible pour charger un espace autorisé." }
                }
                form { class: "owner-query", onsubmit: move |event| event.prevent_default(),
                    label { class: "presto-label", r#for: "owner-query", "Question au corpus" }
                    textarea {
                        class: "presto-input owner-query__input",
                        id: "owner-query",
                        name: "query",
                        rows: "2",
                        disabled: true,
                        aria_describedby: "owner-query-help",
                        placeholder: "Connexion requise pour interroger vos sources",
                    }
                    p { class: "presto-help", id: "owner-query-help", "Aucune requête réseau n’est envoyée par ce shell." }
                    button { class: "presto-button presto-button--primary", r#type: "submit", disabled: true, "Envoyer" }
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
                    body: "Déconnecté · aucun token exposé au client Rust/WASM.".to_string(),
                    a { class: "owner-card-link", href: "/app/login", "Voir la couture de connexion" }
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

        assert!(home.contains("sans session"));
        assert!(login.contains("Aucune session durable"));
        assert!(notebook.contains("Aucune requête réseau"));
        assert!(corpus.contains("non chargé sans authentification"));
        assert!(settings.contains("Aucun réglage n’est persisté"));
        assert!(!notebook.contains("grounded source"));
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
        assert!(html.contains(" disabled"));
        assert!(OWNER_STYLES.contains("position: sticky"));
    }

    #[test]
    fn app_styles_use_portal_tokens_and_no_raw_colors() {
        assert!(OWNER_STYLES.contains("var(--presto-color-bg)"));
        assert!(!OWNER_STYLES.contains('#'));
    }
}
