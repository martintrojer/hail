#!/usr/bin/env python3
"""Validate provider smarthost deployment examples.

This keeps the optional Stalwart smarthost snippets and Compose overlay honest:
- example TOML must parse and point relay credentials at env placeholders;
- the overlay YAML must parse and only inject smarthost env vars into Stalwart;
- the canonical Compose deployment must not include provider smarthost settings.
"""

from __future__ import annotations

import pathlib
import re
import sys
import tomllib
from typing import Any

try:
    import yaml
except ImportError as exc:  # pragma: no cover - exercised only on hosts missing PyYAML.
    raise SystemExit(
        "PyYAML is required for Compose YAML validation. Install it with "
        "`python3 -m pip install PyYAML` or run in the dev toolbox if provided."
    ) from exc

REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]
DEPLOY_DIR = REPO_ROOT / "deploy"

ENV_USERNAME = "%{env:HAIL_PROVIDER_SMTP_USERNAME}%"
ENV_SECRET = "%{env:HAIL_PROVIDER_SMTP_SECRET}%"
ENV_HOST = "%{env:HAIL_PROVIDER_SMTP_HOST}%"

EXAMPLES = {
    "gmail": {
        "path": DEPLOY_DIR / "stalwart-provider-gmail-smarthost.example.toml",
        "route_name": "provider-gmail-smtp",
        "address": "smtp.gmail.com",
    },
    "generic": {
        "path": DEPLOY_DIR / "stalwart-provider-generic-smarthost.example.toml",
        "route_name": "provider-smtp",
        "address": ENV_HOST,
    },
}

SUSPICIOUS_SECRET_RE = re.compile(
    r"(?i)(ya29\.|ghp_[a-z0-9]|xox[baprs]-|sk_live_|AKIA[0-9A-Z]{16}|-----BEGIN [A-Z ]*PRIVATE KEY-----)"
)


def load_toml(path: pathlib.Path) -> dict[str, Any]:
    try:
        return tomllib.loads(path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as exc:
        raise AssertionError(f"{path} is not valid TOML: {exc}") from exc


def load_yaml(path: pathlib.Path) -> dict[str, Any]:
    try:
        data = yaml.safe_load(path.read_text(encoding="utf-8"))
    except yaml.YAMLError as exc:
        raise AssertionError(f"{path} is not valid YAML: {exc}") from exc
    if not isinstance(data, dict):
        raise AssertionError(f"{path} did not parse to a mapping")
    return data


def assert_no_checked_in_secret(path: pathlib.Path) -> None:
    body = path.read_text(encoding="utf-8")
    if SUSPICIOUS_SECRET_RE.search(body):
        raise AssertionError(f"{path} appears to contain a real secret/token")


def assert_smarthost_example(name: str, spec: dict[str, Any]) -> None:
    path = spec["path"]
    assert_no_checked_in_secret(path)
    config = load_toml(path)

    queue = config.get("queue")
    if not isinstance(queue, dict):
        raise AssertionError(f"{path} missing [queue] table")

    strategy = queue.get("strategy")
    routes = queue.get("route")
    if not isinstance(strategy, dict) or not isinstance(routes, dict):
        raise AssertionError(f"{path} missing queue.strategy or queue.route tables")

    route_plan = strategy.get("route")
    expected_route_name = spec["route_name"]
    expected_route_plan = [
        {"if": "is_local_domain('', rcpt_domain)", "then": "'local'"},
        {"else": f"'{expected_route_name}'"},
    ]
    if route_plan != expected_route_plan:
        raise AssertionError(f"{path} route plan changed: {route_plan!r}")

    local = routes.get("local")
    if local != {"type": "local"}:
        raise AssertionError(f"{path} must preserve local-domain delivery before relaying")

    relay = routes.get(expected_route_name)
    if not isinstance(relay, dict):
        raise AssertionError(f"{path} missing queue.route.{expected_route_name}")

    expected_relay = {
        "type": "relay",
        "address": spec["address"],
        "port": 587,
        "protocol": "smtp",
        "tls": {"implicit": False, "allow-invalid-certs": False},
        "auth": {"username": ENV_USERNAME, "secret": ENV_SECRET},
    }
    if relay != expected_relay:
        raise AssertionError(f"{name} relay config mismatch in {path}: {relay!r}")


def assert_overlay_yaml() -> None:
    overlay_path = DEPLOY_DIR / "docker-compose.provider-smarthost.yml"
    assert_no_checked_in_secret(overlay_path)
    overlay = load_yaml(overlay_path)

    stalwart = overlay.get("services", {}).get("stalwart")
    if not isinstance(stalwart, dict):
        raise AssertionError(f"{overlay_path} must define services.stalwart")
    environment = stalwart.get("environment")
    if not isinstance(environment, list):
        raise AssertionError(f"{overlay_path} services.stalwart.environment must be a list")

    expected_env = {
        "HAIL_PROVIDER_SMTP_USERNAME=${HAIL_PROVIDER_SMTP_USERNAME:?set HAIL_PROVIDER_SMTP_USERNAME in deploy/.env or your secret manager}",
        "HAIL_PROVIDER_SMTP_SECRET=${HAIL_PROVIDER_SMTP_SECRET:?set HAIL_PROVIDER_SMTP_SECRET in deploy/.env or your secret manager}",
        "HAIL_PROVIDER_SMTP_HOST=${HAIL_PROVIDER_SMTP_HOST:-smtp.relay.example}",
    }
    if set(environment) != expected_env:
        raise AssertionError(f"{overlay_path} smarthost environment mismatch: {environment!r}")

    for item in environment:
        if "HAIL_PROVIDER_SMTP_SECRET=" in item and "${HAIL_PROVIDER_SMTP_SECRET:" not in item:
            raise AssertionError(f"{overlay_path} must not contain a literal provider SMTP secret")


def assert_canonical_deployment_unaffected() -> None:
    compose_path = DEPLOY_DIR / "docker-compose.yml"
    compose = load_yaml(compose_path)
    stalwart = compose.get("services", {}).get("stalwart")
    if not isinstance(stalwart, dict):
        raise AssertionError(f"{compose_path} must define services.stalwart")

    rendered = repr(stalwart)
    if "HAIL_PROVIDER_SMTP_" in rendered or "provider-smarthost" in rendered:
        raise AssertionError(f"{compose_path} should not include provider smarthost overlay settings")

    volumes = stalwart.get("volumes")
    if not isinstance(volumes, list) or "./stalwart.example.toml:/opt/stalwart/etc/config.toml:ro" not in volumes:
        raise AssertionError(f"{compose_path} should keep the normal Stalwart config mount")

    stalwart_toml = (DEPLOY_DIR / "stalwart.example.toml").read_text(encoding="utf-8")
    forbidden = ["provider-gmail-smtp", "provider-smtp", "type = \"relay\""]
    present = [needle for needle in forbidden if needle in stalwart_toml]
    if present:
        raise AssertionError(f"deploy/stalwart.example.toml should remain relay-free; found {present!r}")
    if "HAIL_PROVIDER_SMTP_SECRET" in stalwart_toml and "[queue.strategy]" in stalwart_toml:
        raise AssertionError("deploy/stalwart.example.toml should not define provider smarthost routing")


def main() -> int:
    for name, spec in EXAMPLES.items():
        assert_smarthost_example(name, spec)
    assert_overlay_yaml()
    assert_canonical_deployment_unaffected()
    print("provider smarthost deployment examples validated")
    return 0


if __name__ == "__main__":
    sys.exit(main())
