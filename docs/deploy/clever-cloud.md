# Déploiement Presto-Matic sur Clever Cloud

Ce dépôt supporte deux topologies **incompatibles**. Choisissez-en une seule.

## Topologie A — owner RC single-instance

Usage : staging #109.

- OIDC réel configuré (`OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_REDIRECT_URI`)
- `OWNER_AUTH_SINGLE_INSTANCE=1`
- `BISCUIT_PRIVATE_KEY` partagé
- `INGEST_TOKEN` présent
- **exactement 1 instance**
- **sans** `DATABASE_URL`
- **sans** `REDIS_URL`
- pas d’auto-récupération au redémarrage : owner, corpus et état live sont perdus

Clever AI est **désactivé / non configuré** pour ce staging : aucun `CLEVER_AI_*`, aucun `LOCAL_AI_*`.

## Topologie B — anonymous live multi-instance

Usage : sessions live anonymes uniquement.

- owner auth désactivé (pas de tuple OIDC)
- `DATABASE_URL` requis
- `REDIS_URL` requis
- `BISCUIT_PRIVATE_KEY` requis
- `INGEST_TOKEN` requis
- plusieurs instances autorisées
- **ne prétend pas supporter owner**

## Pré-build

Le hook pré-build est obligatoire : il installe/pin le Dioxus CLI 0.7.9, construit **les deux bundles canoniques** depuis le checkout déployé (`owner-app` et `join-app`) puis les vérifie avant la compilation Rust.

```bash
clever env set CC_RUST_BIN presto-server
clever env set CC_CACHE_DEPENDENCIES true
clever env set CC_PRE_BUILD_HOOK './scripts/clever-pre-build.sh'
```

## Déploiement staging

exécuter via le garde local approuvé : `git -C "$(git rev-parse --show-toplevel)" push cc-staging origin/main:master`

- ne pas utiliser `clever deploy`
- prod est hors scope ici

## Rollback sûr

- noter le SHA `main` avant le push staging
- en cas d’échec, préparer et relire un revert sur `main`
- attendre que CI et security repassent au vert
- rejouer ensuite la même séquence de garde local approuvé puis le push staging
- pas de force-push, pas de rollback instantané, pas de `clever deploy`

## Variables pour A

```bash
clever env set OIDC_ISSUER "<https issuer>"
clever env set OIDC_CLIENT_ID "<client id>"
clever env set OIDC_REDIRECT_URI "https://<app>/auth/callback"
clever env set OWNER_AUTH_SINGLE_INSTANCE "1"
clever env set BISCUIT_PRIVATE_KEY "<opaque hex key>"
clever env set INGEST_TOKEN "<strong printable ASCII token>"
```

Ne définissez pas `DATABASE_URL` ni `REDIS_URL` sur A.

## Variables pour B

```bash
clever env set DATABASE_URL "<postgresql uri>"
clever env set REDIS_URL "<redis uri>"
clever env set BISCUIT_PRIVATE_KEY "<opaque hex key>"
clever env set INGEST_TOKEN "<strong printable ASCII token>"
```

Ne configurez pas le tuple OIDC pour B ; l’owner reste hors contrat.

## Smoke

```bash
scripts/clever-smoke.sh https://<your-app>.cleverapps.io
```

Le smoke vérifie `/health` puis `POST /sessions` sans afficher de JSON, de `host_token`, de join token ni de fragment d’URL.

## Notes

- servir en **HTTPS/WSS** ; `/ws/{session_id}` transporte le token dans la query string, donc ne jamais logger les queries `/ws`
- la preuve de politique de logs passe par la revue de configuration proxy/drain ; ne pas collecter/copier les logs bruts `/ws?...` comme preuve
- les secrets ne doivent jamais être copiés dans des exemples réalistes
- `POST /sessions` crée une session éphémère : le token et le lien de join ne doivent pas être persistés côté client
