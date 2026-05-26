# hail roadmap

Shipped **v1** on 2026-05-26. This is the post-v1 backlog, grouped into release themes.

---

## v1.1 — Polish & hardening

Quick wins from review findings and code quality cleanup. Mostly small effort, high ROI.

### Features
- **Hamburger keyboard nav** — fix j/k/Enter in the main menu dropdown (`fix-hamburger-keyboard-nav`)
- **Scheduled send cancel UI** — show pending scheduled sends with cancel button (`spa-scheduled-send-cancel-ui`)
- **Mark thread read/unread** — wire SPA API client contract (`fix-spa-api-client-mark-thread-contract`)
- **Spam filtering** — Stalwart spam verdict routing, spam view, mark-as-spam, auto-purge (`spam-filtering` → `mark-as-spam-verb`, `worker-spam-routing`, `spam-view`, `spam-auto-purge`)
- **Auth rate limiting** — per-IP rate limit on login/setup endpoints (`auth-rate-limiting`)

### Performance
- **Screener N+1 fix** — batch JMAP queries instead of per-sender lookups (`fix-screener-view-n-plus-one`)
- **Batch thread preview hydration** — reduce JMAP round-trips for pile/bubble-up previews (`fix-batch-thread-preview-hydration`)
- **Worker reconciliation optimization** — batch thread verification (`optimize-worker-reconcile-thread-verification`)

### Reliability
- **Worker catchup cancellation** — make all catch-up branches cancellation-aware (`fix-catchup-cancellation-branches`)
- **Auth /me resilience** — surface DB errors instead of silent fallback (`auth-me-display-name-db-errors`)
- **Admin management timeouts** — timeout Stalwart management API calls (`add-stalwart-management-timeouts`)
- **JMAP URL parser hardening** — proper URL parsing for base URL construction (`harden-jmap-base-url-host-parser`)
- **Logout CSRF hardening** — require CSRF token on logout (`harden-public-logout-csrf`)

### Code quality
- **Dedup defaultApiClient** — 60 import sites → React context (`dedup-default-api-client`)
- **Refactor mail classification** — unified enum across API/worker/SPA (`refactor-api-mail-classification`)
- **Refactor compose/draft pipeline** — shared outbound helpers (`refactor-outbound-compose-draft-common`)
- **Thread state clearing** — centralized sidecar cleanup on reclassify (`fix-dedup-thread-state-clearing`)
- **Screener shared primitives** — merge API/worker screener helpers (`fix-screener-shared-primitives`)
- **Typed JSON error responses** — structured error bodies (`fix-typed-json-error-responses`)
- Dead code removal: `cleanup-spa-pile-preview-dead-snippet`, `cleanup-webapp-dead-app-component`, `cleanup-worker-crypto-stale-dead-code`

### Tests
- **Test parallel safety** — eliminate env-var races in API tests (`harden-api-test-env-fixtures-parallel-safety`)
- **Trash purge session filter** — assert only active users are purged (`test-worker-trash-purge-active-session-filter`)
- **Keyboard shortcut tests** — cover vim keybinds (`test-spa-keyboard-shortcuts`)
- Various edge case coverage: `fix-mail-views-query-edge-tests`, `fix-spa-undo-toast-stale-async-tests`, `setup-cookie-helper-invariant-tests`, `test-worker-bubble-scheduler-concurrency`, `test-worker-catchup-cancel-current-state`

---

## v1.2 — New features

User-facing features that didn't make v1.

### Email workflow
- **Workflows / mail rules** — user-defined routing rules (`ui-workflows-rules` → `workflows-rules-api`, `workflows-rules-spa`)
- **Screener Speakeasy** — secret address that bypasses the Screener (`ui-screener-speakeasy`)
- **All Files view** — browse all attachments across threads (`ui-all-files-view`)
- **Compose identity selection** — pick sender identity for multi-domain users (`tighten-compose-identity-selection`)
- **Clips** — highlight and save text snippets from emails (`clips-feature`)
- **User invites** — admin invite link for passwordless onboarding (`user-invite-flow`)
- **PWA** — installable web app with an offline shell for the signed-in SPA. The service worker caches same-origin static assets only and deliberately bypasses `/api/*`, `/healthz`, and `/readyz`, so authenticated mail/API responses are never served stale. Web push notifications remain a separate future task (`pwa-service-worker`).

### Deployment
- **Cloudflare Tunnel E2E** — validated runbook for Tunnel + Email Routing (`e2e-cloudflare-mail-smoke`)
- **Shared Stalwart provisioning** — deduplicate user/domain provisioning between setup and admin (`share-email-domain-stalwart-provisioning`)
- **Litestream backup** — compose overlay for continuous SQLite replication to S3/R2 (`litestream-backup`)

---

## v2 — Terminal UI

A keyboard-native TUI client for hail, built with Node/Ink. Same API, different surface.

- **Architecture spike** — evaluate Node/Ink vs Rust/ratatui, API client reuse (`tui-architecture-spike`)
- **Scaffold** — Ink app with auth, navigation, Imbox list (`tui-node-ink-scaffold`)
- **Thread reader** — render sanitized HTML as terminal text (`tui-thread-reader`)
- **Screener** — approve/deny flow in the terminal (`tui-screener`)
- **Offline cache spike** — optional local SQLite cache for offline reading (`tui-offline-cache-spike`)

---

## Ongoing refactors (pick up anytime)

Low-urgency cleanups that improve developer experience. Good first tasks for new contributors.

| Task | Area | Effort |
|------|------|--------|
| `cleanup-compose-draft-outbound-duplication` | API | 0.75d |
| `cleanup-composer-autosave-effect` | SPA | 0.25d |
| `cleanup-webapp-query-client-singleton` | SPA | 0.25d |
| `cleanup-api-route-test-helper-duplication` | Tests | 0.25d |
| `cleanup-worker-path-included-module-test-noise` | Tests | 0.25d |
| `dedup-worker-api-app-event-types` | API/Worker | 0.5d |
| `fix-simplify-thread-action-trait-futures` | API | 0.5d |
| `fix-dedup-thread-participant-collection` | API | 0.25d |
| `fix-spa-message-popup-menu-roles` | SPA | 0.25d |
| `refactor-admin-duplicate-response-helpers` | API | 0.25d |
| `refactor-api-route-helpers` | API | 0.75d |
| `refactor-mail-render-quote-stripping-dom` | Core | 1d |
| `refactor-management-traits-async-trait-boxing` | API | 0.5d |
| `refactor-spa-mail-row-variants` | SPA | 0.25d |
| `refactor-spa-router-querykeys` | SPA | 0.25d |
| `refactor-spa-test-router-harness` | SPA | 0.5d |
| `refactor-spa-threadlink-source-search` | SPA | 0.25d |
| `refactor-worker-event-loop-catchup-helper` | Worker | 0.25d |
| `refactor-worker-shared-live-jmap-session` | Worker | 0.75d |
| `remove-redundant-session-ct-compare` | API | 0.25d |
| `simplify-management-traits-async-trait` | API | 0.5d |
| `prune-app-events-outbox` | DB | 0.5d |
| `thread-view-quoted-history-metadata` | Core | 0.5d |
| `fix-worker-jmapops-dead-account-id` | Worker | 0.1d |
