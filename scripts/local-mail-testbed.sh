#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DRY_RUN=0
NO_BUILD=0
COMPOSE_PROVIDER=""

usage() {
  cat <<'USAGE'
Usage: scripts/local-mail-testbed.sh [--dry-run] [--no-build]

Run a local Stalwart + hail testbed and inject synthetic inbound mail fixtures.

Environment:
  HAIL_TESTBED_EMAIL       mailbox to import into (default: alice@hail.test)
  HAIL_TESTBED_PASSWORD    mailbox password (required for real import)
  HAIL_TESTBED_JMAP_URL    JMAP URL (default: http://127.0.0.1:18080)
  HAIL_TESTBED_HAIL_URL    hail URL printed in checks (default: http://127.0.0.1:18081)
  HAIL_TESTBED_COMPOSE     compose provider override: podman compose|podman-compose|docker compose
  HAIL_SERVER_KEY          hail server key; generated for local compose when unset

Options:
  --dry-run    Validate fixture plan and print commands/checks; do not build/start/import.
  --no-build   Skip podman/docker image build.
  -h, --help   Show this help.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --no-build) NO_BUILD=1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 1
  fi
}

choose_compose() {
  if [[ -n "${HAIL_TESTBED_COMPOSE:-}" ]]; then
    COMPOSE_PROVIDER="$HAIL_TESTBED_COMPOSE"
    return
  fi
  if command -v podman >/dev/null 2>&1 && podman compose version >/dev/null 2>&1; then
    COMPOSE_PROVIDER="podman compose"
  elif command -v podman-compose >/dev/null 2>&1; then
    COMPOSE_PROVIDER="podman-compose"
  elif command -v docker >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
    COMPOSE_PROVIDER="docker compose"
  else
    echo "no compose provider found; install podman compose, podman-compose, or docker compose" >&2
    exit 1
  fi
}

print_plan() {
  cargo test -p hail-test --test local_mail_testbed local_mail_testbed_dry_run_lists_required_fixtures -- --nocapture
}

HAIL_TESTBED_EMAIL="${HAIL_TESTBED_EMAIL:-alice@hail.test}"
HAIL_TESTBED_JMAP_URL="${HAIL_TESTBED_JMAP_URL:-http://127.0.0.1:18080}"
HAIL_TESTBED_HAIL_URL="${HAIL_TESTBED_HAIL_URL:-http://127.0.0.1:18081}"

cd "$ROOT_DIR"
need cargo

echo "==> local mail testbed fixture plan"
print_plan

if [[ "$DRY_RUN" == "1" ]]; then
  cat <<EOF

DRY RUN ONLY.
Would build image:  podman build -t hail:local .
Would start stack:  scripts/local-mail-testbed.sh
Would import into:  ${HAIL_TESTBED_EMAIL} at ${HAIL_TESTBED_JMAP_URL}

Expected URLs/checks after a real run:
  hail SPA/API:        ${HAIL_TESTBED_HAIL_URL}
  Stalwart JMAP:      ${HAIL_TESTBED_JMAP_URL}/.well-known/jmap
  API health:         curl -fsS ${HAIL_TESTBED_HAIL_URL}/api/health
  Login as:           ${HAIL_TESTBED_EMAIL} / <HAIL_TESTBED_PASSWORD>
  Expected fixtures:  personal-simple.eml, newsletter-tracking-pixel.eml, receipt-papertrail.eml
EOF
  exit 0
fi

choose_compose

if [[ "$NO_BUILD" != "1" ]]; then
  if command -v podman >/dev/null 2>&1; then
    echo "==> building hail:local with podman"
    podman build -t hail:local .
  elif command -v docker >/dev/null 2>&1; then
    echo "==> building hail:local with docker"
    docker build -t hail:local .
  else
    echo "neither podman nor docker found for image build" >&2
    exit 1
  fi
fi

export HAIL_SERVER_KEY="${HAIL_SERVER_KEY:-$(python3 - <<'PY'
import secrets
print(secrets.token_hex(32))
PY
)}"
export HAIL_PUBLIC_URL="${HAIL_PUBLIC_URL:-$HAIL_TESTBED_HAIL_URL}"

echo "==> starting local testbed compose stack with: ${COMPOSE_PROVIDER}"
$COMPOSE_PROVIDER -f scripts/local-mail-testbed.compose.yml up -d

echo "==> waiting for Stalwart JMAP discovery at ${HAIL_TESTBED_JMAP_URL}/.well-known/jmap"
for _ in $(seq 1 60); do
  if curl -fsS "${HAIL_TESTBED_JMAP_URL}/.well-known/jmap" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
curl -fsS "${HAIL_TESTBED_JMAP_URL}/.well-known/jmap" >/dev/null

echo "==> waiting for hail API health at ${HAIL_TESTBED_HAIL_URL}/api/health"
for _ in $(seq 1 60); do
  if curl -fsS "${HAIL_TESTBED_HAIL_URL}/api/health" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
curl -fsS "${HAIL_TESTBED_HAIL_URL}/api/health" >/dev/null

if [[ -z "${HAIL_TESTBED_PASSWORD:-}" ]]; then
  cat >&2 <<EOF

TODO: automatic Stalwart domain/user provisioning is not implemented yet.
The stack is running, but fixture import needs an existing mailbox.

Create domain/user manually in Stalwart:
  domain: hail.test
  user:   ${HAIL_TESTBED_EMAIL}

Then rerun with:
  HAIL_TESTBED_PASSWORD='<password>' scripts/local-mail-testbed.sh --no-build

Or import directly with the gated Rust test:
  HAIL_RUN_LOCAL_MAIL_TESTBED=1 HAIL_TESTBED_PASSWORD='<password>' \
    cargo test -p hail-test --test local_mail_testbed -- --nocapture
EOF
  exit 3
fi

echo "==> importing local mail fixtures through JMAP Email/import"
HAIL_RUN_LOCAL_MAIL_TESTBED=1 \
HAIL_TESTBED_JMAP_URL="$HAIL_TESTBED_JMAP_URL" \
HAIL_TESTBED_EMAIL="$HAIL_TESTBED_EMAIL" \
HAIL_TESTBED_PASSWORD="$HAIL_TESTBED_PASSWORD" \
cargo test -p hail-test --test local_mail_testbed local_mail_testbed_imports_fixtures_when_enabled -- --nocapture

cat <<EOF

Local mail testbed ready.
  hail SPA/API:   ${HAIL_TESTBED_HAIL_URL}
  Stalwart JMAP: ${HAIL_TESTBED_JMAP_URL}/.well-known/jmap
  Login:         ${HAIL_TESTBED_EMAIL} / <HAIL_TESTBED_PASSWORD>

Expected API checks:
  curl -fsS ${HAIL_TESTBED_HAIL_URL}/api/health
  Browser login should show imported synthetic mail after screener/routing workers process it.
  Imported fixtures: personal-simple.eml, newsletter-tracking-pixel.eml, receipt-papertrail.eml
EOF
