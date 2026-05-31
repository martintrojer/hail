#!/usr/bin/env bash
set -euo pipefail

STALWART_URL="${STALWART_URL:-http://stalwart:8080}"
STALWART_INIT_TIMEOUT_SECONDS="${STALWART_INIT_TIMEOUT_SECONDS:-120}"
STALWART_INIT_POLL_SECONDS="${STALWART_INIT_POLL_SECONDS:-2}"
ADMIN_CREDS="${STALWART_RECOVERY_ADMIN:?set STALWART_RECOVERY_ADMIN as user:password}"
ADMIN_USER="${ADMIN_CREDS%%:*}"
ADMIN_PASS="${ADMIN_CREDS#*:}"
HAIL_LOCAL_SINK="${HAIL_LOCAL_SINK:-0}"

if [[ -z "$ADMIN_USER" || -z "$ADMIN_PASS" || "$ADMIN_CREDS" != *:* ]]; then
  echo "stalwart-init: STALWART_RECOVERY_ADMIN must be user:password" >&2
  exit 1
fi

BASE_URL="${STALWART_URL%/}"

curl_common=(
  --silent
  --show-error
  --fail-with-body
  --connect-timeout 5
  --max-time 30
  --location
)

redact_secrets() {
  jq 'walk(
    if type == "object" then
      with_entries(
        if (.key | IN("access_token", "accessToken", "refresh_token", "refreshToken", "id_token", "token", "client_code", "clientCode", "code", "accountSecret", "account_secret", "secret", "password", "secrets"))
        then .value = "<redacted>"
        else .
        end
      )
    else .
    end
  )'
}

wait_for_stalwart() {
  local deadline=$((SECONDS + STALWART_INIT_TIMEOUT_SECONDS))
  echo "stalwart-init: waiting for Stalwart at ${BASE_URL}"
  while (( SECONDS < deadline )); do
    if curl "${curl_common[@]}" "${BASE_URL}/healthz/live" >/dev/null 2>&1; then
      echo "stalwart-init: Stalwart health endpoint is live"
      return 0
    fi
    sleep "$STALWART_INIT_POLL_SECONDS"
  done
  echo "stalwart-init: timed out waiting for Stalwart health endpoint" >&2
  return 1
}

authenticate() {
  local nonce auth_response client_code token_response access_token
  nonce="$(head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n')"

  auth_response=$(jq -n \
    --arg account_name "$ADMIN_USER" \
    --arg account_secret "$ADMIN_PASS" \
    --arg nonce "$nonce" \
    '{
      type: "authCode",
      accountName: $account_name,
      accountSecret: $account_secret,
      clientId: "webadmin",
      redirectUri: "https://localhost/",
      nonce: $nonce
    }' | curl "${curl_common[@]}" \
      --header 'Content-Type: application/json' \
      --data-binary @- \
      "${BASE_URL}/api/auth")
  client_code=$(jq -er '.client_code // .clientCode' <<<"$auth_response")

  token_response=$(curl "${curl_common[@]}" \
    --header 'Content-Type: application/x-www-form-urlencoded' \
    --data-urlencode 'grant_type=authorization_code' \
    --data-urlencode "code=${client_code}" \
    --data-urlencode 'client_id=webadmin' \
    --data-urlencode 'redirect_uri=https://localhost/' \
    "${BASE_URL}/auth/token")
  access_token=$(jq -er '.access_token // .accessToken' <<<"$token_response")
  printf '%s' "$access_token"
}

jmap_account_id() {
  local access_token="$1" session_json account_id
  session_json=$(curl "${curl_common[@]}" \
    --header "Authorization: Bearer ${access_token}" \
    "${BASE_URL}/.well-known/jmap")
  account_id=$(jq -er '.primaryAccounts["urn:stalwart:jmap"] // .primaryAccounts["urn:ietf:params:jmap:mail"]' <<<"$session_json")
  if [[ -z "$account_id" || "$account_id" == "null" ]]; then
    echo "stalwart-init: admin JMAP session did not expose a management account" >&2
    return 1
  fi
  printf '%s' "$account_id"
}

apply_settings() {
  local access_token="$1" account_id response status
  account_id=$(jmap_account_id "$access_token")
  echo "stalwart-init: applying hail-friendly Stalwart settings"

  response=$(jq -n --arg account_id "$account_id" '{
    using: ["urn:ietf:params:jmap:core", "urn:stalwart:jmap"],
    methodCalls: [
      ["x:Jmap/set", {
        accountId: $account_id,
        update: {
          singleton: {
            maxUploadCount: 100000000,
            uploadQuota: 1099511627776,
            maxUploadSize: 104857600,
            maxConcurrentUploads: 16
          }
        }
      }, "jmap"],
      ["x:Http/set", {
        accountId: $account_id,
        update: {
          singleton: {
            rateLimitAuthenticated: { count: 1000000, period: 60000 },
            rateLimitAnonymous: { count: 1000000, period: 60000 }
          }
        }
      }, "http"],
      ["x:Action/set", {
        accountId: $account_id,
        create: {
          reloadSettings: { "@type": "ReloadSettings" }
        }
      }, "reload"]
    ]
  }' | curl "${curl_common[@]}" \
    --header "Authorization: Bearer ${access_token}" \
    --header 'Content-Type: application/json' \
    --data-binary @- \
    "${BASE_URL}/jmap/")

  printf '%s\n' "$response" | redact_secrets

  status=$(jq -er '
    [ .methodResponses[]
      | select(.[0] == "x:Jmap/set" or .[0] == "x:Http/set" or .[0] == "x:Action/set")
      | .[1]
      | ((.notUpdated // {}) | length) + ((.notCreated // {}) | length) + ((.notDestroyed // {}) | length)
    ] | add // 0
  ' <<<"$response")
  if [[ "$status" != "0" ]]; then
    echo "stalwart-init: Stalwart rejected one or more settings" >&2
    return 1
  fi
}

verify_settings() {
  local access_token="$1" account_id response
  account_id=$(jmap_account_id "$access_token")
  echo "stalwart-init: verifying Stalwart settings"

  response=$(jq -n --arg account_id "$account_id" '{
    using: ["urn:ietf:params:jmap:core", "urn:stalwart:jmap"],
    methodCalls: [
      ["x:Jmap/get", {
        accountId: $account_id,
        ids: ["singleton"],
        properties: ["maxUploadCount", "uploadQuota", "maxUploadSize", "maxConcurrentUploads"]
      }, "jmap"],
      ["x:Http/get", {
        accountId: $account_id,
        ids: ["singleton"],
        properties: ["rateLimitAuthenticated", "rateLimitAnonymous"]
      }, "http"]
    ]
  }' | curl "${curl_common[@]}" \
    --header "Authorization: Bearer ${access_token}" \
    --header 'Content-Type: application/json' \
    --data-binary @- \
    "${BASE_URL}/jmap/")

  printf '%s\n' "$response" | redact_secrets

  jq -e '
    def method(name): .methodResponses[] | select(.[0] == name) | .[1].list[0];
    (method("x:Jmap/get").maxUploadCount >= 100000000) and
    (method("x:Jmap/get").uploadQuota >= 1099511627776) and
    (method("x:Jmap/get").maxUploadSize >= 104857600) and
    (method("x:Jmap/get").maxConcurrentUploads >= 16) and
    (method("x:Http/get").rateLimitAuthenticated.count >= 1000000) and
    (method("x:Http/get").rateLimitAuthenticated.period == 60000) and
    (method("x:Http/get").rateLimitAnonymous.count >= 1000000) and
    (method("x:Http/get").rateLimitAnonymous.period == 60000)
  ' <<<"$response" >/dev/null
}

local_sink_enabled() {
  case "${HAIL_LOCAL_SINK,,}" in
    1|true|yes|on) return 0 ;;
    *) return 1 ;;
  esac
}

apply_local_outbound_sink() {
  local access_token="$1" account_id response status
  account_id=$(jmap_account_id "$access_token")
  echo "stalwart-init: applying local smoke outbound sink"

  # LOCAL SMOKE ONLY: do not run this against a production Stalwart.
  # Stalwart v0.16 exposes MTA outbound routing through the singleton
  # MtaOutboundStrategy object. Force every queued recipient through the built-in
  # Local route instead of the default MX route so local smoke sends never try to
  # deliver to the public internet from the unroutable hail.test domain.
  response=$(jq -n --arg account_id "$account_id" --arg local_route "'local'" '{
    using: ["urn:ietf:params:jmap:core", "urn:stalwart:jmap"],
    methodCalls: [
      ["x:MtaOutboundStrategy/set", {
        accountId: $account_id,
        update: {
          singleton: {
            route: {
              match: {},
              else: $local_route
            }
          }
        }
      }, "mtaOutboundStrategy"],
      ["x:Action/set", {
        accountId: $account_id,
        create: {
          reloadSettings: { "@type": "ReloadSettings" }
        }
      }, "reload"]
    ]
  }' | curl "${curl_common[@]}" \
    --header "Authorization: Bearer ${access_token}" \
    --header 'Content-Type: application/json' \
    --data-binary @- \
    "${BASE_URL}/jmap/")

  printf '%s\n' "$response" | redact_secrets

  status=$(jq -er '
    [ .methodResponses[]
      | select(.[0] == "x:MtaOutboundStrategy/set" or .[0] == "x:Action/set")
      | .[1]
      | ((.notUpdated // {}) | length) + ((.notCreated // {}) | length) + ((.notDestroyed // {}) | length)
    ] | add // 0
  ' <<<"$response")
  if [[ "$status" != "0" ]]; then
    echo "stalwart-init: Stalwart rejected the local outbound sink settings" >&2
    return 1
  fi
}

verify_local_outbound_sink() {
  local access_token="$1" account_id response
  account_id=$(jmap_account_id "$access_token")
  echo "stalwart-init: verifying local smoke outbound sink"

  response=$(jq -n --arg account_id "$account_id" '{
    using: ["urn:ietf:params:jmap:core", "urn:stalwart:jmap"],
    methodCalls: [
      ["x:MtaOutboundStrategy/get", {
        accountId: $account_id,
        ids: ["singleton"],
        properties: ["route"]
      }, "mtaOutboundStrategy"]
    ]
  }' | curl "${curl_common[@]}" \
    --header "Authorization: Bearer ${access_token}" \
    --header 'Content-Type: application/json' \
    --data-binary @- \
    "${BASE_URL}/jmap/")

  printf '%s\n' "$response" | redact_secrets

  jq -e --arg local_route "'local'" '
    .methodResponses[]
    | select(.[0] == "x:MtaOutboundStrategy/get")
    | .[1].list[0].route
    | (.match == {}) and (.else == $local_route)
  ' <<<"$response" >/dev/null
}

wait_for_stalwart
ACCESS_TOKEN="$(authenticate)"
apply_settings "$ACCESS_TOKEN"
verify_settings "$ACCESS_TOKEN"
if local_sink_enabled; then
  apply_local_outbound_sink "$ACCESS_TOKEN"
  verify_local_outbound_sink "$ACCESS_TOKEN"
else
  echo "stalwart-init: local smoke outbound sink disabled"
fi
unset ACCESS_TOKEN

echo "stalwart-init: complete"
