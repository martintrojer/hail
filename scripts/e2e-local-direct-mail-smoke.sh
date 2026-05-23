#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ "${HAIL_RUN_LOCAL_MAIL_TESTBED:-}" != "1" ]]; then
  cat <<'EOF'
Skipping local/direct mail smoke.
Set HAIL_RUN_LOCAL_MAIL_TESTBED=1 to run the disposable local Stalwart + hail-api + hail-worker E2E smoke.

Manual command:
  HAIL_RUN_LOCAL_MAIL_TESTBED=1 scripts/e2e-local-direct-mail-smoke.sh

This is intentionally gated because it starts Podman containers and live hail processes.
EOF
  exit 0
fi

if ! command -v podman >/dev/null 2>&1; then
  cat >&2 <<'EOF'
Cannot run local/direct mail smoke: podman is not on PATH.
Install/enable podman, then run:
  HAIL_RUN_LOCAL_MAIL_TESTBED=1 scripts/e2e-local-direct-mail-smoke.sh
EOF
  exit 2
fi

echo "==> building hail binaries used by the smoke"
RUSTFLAGS="${RUSTFLAGS:--D warnings}" cargo build -p hail-api -p hail-worker -p hail-test

echo "==> running local/direct mail smoke"
HAIL_RUN_LOCAL_MAIL_TESTBED=1 \
HAIL_E2E_LOCAL_DIRECT_MAIL_SMOKE_DIRECT=1 \
HAIL_RUN_STALWART_TESTS=1 \
RUSTFLAGS="${RUSTFLAGS:--D warnings}" \
cargo test -p hail-test --test e2e_local_direct_mail_smoke local_direct_mail_smoke_flow_when_enabled -- --nocapture
