//! rumble-lm-ui — mobile-first product UI primitives for Rumble LM.
//!
//! This crate is intentionally small and sovereign: no remote component service,
//! no CDN, and no business logic. Shared client-platform semantics belong to
//! Portal; product state lives in `presto-core`; this crate renders accessible
//! LM-specific primitives over shared DTOs.

#![allow(non_snake_case)]

use dioxus::prelude::*;
use dioxus_primitives::label::Label;
use presto_core::api::SourceCitation;

// Dioxus Primitives: headless ARIA-compliant components, imported by module
// (label, dialog, collapsible, tabs, …). TextInput consumes `label::Label`
// (I1 2026-07-10). Button/Card remain custom: no primitive equivalents at the
// pinned rev (Radix philosophy — no primitives for trivially-native
// elements). Dialog and Toast stay custom too: see the notes on each
// component for the verified reasons.

/// CSS custom properties: colors, spacing, radius, typography, motion, and safe areas.
pub const TOKENS_CSS: &str = include_str!("tokens.css");

/// Compatibility bridge from generated Portal token names to product-local variables.
pub const PORTAL_BRIDGE_CSS: &str = include_str!("portal-bridge.css");

/// Component classes built exclusively on top of token variables.
pub const COMPONENTS_CSS: &str = include_str!("components.css");

/// Include once near the root of a Dioxus app.
#[component]
pub fn ThemeStyles() -> Element {
    rsx! {
        style { "{TOKENS_CSS}\n{PORTAL_BRIDGE_CSS}\n{COMPONENTS_CSS}" }
    }
}

/// A mobile app surface that applies safe-area padding and base typography.
#[component]
pub fn AppSurface(children: Element) -> Element {
    rsx! {
        main { class: "presto-app-surface", {children} }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonVariant {
    #[default]
    Primary,
    Secondary,
    Ghost,
}

impl ButtonVariant {
    fn class(self) -> &'static str {
        match self {
            Self::Primary => "presto-button presto-button--primary",
            Self::Secondary => "presto-button presto-button--secondary",
            Self::Ghost => "presto-button presto-button--ghost",
        }
    }
}

#[component]
pub fn Button(
    label: String,
    #[props(default)] variant: ButtonVariant,
    #[props(default)] disabled: bool,
    #[props(optional)] aria_label: Option<String>,
) -> Element {
    let aria_label = aria_label.unwrap_or_else(|| label.clone());
    rsx! {
        button {
            class: variant.class(),
            disabled,
            aria_label: "{aria_label}",
            "{label}"
        }
    }
}

#[component]
pub fn TextInput(
    id: String,
    label: String,
    #[props(default)] value: String,
    #[props(default)] placeholder: String,
    #[props(default = String::from("text"))] input_type: String,
    #[props(optional)] help: Option<String>,
) -> Element {
    let described_by = help.as_ref().map(|_| format!("{id}-help"));
    rsx! {
        div { class: "presto-field",
            Label { class: "presto-label", html_for: "{id}", "{label}" }
            input {
                class: "presto-input",
                id: "{id}",
                r#type: "{input_type}",
                value: "{value}",
                placeholder: "{placeholder}",
                aria_describedby: described_by.unwrap_or_default(),
            }
            if let Some(help) = help {
                p { class: "presto-help", id: "{id}-help", "{help}" }
            }
        }
    }
}

#[component]
pub fn Card(
    title: String,
    #[props(default)] body: String,
    #[props(default)] children: Element,
) -> Element {
    rsx! {
        article { class: "presto-card",
            h2 { class: "presto-card__title", "{title}" }
            if !body.is_empty() {
                div { class: "presto-card__body", "{body}" }
            }
            {children}
        }
    }
}

/// Dialog stays custom — `dioxus_primitives::dialog` cannot render in static
/// SSR at the pinned rev (verified 2026-07-10): `DialogRoot` gates its
/// children behind `use_animated_open`, whose `show_in_dom` signal starts
/// `false` and only flips inside a `use_effect` (never run by one-shot
/// `dioxus_ssr::render`), and `DialogCtx` has private fields so the context
/// cannot be provided manually to render `DialogContent` directly.
#[component]
pub fn Dialog(title: String, children: Element) -> Element {
    let title_id = format!("presto-dialog-{}", stable_id(&title));
    rsx! {
        div { class: "presto-dialog-backdrop",
            section {
                class: "presto-dialog",
                role: "dialog",
                aria_modal: "true",
                aria_labelledby: "{title_id}",
                h2 { id: "{title_id}", class: "presto-card__title", "{title}" }
                {children}
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToastTone {
    #[default]
    Info,
    Success,
    Warning,
    Danger,
}

impl ToastTone {
    fn class(self) -> &'static str {
        match self {
            Self::Info => "presto-toast",
            Self::Success => "presto-toast presto-toast--success",
            Self::Warning => "presto-toast presto-toast--warning",
            Self::Danger => "presto-toast presto-toast--danger",
        }
    }
}

/// Toast (alert) component for live region announcements.
/// Deliberately NOT migrated to `dioxus_primitives::toast` (2026-07-10):
/// that module is a client-side toast *system* (provider + stack + imperative
/// push API), while this component is a static SSR live region
/// (`role="status"`, `aria-live`). Adopting the provider would add client
/// state for zero accessibility gain here.
#[component]
pub fn Toast(message: String, #[props(default)] tone: ToastTone) -> Element {
    rsx! {
        aside { class: tone.class(), role: "status", aria_live: "polite", "{message}" }
    }
}

#[component]
pub fn SourceCard(citation: SourceCitation) -> Element {
    let title = citation
        .title
        .clone()
        .or(citation.document_id.clone())
        .unwrap_or_else(|| citation.source_section_id.clone());
    rsx! {
        article { class: "presto-card presto-source-card",
            span { class: "presto-source-card__badge", "grounded source" }
            h2 { class: "presto-card__title", "{title}" }
            p { class: "presto-help", "{citation.source_section_id}" }
            if let Some(excerpt) = citation.excerpt {
                blockquote { class: "presto-card__body", "{excerpt}" }
            }
        }
    }
}

/// One item in the mobile bottom navigation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavItem {
    pub label: String,
    pub href: String,
    pub current: bool,
}

impl NavItem {
    pub fn new(label: impl Into<String>, href: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            href: href.into(),
            current: false,
        }
    }

    pub fn current(mut self) -> Self {
        self.current = true;
        self
    }
}

#[component]
pub fn BottomNav(
    items: Vec<NavItem>,
    #[props(default = String::from("Primary"))] label: String,
) -> Element {
    let count = items.len().max(1);
    let style = format!("--presto-bottom-nav-count: {count};");
    rsx! {
        nav { class: "presto-bottom-nav", aria_label: "{label}", style: "{style}",
            for item in items {
                a {
                    class: "presto-bottom-nav__link",
                    href: "{item.href}",
                    aria_current: if item.current { "page" } else { "false" },
                    "{item.label}"
                }
            }
        }
    }
}

/// A compact demo fragment for `/app/demo` or snapshot-style checks.
#[component]
pub fn MobileDemo() -> Element {
    let citation = SourceCitation {
        source_section_id: "demo#p0".to_string(),
        document_id: Some("demo".to_string()),
        title: Some("Demo document".to_string()),
        excerpt: Some("Every answer is rendered with a grounded citation.".to_string()),
    };
    rsx! {
        AppSurface {
            ThemeStyles {}
            div { class: "presto-stack",
                Card { title: "Presto notebook".to_string(), body: "Mobile-first Rust UI primitives.".to_string() }
                TextInput { id: "query".to_string(), label: "Ask your corpus".to_string(), placeholder: "What should I learn?".to_string(), help: "Answers are grounded before they are shown.".to_string() }
                Button { label: "Ask".to_string() }
                SourceCard { citation }
                Toast { message: "Grounding verified".to_string(), tone: ToastTone::Success }
            }
            BottomNav { items: vec![
                NavItem::new("Home", "/app").current(),
                NavItem::new("Corpus", "/app/corpus"),
                NavItem::new("Settings", "/app/settings"),
            ] }
        }
    }
}

fn stable_id(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(element: Element) -> String {
        dioxus_ssr::render_element(element)
    }

    #[test]
    fn theme_styles_include_tokens_and_reduced_motion() {
        let html = render(rsx! { ThemeStyles {} });
        assert!(html.contains("--presto-color-primary"));
        assert!(html.contains("--presto-touch-target: 44px"));
        assert!(html.contains("prefers-reduced-motion"));
        assert!(html.contains("--presto-color-primary: var(--color-brand"));
        assert!(
            !COMPONENTS_CSS.contains("#"),
            "component CSS must use token colors only"
        );
    }

    #[test]
    fn portal_fixture_provides_tokens_consumed_by_bridge() {
        const PORTAL_TOKENS: &str = include_str!("../fixtures/portal/tokens.css");
        const PORTAL_CONTRAST_REPORT: &str =
            include_str!("../fixtures/portal/contrast-report.json");

        for token in [
            "--color-background:",
            "--color-surface:",
            "--color-text:",
            "--color-brand:",
            "--spacing-md:",
            "--radius-md:",
            "--font-family-body:",
        ] {
            assert!(
                PORTAL_TOKENS.contains(token),
                "missing Portal token {token}"
            );
        }

        for mapping in [
            "var(--color-background",
            "var(--color-surface",
            "var(--color-text",
            "var(--color-brand",
            "var(--spacing-md",
            "var(--radius-md",
            "var(--font-family-body",
        ] {
            assert!(
                PORTAL_BRIDGE_CSS.contains(mapping),
                "bridge does not consume {mapping}"
            );
        }

        assert!(PORTAL_CONTRAST_REPORT.contains("portal.contrast_report.v0.1"));
        assert!(PORTAL_CONTRAST_REPORT.contains("\"passes_wcag_aa\": true"));
    }

    #[test]
    fn button_renders_accessible_touch_target_class() {
        let html = render(rsx! { Button { label: "Ask".to_string() } });
        assert!(html.contains("presto-button--primary"));
        assert!(html.contains("aria-label=\"Ask\""));
        assert!(COMPONENTS_CSS.contains("min-height: var(--presto-touch-target)"));
    }

    #[test]
    fn text_input_links_label_and_help() {
        let html = render(rsx! {
            TextInput {
                id: "query".to_string(),
                label: "Question".to_string(),
                help: "Use a grounded query".to_string(),
            }
        });
        assert!(html.contains("for=\"query\""));
        assert!(html.contains("aria-describedby=\"query-help\""));
        assert!(html.contains("Use a grounded query"));
    }

    #[test]
    fn dialog_has_modal_a11y_attributes() {
        let html = render(
            rsx! { Dialog { title: "Confirm".to_string(), Button { label: "Close".to_string() } } },
        );
        assert!(html.contains("role=\"dialog\""));
        assert!(html.contains("aria-modal=\"true\""));
        assert!(html.contains("aria-labelledby=\"presto-dialog-confirm\""));
        assert!(html.contains("id=\"presto-dialog-confirm\""));
    }

    #[test]
    fn text_input_label_links_to_input_via_primitive() {
        let html = render(rsx! { TextInput {
            id: "email".to_string(),
            label: "Email".to_string(),
        } });
        // The Primitives Label must render a real <label for=…> linked to the input.
        assert!(html.contains("<label"));
        assert!(html.contains("for=\"email\""));
        assert!(html.contains("id=\"email\""));
        assert!(html.contains("Email"));
    }

    #[test]
    fn source_card_renders_citation_without_server_dependency() {
        let html = render(rsx! {
            SourceCard { citation: SourceCitation {
                source_section_id: "doc#p0".to_string(),
                document_id: Some("doc".to_string()),
                title: Some("Doc".to_string()),
                excerpt: Some("A cited excerpt".to_string()),
            } }
        });
        assert!(html.contains("grounded source"));
        assert!(html.contains("doc#p0"));
        assert!(html.contains("A cited excerpt"));
    }

    #[test]
    fn bottom_nav_marks_current_page() {
        let html = render(rsx! {
            BottomNav { items: vec![
                NavItem::new("Home", "/app").current(),
                NavItem::new("Corpus", "/app/corpus"),
            ] }
        });
        assert!(html.contains("aria-label=\"Primary\""));
        assert!(html.contains("aria-current=\"page\""));
        assert!(html.contains("--presto-bottom-nav-count: 2"));
    }

    #[test]
    fn demo_renders_mobile_primitives() {
        let html = render(rsx! { MobileDemo {} });
        assert!(html.contains("Presto notebook"));
        assert!(html.contains("Ask your corpus"));
        assert!(html.contains("Grounding verified"));
        assert!(html.contains("presto-bottom-nav"));
    }
}
