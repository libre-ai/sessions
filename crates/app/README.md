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

## Limites intentionnelles

Ce lot ne fournit ni OIDC, ni cookie/session, ni appel RAG, ni API corpus, ni service worker/PWA. `/app/login` est une couture d’interface vers le futur `/auth/login`; aucune session durable ou authentification fictive n’est créée. Le client ne lit ni token ni stockage web et ne dépend pas de `presto-server` comme bibliothèque.
