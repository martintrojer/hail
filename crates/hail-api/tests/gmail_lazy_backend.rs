use std::sync::Arc;

use hail_api::backend_factory::{ApiBackendFactory, LazyMailBackend};
use hail_backend::{MailBackend, Query};
use hail_blob_store::FilesystemBlobStore;
use hail_cache::{CachePolicy, CachedMail};
use hail_core::{
    MailBackend as ConfigMailBackend, ProviderOAuthToken, ProviderOAuthTokenKind,
    ProviderTokenContext, seal_provider_oauth_token,
};
use hail_test::{fixture_config, fixture_state, seed_session};

#[tokio::test]
async fn gmail_lazy_backend_returns_not_connected_without_account() {
    let (mut state, key) = fixture_state().await;
    state.config.mail.backend = ConfigMailBackend::Gmail;

    let backend = LazyMailBackend::new(ApiBackendFactory::new(
        Arc::new(state.config.clone()),
        state.db.clone(),
        key,
    ));
    let err = backend
        .list_message_ids(&Query::all(), &hail_backend::PageRequest::first(1))
        .await
        .expect_err("gmail backend should require a connected account only at mail-operation time");

    assert_eq!(err.to_string(), "mail backend account is not connected");
}

#[tokio::test]
async fn gmail_lazy_backend_resolves_after_account_connects() {
    let (mut state, key) = fixture_state().await;
    state.config.mail.backend = ConfigMailBackend::Gmail;
    state.config.provider_import.gmail.api_base_url =
        Some("http://127.0.0.1:9/gmail/v1/".to_owned());
    let (user_id, _session) = seed_session(&state, &key, "owner@example.org").await;

    let factory = ApiBackendFactory::new(Arc::new(state.config.clone()), state.db.clone(), key);
    assert!(
        factory
            .backend_for_current_account(tokio_util::sync::CancellationToken::new())
            .await
            .expect("lazy lookup before connect should not hard-fail")
            .is_none()
    );

    let provider_account_id = "owner@gmail.com";
    let context = ProviderTokenContext::new(
        user_id,
        1,
        "gmail",
        provider_account_id,
        ProviderOAuthTokenKind::Refresh,
    );
    let refresh_token =
        seal_provider_oauth_token(&ProviderOAuthToken::new("refresh-token"), &key, &context)
            .expect("seal refresh token")
            .into_bytes();
    let now = chrono::Utc::now();
    sqlx::query(
        "INSERT INTO mail_accounts \
         (id, user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, display_email, \
          granted_scopes_json, consented_at, refresh_token_enc, sync_status, created_at, updated_at) \
         VALUES (1, ?1, 'jmap-acct', 'gmail', 'gmail', ?2, ?2, ?2, \
          '[]', ?3, ?4, 'active', ?3, ?3)",
    )
    .bind(user_id)
    .bind(provider_account_id)
    .bind(now)
    .bind(refresh_token)
    .execute(&state.db)
    .await
    .expect("insert connected gmail account");

    assert!(
        factory
            .backend_for_current_account(tokio_util::sync::CancellationToken::new())
            .await
            .expect("lazy lookup after connect should build a backend")
            .is_some()
    );
}

#[tokio::test]
async fn gmail_mode_cached_mail_constructs_with_empty_database() {
    let url = "sqlite::memory:";
    let db = hail_db::connect(url).await.expect("open sqlite");
    hail_db::migrate(&db).await.expect("migrate");
    let key = [0xA5; hail_core::KEY_LEN];
    let mut config = fixture_config(url, &key);
    config.mail.backend = ConfigMailBackend::Gmail;

    let _mail = CachedMail::new(
        db.clone(),
        Arc::new(FilesystemBlobStore::new(
            config.mail.cache.blob_root.clone(),
        )),
        Box::new(LazyMailBackend::new(ApiBackendFactory::new(
            Arc::new(config.clone()),
            db,
            key,
        ))),
        CachePolicy::from(&config.mail.cache),
    );
}
