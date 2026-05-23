use hail_test::local_mail_testbed::{
    LOCAL_TESTBED_FIXTURE_NAMES, import_local_testbed_fixtures_via_jmap, local_testbed_plan,
};
use secrecy::SecretString;
use std::process::Command;

fn local_mail_testbed_enabled() -> bool {
    std::env::var("HAIL_RUN_LOCAL_MAIL_TESTBED").is_ok_and(|value| value == "1")
}

#[test]
fn local_mail_testbed_dry_run_lists_required_fixtures() {
    let plan = local_testbed_plan().expect("dry-run plan should load fixtures");
    let names = plan.iter().map(|fixture| fixture.name).collect::<Vec<_>>();
    assert_eq!(names, LOCAL_TESTBED_FIXTURE_NAMES);

    for fixture in &plan {
        println!(
            "fixture={} bytes={} intended_view={}",
            fixture.name,
            fixture.bytes,
            fixture.intended_view.as_deref().unwrap_or("<missing>")
        );
    }

    assert!(
        plan.iter()
            .any(|fixture| fixture.name == "personal-simple.eml")
    );
    assert!(
        plan.iter()
            .any(|fixture| fixture.name == "newsletter-tracking-pixel.eml")
    );
    assert!(
        plan.iter()
            .any(|fixture| fixture.name == "receipt-papertrail.eml")
    );
}

#[test]
fn local_mail_testbed_script_dry_run_prints_expected_checks() {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("scripts/local-mail-testbed.sh");
    let output = Command::new(script)
        .arg("--dry-run")
        .output()
        .expect("run dry-run script");
    assert!(
        output.status.success(),
        "dry-run failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("DRY RUN ONLY"));
    assert!(stdout.contains("personal-simple.eml"));
    assert!(stdout.contains("newsletter-tracking-pixel.eml"));
    assert!(stdout.contains("receipt-papertrail.eml"));
    assert!(stdout.contains("curl -fsS http://127.0.0.1:18081/api/health"));
}

#[tokio::test]
async fn local_mail_testbed_imports_fixtures_when_enabled() {
    if !local_mail_testbed_enabled() {
        eprintln!(
            "skipping local mail testbed import; set HAIL_RUN_LOCAL_MAIL_TESTBED=1 to run it"
        );
        return;
    }

    let jmap_url = std::env::var("HAIL_TESTBED_JMAP_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:18080".to_owned());
    let email = std::env::var("HAIL_TESTBED_EMAIL")
        .unwrap_or_else(|_| "alice@hail.test".to_owned());
    let password = std::env::var("HAIL_TESTBED_PASSWORD").expect(
        "HAIL_TESTBED_PASSWORD is required; automatic Stalwart user provisioning is TODO",
    );

    let imported = import_local_testbed_fixtures_via_jmap(
        &jmap_url,
        &email,
        SecretString::from(password),
    )
    .await
    .expect("fixtures should import through JMAP Email/import");

    let names = imported
        .iter()
        .map(|email| email.fixture_name)
        .collect::<Vec<_>>();
    assert_eq!(names, LOCAL_TESTBED_FIXTURE_NAMES);
    assert!(imported.iter().all(|email| !email.email_id.is_empty()));

    for email in imported {
        println!(
            "imported fixture={} email_id={} thread_id={}",
            email.fixture_name,
            email.email_id,
            email.thread_id.as_deref().unwrap_or("<none>")
        );
    }
}
