#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ "${HAIL_RUN_LOCAL_MAIL_TESTBED:-}" != "1" ]]; then
  cat <<'EOF'
Skipping compose send-later smoke.
Set HAIL_RUN_LOCAL_MAIL_TESTBED=1 to run the disposable local Stalwart + hail-api + hail-worker compose send-later E2E smoke.

Manual command:
  HAIL_RUN_LOCAL_MAIL_TESTBED=1 scripts/e2e-compose-send-later-smoke.sh

This is intentionally gated because it starts Podman containers and live hail processes.
EOF
  exit 0
fi

if ! command -v podman >/dev/null 2>&1; then
  cat >&2 <<'EOF'
Cannot run compose send-later smoke: podman is not on PATH.
Install/enable podman, then run:
  HAIL_RUN_LOCAL_MAIL_TESTBED=1 scripts/e2e-compose-send-later-smoke.sh
EOF
  exit 2
fi

echo "==> building hail binaries used by the smoke"
RUSTFLAGS="${RUSTFLAGS:--D warnings}" cargo build -p hail-api -p hail-worker -p hail-test

echo "==> running compose send-later smoke"
HAIL_RUN_LOCAL_MAIL_TESTBED=1 \
HAIL_E2E_COMPOSE_SEND_LATER_DIRECT=1 \
HAIL_RUN_STALWART_TESTS=1 \
RUSTFLAGS="${RUSTFLAGS:--D warnings}" \
cargo test -p hail-test --test e2e_compose_send_later_smoke compose_send_later_smoke_flow_when_enabled -- --nocapture
