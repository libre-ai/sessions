# Deploying Presto-Matic on Clever Cloud (sovereign, multi-instance)

The binary already selects its store/fanout/auth from the environment (see
`crates/server/src/main.rs`), so a multi-instance deployment is configuration —
no code change. This guide covers the Clever-specific bits.

## Prerequisites

- A Clever Cloud account and the [`clever` CLI](https://www.clever-cloud.com/developers/doc/cli/)
  (`clever login`).
- A **production** PostgreSQL plan (pgvector is **not** available on DEV plans).

## 1. Create the app (Rust)

```bash
clever create --type rust presto-matic --region par
# Select the workspace binary and cache deps for faster builds:
clever env set CC_RUST_BIN presto-server
clever env set CC_CACHE_DEPENDENCIES true
clever env set CC_PRE_BUILD_HOOK './scripts/clever-pre-build.sh'
```

Le hook fail-closed installe Dioxus CLI 0.7.9, construit le bundle owner depuis le checkout déployé et vérifie son paquet SHA-256 avant le build Rust. Les assets sont générés et ne sont volontairement pas suivis par Git. Ne retirez pas ce hook : le build serveur doit embarquer le bundle qu’il vient de produire, jamais un bundle committé obsolète.

Clever builds ensuite avec `cargo build --release --locked` (so `Cargo.lock` must be
committed — it is) and expects the app to listen on `0.0.0.0:8080`. `main.rs`
defaults `PORT` to 8080, so no extra config is needed.

## 2. Add-ons

### PostgreSQL + pgvector

```bash
clever addon create postgresql-addon --plan <production-plan> pm-postgres --region par
clever service link-addon pm-postgres
```

Then **open a Ticket Center request to enable the `pgvector` extension** on this
add-on (it is provided on demand, not self-serve, and not on DEV plans). The app
runs `CREATE EXTENSION IF NOT EXISTS vector;` on connect, which only succeeds
once support has enabled it.

### Redis

```bash
clever addon create redis-addon --plan <plan> pm-redis --region par
clever service link-addon pm-redis
```

## 3. Environment

Set the six runtime variables. The two add-on URIs come from each add-on's
dashboard / `clever env` after linking (Clever injects add-on variables under its
own names; copy the connection URIs into the names the app reads):

```bash
# Shared session state + fanout (required for multi-instance):
clever env set DATABASE_URL "<postgresql connection uri>"
clever env set REDIS_URL    "<redis connection uri>"

# Shared Biscuit key — MUST be identical across instances. Generate one:
#   cargo run -p presto-server -- keygen
clever env set BISCUIT_PRIVATE_KEY "<hex from keygen>"

# Hosted AI is Clever AI only. Values stay unset until the private contract,
# region, retention and endpoint reference have been approved.
clever env set CLEVER_AI_ENABLED "1"
clever env set CLEVER_AI_BASE_URL "<approved Clever AI HTTPS origin>"
clever env set CLEVER_AI_API_KEY "<secret managed outside Git>"
clever env set CLEVER_AI_CONTRACT_REF "<versioned non-secret contract reference>"
clever env set CLEVER_AI_EMBED_MODEL "<approved embedding model>"
clever env set CLEVER_AI_CHAT_MODEL "<approved chat model>"
```

> This is a configuration reference, not a provisioning instruction. Direct
> third-party hosted fallbacks are rejected by the provider policy.

## 4. Deploy + scale

```bash
git push clever feat/p7-clever-deploy:master   # or your default branch
clever scale --min-instances 2 --max-instances 2   # exercise Redis fanout + shared state
```

With Postgres + Redis + a shared Biscuit key set, two instances share session
state (Postgres), fan out live events (Redis), and accept each other's tokens
(shared key) — the multi-instance path proven by the gated integration tests.

## 5. Smoke test

```bash
scripts/clever-smoke.sh https://<your-app>.cleverapps.io
```

Checks `/health` and that `POST /sessions` returns a host token (which exercises
the Postgres write + Biscuit mint in production).

## Notes

- Serve over **HTTPS/WSS** (Clever terminates TLS): the WS join token rides the
  query string (browsers cannot set WS headers); TLS keeps it encrypted in
  transit. Do not enable access logging of WS URLs with query strings.
- `POST /sessions` is currently open (anyone can host). Add a rate-limit / host
  identity (OIDC/Keycloak) before a public launch.
