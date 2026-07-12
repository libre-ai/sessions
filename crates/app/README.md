# rumble-lm-app

Shell Dioxus 0.7 mobile-first de l’espace owner, servi sous `/app` par `presto-server`.

## Construire le bundle web

Le dépôt épingle Dioxus à la série `0.7` (`0.7.9` dans `Cargo.lock`). Installer la CLI correspondante puis régénérer les assets embarqués :

```bash
cargo install dioxus-cli --version 0.7.9 --locked
./scripts/build-owner-app.sh
```

Le script exécute un build web release avec `base_path = "app"`, vérifie les références/no-CDN, puis copie le résultat généré dans `crates/server/static/owner-app/`. Ce répertoire est ignoré par Git : Dioxus embarque des chemins absolus de dépendances dans le WASM, donc le bundle n’est pas reproductible octet par octet entre machines. Il ne doit jamais être committé.

Le serveur embarque les octets présents dans ce répertoire. Sur un checkout propre, construire le bundle **avant** toute commande qui compile `presto-server` :

```bash
./scripts/build-owner-app.sh
cargo build --bin presto-server --release --locked
```

En CI, un job unique construit le bundle depuis le même checkout, le place dans un paquet avec `SHA256SUMS`, puis tous les jobs Rust, release et E2E vérifient et installent ce paquet avant de compiler le serveur.

## Vérifier

```bash
cargo check -p rumble-lm-app
cargo check -p rumble-lm-app --target wasm32-unknown-unknown
cargo test -p rumble-lm-app --all-features
./scripts/test-owner-app-package.sh
cd e2e && npx playwright test tests/owner-shell.spec.ts --project=chromium
```

## Authentification et limites

`/app/login` redirige vers le flux OIDC serveur, `/app/notebook` charge l’espace courant puis soumet `POST /api/rag/query` en same-origin, et `/app/settings` soumet le logout. Le client ne lit jamais le cookie HttpOnly, ne manipule aucun token, n’utilise aucun stockage web et ne dépend pas de `presto-server` comme bibliothèque. Les réponses/citations sont échappées par Dioxus, sans `innerHTML` ni `eval`, et les projections owner restent les DTO réseau de `presto-core`.

Le Notebook rend les états idle/loading/grounded/rejected/failure/session expirée. Le Corpus reste un placeholder pour #34 ; ce lot ne fournit ni upload owner, ni service worker/PWA. Les sessions owner et l’autorité membership restent process-locales : un redémarrage déconnecte et le multi-instance owner n’est pas supporté. Voir `docs/security/owner-web-auth.md`.
