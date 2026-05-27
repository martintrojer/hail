use hail_db::labels::{
    LabelDbError, LabelSource, assign_label_name_to_thread, assign_label_name_to_threads,
    assign_label_to_thread, assign_label_to_threads, assigned_thread_ids_for_label, create_label,
    delete_label, find_label_by_name, list_label_thread_ids, list_labels, list_thread_labels,
    normalize_label_path, remove_label_from_thread, rename_label, upsert_gmail_label,
};

fn fresh_db_url() -> (String, TempDb) {
    let mut dir = std::env::temp_dir();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos();

    for attempt in 0..100_u8 {
        dir.push(format!("hail-db-labels-test-{pid}-{nanos}-{attempt}"));
        match std::fs::create_dir(&dir) {
            Ok(()) => {
                let path = dir.join("hail.db");
                let url = format!("sqlite://{}", path.display());
                return (url, TempDb { dir, path });
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                dir.pop();
            }
            Err(err) => panic!("create temp db dir: {err}"),
        }
    }

    panic!("failed to allocate unique temp db dir");
}

struct TempDb {
    dir: std::path::PathBuf,
    path: std::path::PathBuf,
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(self.path.with_extension("db-wal"));
        let _ = std::fs::remove_file(self.path.with_extension("db-shm"));
        let _ = std::fs::remove_dir(&self.dir);
    }
}

async fn setup() -> (sqlx::SqlitePool, TempDb) {
    let (url, guard) = fresh_db_url();
    let pool = hail_db::connect(&url).await.expect("connect");
    hail_db::migrate(&pool).await.expect("migrate");
    (pool, guard)
}

async fn insert_user(pool: &sqlx::SqlitePool, email: &str, account_id: &str) -> i64 {
    sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?, ?, ?)")
        .bind(email)
        .bind(account_id)
        .bind("2026-01-01T00:00:00Z")
        .execute(pool)
        .await
        .expect("user insert");

    sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind(email)
        .fetch_one(pool)
        .await
        .expect("user id")
}

#[tokio::test]
async fn normalizes_and_validates_flat_paths() {
    let path = normalize_label_path("  Work /  Big   Receipts  ").expect("valid path");
    assert_eq!(path.name, "Work/Big   Receipts");
    assert_eq!(path.normalized_name, "work/big receipts");
    assert_eq!(path.path_segments, vec!["Work", "Big   Receipts"]);
    assert_eq!(path.leaf_name(), "Big   Receipts");

    for invalid in [
        "",
        "   ",
        "/Work",
        "Work/",
        "Work//Receipts",
        "Work/ /Receipts",
    ] {
        assert!(normalize_label_path(invalid).is_err(), "{invalid:?}");
    }
}

#[tokio::test]
async fn create_list_rename_delete_labels_and_enforce_duplicates() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(&pool, "labels-crud@example.com", "acct-labels-crud").await;

    let work = create_label(&pool, user_id, " Work / Receipts ", Some("blue"))
        .await
        .expect("create label");
    assert_eq!(work.name, "Work/Receipts");
    assert_eq!(work.normalized_name, "work/receipts");
    assert_eq!(work.path_segments(), vec!["Work", "Receipts"]);
    assert_eq!(work.leaf_name(), "Receipts");
    assert_eq!(work.source, LabelSource::Manual);
    assert_eq!(work.color.as_deref(), Some("blue"));

    let duplicate = create_label(&pool, user_id, "work/receipts", None).await;
    assert!(matches!(duplicate, Err(LabelDbError::Sqlx(_))));

    let found = find_label_by_name(&pool, user_id, "work / receipts")
        .await
        .expect("find label")
        .expect("label exists");
    assert_eq!(found.id, work.id);

    let renamed = rename_label(&pool, user_id, work.id, "Finance/Receipts")
        .await
        .expect("rename label");
    assert_eq!(renamed.name, "Finance/Receipts");
    assert_eq!(renamed.normalized_name, "finance/receipts");

    let labels = list_labels(&pool, user_id).await.expect("list labels");
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].id, work.id);

    assert!(
        delete_label(&pool, user_id, work.id)
            .await
            .expect("delete label")
    );
    assert!(
        list_labels(&pool, user_id)
            .await
            .expect("list after delete")
            .is_empty()
    );
    assert!(
        !delete_label(&pool, user_id, work.id)
            .await
            .expect("delete absent")
    );
}

#[tokio::test]
async fn nested_full_paths_are_flat_and_assignments_cascade_on_delete() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(&pool, "labels-flat@example.com", "acct-labels-flat").await;

    let label = create_label(&pool, user_id, "Work/Receipts", None)
        .await
        .expect("create nested label");
    assert_eq!(list_labels(&pool, user_id).await.expect("labels").len(), 1);
    assert!(
        find_label_by_name(&pool, user_id, "Work")
            .await
            .expect("find parent")
            .is_none()
    );

    assert!(
        assign_label_to_thread(&pool, user_id, "thread-a", label.id)
            .await
            .expect("assign")
    );
    assert!(
        !assign_label_to_thread(&pool, user_id, "thread-a", label.id)
            .await
            .expect("idempotent assign")
    );
    assert_eq!(
        list_thread_labels(&pool, user_id, "thread-a")
            .await
            .expect("thread labels")[0]
            .name,
        "Work/Receipts"
    );
    assert_eq!(
        list_label_thread_ids(&pool, user_id, label.id, 10, 0)
            .await
            .expect("label threads"),
        vec!["thread-a".to_owned()]
    );

    delete_label(&pool, user_id, label.id)
        .await
        .expect("delete label");
    assert!(
        list_thread_labels(&pool, user_id, "thread-a")
            .await
            .expect("thread labels after delete")
            .is_empty()
    );
}

#[tokio::test]
async fn thread_assignment_helpers_support_remove_batch_and_inline_create() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(&pool, "labels-assign@example.com", "acct-labels-assign").await;
    let label = create_label(&pool, user_id, "Projects", None)
        .await
        .expect("create label");

    let assigned = assign_label_to_threads(&pool, user_id, &["t1", "t2", "t1"], label.id)
        .await
        .expect("batch assign");
    assert_eq!(assigned, 2);
    assert_eq!(
        label_threads_sorted(&pool, user_id, label.id).await,
        vec!["t1", "t2"]
    );

    assert!(
        remove_label_from_thread(&pool, user_id, "t1", label.id)
            .await
            .expect("remove")
    );
    assert!(
        !remove_label_from_thread(&pool, user_id, "t1", label.id)
            .await
            .expect("remove absent")
    );
    assert_eq!(
        label_threads_sorted(&pool, user_id, label.id).await,
        vec!["t2"]
    );

    let inline = assign_label_name_to_thread(&pool, user_id, "t3", "Inline/New")
        .await
        .expect("inline create");
    assert_eq!(inline.name, "Inline/New");
    assert_eq!(inline.thread_count, 1);

    let inline_again = assign_label_name_to_threads(&pool, user_id, &["t3", "t4"], "inline / new")
        .await
        .expect("inline batch reuse");
    assert_eq!(inline_again.id, inline.id);
    assert_eq!(inline_again.thread_count, 2);
}

#[tokio::test]
async fn strict_user_scoping_prevents_cross_user_reads_and_assignments() {
    let (pool, _guard) = setup().await;
    let user_a = insert_user(&pool, "labels-a@example.com", "acct-labels-a").await;
    let user_b = insert_user(&pool, "labels-b@example.com", "acct-labels-b").await;

    let a_label = create_label(&pool, user_a, "Shared", None)
        .await
        .expect("create a label");
    let b_label = create_label(&pool, user_b, "Shared", None)
        .await
        .expect("create b label");
    assert_ne!(a_label.id, b_label.id);

    assert!(
        !assign_label_to_thread(&pool, user_b, "thread-b", a_label.id)
            .await
            .expect("cross-user assignment ignored")
    );
    assert!(
        list_thread_labels(&pool, user_b, "thread-b")
            .await
            .expect("b thread labels")
            .is_empty()
    );

    assign_label_to_thread(&pool, user_a, "same-thread-id", a_label.id)
        .await
        .expect("a assign");
    assign_label_to_thread(&pool, user_b, "same-thread-id", b_label.id)
        .await
        .expect("b assign");
    assert_eq!(
        list_thread_labels(&pool, user_a, "same-thread-id")
            .await
            .unwrap()[0]
            .id,
        a_label.id
    );
    assert_eq!(
        list_thread_labels(&pool, user_b, "same-thread-id")
            .await
            .unwrap()[0]
            .id,
        b_label.id
    );
    assert_eq!(
        assigned_thread_ids_for_label(
            &pool,
            user_a,
            a_label.id,
            &["same-thread-id".to_owned(), "thread-b".to_owned()],
        )
        .await
        .expect("a search label filter ids"),
        vec!["same-thread-id".to_owned()]
    );
    assert!(
        assigned_thread_ids_for_label(
            &pool,
            user_b,
            a_label.id,
            &["same-thread-id".to_owned(), "thread-b".to_owned()],
        )
        .await
        .expect("cross-user search label filter ids")
        .is_empty()
    );
    assert_eq!(
        assigned_thread_ids_for_label(&pool, user_b, b_label.id, &["same-thread-id".to_owned()])
            .await
            .expect("b search label filter ids"),
        vec!["same-thread-id".to_owned()]
    );
}

#[tokio::test]
async fn gmail_upsert_merges_by_provider_identity_then_normalized_name() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(&pool, "labels-gmail@example.com", "acct-labels-gmail").await;

    let manual = create_label(&pool, user_id, "Work/Receipts", None)
        .await
        .expect("create manual");
    let merged = upsert_gmail_label(
        &pool,
        user_id,
        "Label_1",
        " work / receipts ",
        Some("green"),
    )
    .await
    .expect("merge gmail by normalized name");
    assert_eq!(merged.id, manual.id);
    assert_eq!(merged.source, LabelSource::Manual);
    assert_eq!(merged.provider_kind.as_deref(), Some("gmail"));
    assert_eq!(merged.provider_label_id.as_deref(), Some("Label_1"));

    let renamed = upsert_gmail_label(&pool, user_id, "Label_1", "Work/Renamed", Some("red"))
        .await
        .expect("rename gmail by provider id");
    assert_eq!(renamed.id, manual.id);
    assert_eq!(renamed.name, "Work/Renamed");
    assert_eq!(renamed.source, LabelSource::Gmail);
    assert_eq!(renamed.color.as_deref(), Some("red"));

    let new_gmail = upsert_gmail_label(&pool, user_id, "Label_2", "Travel", None)
        .await
        .expect("create gmail");
    assert_eq!(new_gmail.source, LabelSource::Gmail);
    assert_eq!(list_labels(&pool, user_id).await.expect("labels").len(), 2);
}

#[tokio::test]
async fn invalid_thread_and_provider_values_are_rejected_before_sql() {
    let (pool, _guard) = setup().await;
    let user_id = insert_user(&pool, "labels-invalid@example.com", "acct-labels-invalid").await;
    let label = create_label(&pool, user_id, "Valid", None)
        .await
        .expect("create label");

    assert!(matches!(
        assign_label_to_thread(&pool, user_id, " thread ", label.id).await,
        Err(LabelDbError::InvalidThreadId(_))
    ));
    assert!(matches!(
        upsert_gmail_label(&pool, user_id, " Label_1 ", "Gmail", None).await,
        Err(LabelDbError::InvalidProviderLabelId(_))
    ));
}

async fn label_threads_sorted(pool: &sqlx::SqlitePool, user_id: i64, label_id: i64) -> Vec<String> {
    let mut threads = list_label_thread_ids(pool, user_id, label_id, 100, 0)
        .await
        .expect("label thread ids");
    threads.sort();
    threads
}
