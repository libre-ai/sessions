# rumble-lm-ui

Mobile-first Dioxus product UI primitives for `rumble-lm`.

> Rename note: this crate is now `rumble-lm-ui`. Historical docs may still mention `presto-ui`; shared design-system responsibility belongs to Portal.

## Scope

`rumble-lm-ui` renders only product-specific UI primitives for `rumble-lm`. Shared tokens, accessibility conventions, i18n UI, and native/web platform adapters belong to Portal. Product state, API contracts, and protocol transitions stay in `presto-core`; apps must not depend on `presto-server` as a Rust library.

`fixtures/portal/` contient le bundle Libre IA Design System 2.0 généré par `portal-forge`, son rapport de contraste et son manifest SHA-256. `src/portal-bridge.css` ne porte plus aucun fallback visuel : il mappe uniquement les noms historiques du composant vers les tokens sémantiques partagés.

## Components

- `ThemeStyles` — injects token + component CSS.
- `AppSurface` — mobile safe-area surface.
- `Button`
- `TextInput`
- `Card`
- `Dialog`
- `Toast`
- `SourceCard`
- `BottomNav`
- `MobileDemo` — compact demo fragment for smoke/snapshot rendering.

## Mobile/a11y constraints

- Touch targets currently use the legacy `--presto-touch-target: 44px` CSS variable; target migration is Portal token names.
- Focus uses `:focus-visible` with a tokenized focus color.
- Dialogs render `role="dialog"`, `aria-modal`, and `aria-labelledby`.
- Toasts render as polite status regions.
- No remote fonts, CDN, or component SaaS.
- Component CSS must use token variables for colors; raw color values live only in
  the generated `tokens.css`.
- Flat surfaces only: no gradient, glow, realistic shadow or remote font.
