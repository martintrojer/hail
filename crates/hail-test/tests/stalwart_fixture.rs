use hail_test::stalwart::{stalwart_tests_enabled, start_stalwart_fixture};

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
