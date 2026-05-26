use hail_test::stalwart::{render_stalwart_config, stalwart_tests_enabled, start_stalwart_fixture};

#[test]
fn rendered_fixture_config_remains_relay_free_by_default() {
    let config = hail_test::stalwart::StalwartConfig {
        hostname: "mail.hail.test".to_owned(),
        admin_user: "admin".to_owned(),
        admin_secret: "redacted".to_owned(),
    };
    let rendered = render_stalwart_config(&config);

    assert!(!rendered.contains("[queue.strategy]"));
    assert!(!rendered.contains("type = \"relay\""));
    assert!(!rendered.contains("HAIL_PROVIDER_SMTP_SECRET"));
}

#[test]
fn provider_smarthost_examples_use_env_secrets_and_preserve_local_delivery() {
    let deploy_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("deploy");
    for path in [
        deploy_dir.join("stalwart-provider-gmail-smarthost.example.toml"),
        deploy_dir.join("stalwart-provider-generic-smarthost.example.toml"),
    ] {
        let body = std::fs::read_to_string(&path).expect("read smarthost example");

        assert!(body.contains("[queue.strategy]"));
        assert!(body.contains("is_local_domain('', rcpt_domain)"));
        assert!(body.contains("then = \"'local'\""));
        assert!(body.contains("type = \"local\""));
        assert!(body.contains("type = \"relay\""));
        assert!(body.contains("port = 587"));
        assert!(body.contains("implicit = false"));
        assert!(body.contains("allow-invalid-certs = false"));
        assert!(body.contains("username = \"%{env:HAIL_PROVIDER_SMTP_USERNAME}%\""));
        assert!(body.contains("secret = \"%{env:HAIL_PROVIDER_SMTP_SECRET}%\""));
        assert!(!body.contains("CHANGE_ME"));
    }

    let gmail =
        std::fs::read_to_string(deploy_dir.join("stalwart-provider-gmail-smarthost.example.toml"))
            .expect("read gmail smarthost example");
    assert!(gmail.contains("address = \"smtp.gmail.com\""));

    let generic = std::fs::read_to_string(
        deploy_dir.join("stalwart-provider-generic-smarthost.example.toml"),
    )
    .expect("read generic smarthost example");
    assert!(generic.contains("address = \"%{env:HAIL_PROVIDER_SMTP_HOST}%\""));
}

#[tokio::test]
async fn stalwart_container_reaches_jmap_or_is_explicitly_skipped() {
    if !stalwart_tests_enabled() {
        eprintln!("skipping Stalwart integration test; set HAIL_RUN_STALWART_TESTS=1 to run it");
        return;
    }

    let fixture = start_stalwart_fixture()
        .await
        .expect("Stalwart container should start and expose JMAP");
    assert!(fixture.jmap_url().starts_with("http://localhost:"));
    assert!(fixture.http_port() > 0);
    assert!(fixture.smtp_port() > 0);
}

#[tokio::test]
async fn seeded_user_can_login_after_fixture_provisioning() {
    if !stalwart_tests_enabled() {
        eprintln!("skipping Stalwart provisioning test; set HAIL_RUN_STALWART_TESTS=1 to run it");
        return;
    }

    let fixture = start_stalwart_fixture()
        .await
        .expect("Stalwart container should start, bootstrap, and provision a test user");
    let session = fixture
        .login_seeded_user()
        .await
        .expect("seeded user should log in through hail_jmap");
    assert!(!session.account_id().is_empty());
}
