# Development Keycloak

Pinned local OIDC provider for the owner mobile E2E. The imported realm enables only Authorization Code, requires PKCE S256, uses a public client (there is no client secret), and accepts only `http://localhost:3000/auth/callback`.

```bash
./scripts/keycloak-dev.sh up
```

The script generates random bootstrap/test passwords in ignored `dev/keycloak/.env` with permissions 0600. Neither generated credentials nor Keycloak data are committed. The realm JSON contains only an environment placeholder, never a password.

See [`docs/e2e-testing.md`](../../docs/e2e-testing.md) for server variables and the mobile Playwright gate. `reset` destroys the local container state and generated credentials:

```bash
./scripts/keycloak-dev.sh reset
```
