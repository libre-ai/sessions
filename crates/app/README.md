# rumble-lm-app

Shell Dioxus 0.7 mobile-first de l’espace owner, servi sous `/app` par `presto-server`.

## Construire le bundle web

Le dépôt épingle Dioxus à la série `0.7` (`0.7.9` dans `Cargo.lock`). Installer la CLI correspondante puis régénérer les assets embarqués :

```bash
cargo install dioxus-cli --version 0.7.9 --locked
./scripts/build-owner-app.sh
```

Le script exécute un build web release avec `base_path = "app"`, puis copie le résultat dans `crates/server/static/owner-app/`. Le serveur embarque ces fichiers dans son binaire et sert le document d’entrée pour `/app` et `/app/*`. Après toute modification du crate, régénérer le bundle avant de committer.

## Vérifier

```bash
cargo check -p rumble-lm-app
cargo check -p rumble-lm-app --target wasm32-unknown-unknown
cargo test -p rumble-lm-app --all-features
cd e2e && npx playwright test tests/owner-shell.spec.ts --project=chromium
```

## Limites intentionnelles

Ce lot ne fournit ni OIDC, ni cookie/session, ni appel RAG, ni API corpus, ni service worker/PWA. `/app/login` est une couture d’interface vers le futur `/auth/login`; aucune session durable ou authentification fictive n’est créée. Le client ne lit ni token ni stockage web et ne dépend pas de `presto-server` comme bibliothèque.
