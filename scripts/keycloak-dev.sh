#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dir="$root/dev/keycloak"
env_file="$dir/.env"
command="${1:-up}"
if docker compose version >/dev/null 2>&1; then
  compose=(docker compose)
elif command -v docker-compose >/dev/null 2>&1; then
  compose=(docker-compose)
else
  echo "Docker Compose is required" >&2
  exit 1
fi

make_secret() {
  python3 -c 'import secrets; print(secrets.token_urlsafe(32))'
}

ensure_env() {
  if [[ ! -f "$env_file" ]]; then
    umask 077
    cat >"$env_file" <<EOF
KEYCLOAK_BOOTSTRAP_ADMIN_USERNAME=admin
KEYCLOAK_BOOTSTRAP_ADMIN_PASSWORD=$(make_secret)
KEYCLOAK_TEST_USERNAME=owner
KEYCLOAK_TEST_PASSWORD=$(make_secret)
EOF
    echo "Generated untracked development credentials in dev/keycloak/.env" >&2
  fi
}

case "$command" in
  up)
    ensure_env
    "${compose[@]}" --env-file "$env_file" -f "$dir/compose.yml" up -d
    for _ in $(seq 1 60); do
      if curl --fail --silent \
        http://localhost:8081/realms/rumble-lm-dev/.well-known/openid-configuration \
        >/dev/null; then
        echo "Keycloak dev is ready on http://localhost:8081" >&2
        exit 0
      fi
      sleep 2
    done
    echo "Keycloak dev did not become ready" >&2
    exit 1
    ;;
  down)
    "${compose[@]}" --env-file "$env_file" -f "$dir/compose.yml" down
    ;;
  reset)
    "${compose[@]}" --env-file "$env_file" -f "$dir/compose.yml" down -v || true
    rm -f "$env_file"
    echo "Removed the generated development credentials" >&2
    ;;
  *)
    echo "usage: $0 {up|down|reset}" >&2
    exit 2
    ;;
esac
