use hail_test::stalwart::{StalwartFixtureError, stalwart_tests_enabled, start_stalwart_fixture};

#[tokio::test]
async fn stalwart_container_reaches_jmap_or_is_explicitly_skipped() {
    if !stalwart_tests_enabled() {
        eprintln!("skipping Stalwart integration test; set HAIL_RUN_STALWART_TESTS=1 to run it");
        return;
    }

    let fixture = start_stalwart_fixture()
        .await
        .expect("Stalwart container should start and expose JMAP");
    assert!(fixture.jmap_url().starts_with("http://127.0.0.1:"));
    assert!(fixture.http_port() > 0);
    assert!(fixture.smtp_port() > 0);
}

#[tokio::test]
async fn seeded_user_login_reports_not_implemented_until_provisioning_is_pinned() {
    if !stalwart_tests_enabled() {
        eprintln!("skipping Stalwart provisioning test; set HAIL_RUN_STALWART_TESTS=1 to run it");
        return;
    }

    let fixture = start_stalwart_fixture()
        .await
        .expect("Stalwart container should start and expose JMAP");
    let err = match fixture.login_seeded_user().await {
        Ok(_) => panic!("seeded login must not fake success before provisioning is implemented"),
        Err(err) => err,
    };
    assert!(matches!(
        err,
        StalwartFixtureError::UserProvisioningNotImplemented
    ));
}
