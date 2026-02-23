use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_test_path(suffix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("migrations-cli-{suffix}-{nanos}"))
}

#[test]
fn emits_json_failure_payload_for_runtime_error() {
    let bin = env!("CARGO_BIN_EXE_migrations");
    let old_path = unique_test_path("old");
    let new_path = unique_test_path("new");

    let output = Command::new(bin)
        .args([
            "libmdbx2rocksdb",
            "--genesis",
            "./does-not-exist-genesis.json",
            "--store.old",
            old_path.to_string_lossy().as_ref(),
            "--store.new",
            new_path.to_string_lossy().as_ref(),
            "--json",
        ])
        .output()
        .expect("failed to execute migrations binary");

    assert!(
        !output.status.success(),
        "command should fail for invalid/non-existent stores"
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf-8");
    let payload: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be valid JSON");

    assert_eq!(payload["schema_version"], 1);
    assert_eq!(payload["status"], "failed");
    assert_eq!(payload["phase"], "execution");
    assert!(payload.get("error_type").is_some());
    assert!(payload.get("error_classification").is_some());
    assert!(payload.get("retry_attempts").is_some());
    assert!(payload.get("error").is_some());
    assert!(payload.get("elapsed_ms").is_some());

    let _ = fs::remove_dir_all(&old_path);
    let _ = fs::remove_dir_all(&new_path);
}

#[test]
fn clap_validation_failure_reports_usage_error() {
    let bin = env!("CARGO_BIN_EXE_migrations");

    let output = Command::new(bin)
        .args([
            "libmdbx2rocksdb",
            "--genesis",
            "g.json",
            "--store.old",
            "old",
            "--store.new",
            "new",
            "--retry-attempts",
            "0",
        ])
        .output()
        .expect("failed to execute migrations binary");

    assert!(!output.status.success(), "command should fail clap validation");

    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");
    assert!(stderr.contains("retry-attempts"));
    assert!(stderr.contains("1..=10") || stderr.contains("range"));
}
