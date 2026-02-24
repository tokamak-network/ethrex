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
    assert!(payload.get("retryable").is_some());
    assert!(payload.get("retry_attempts").is_some());
    assert!(payload.get("retry_attempts_used").is_some());
    assert!(payload.get("error").is_some());
    assert!(payload.get("elapsed_ms").is_some());

    let _ = fs::remove_dir_all(&old_path);
    let _ = fs::remove_dir_all(&new_path);
}

fn run_and_expect_clap_validation_error(args: &[&str], expected_flag: &str) {
    let bin = env!("CARGO_BIN_EXE_migrations");

    let output = Command::new(bin)
        .args(args)
        .output()
        .expect("failed to execute migrations binary");

    assert!(
        !output.status.success(),
        "command should fail clap validation"
    );

    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");
    assert!(stderr.contains(expected_flag));
    assert!(stderr.contains("range") || stderr.contains("..="));
}

#[test]
fn help_command_succeeds_and_lists_core_flags() {
    let bin = env!("CARGO_BIN_EXE_migrations");

    let output = Command::new(bin)
        .args(["libmdbx2rocksdb", "--help"])
        .output()
        .expect("failed to execute migrations binary");

    assert!(output.status.success(), "--help should succeed");

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf-8");
    assert!(stdout.contains("--dry-run"));
    assert!(stdout.contains("--json"));
    assert!(stdout.contains("--report-file"));
    assert!(stdout.contains("--retry-attempts"));
    assert!(stdout.contains("--retry-base-delay-ms"));
}

#[test]
fn clap_validation_failure_reports_retry_attempts_error() {
    run_and_expect_clap_validation_error(
        &[
            "libmdbx2rocksdb",
            "--genesis",
            "g.json",
            "--store.old",
            "old",
            "--store.new",
            "new",
            "--retry-attempts",
            "0",
        ],
        "retry-attempts",
    );
}

#[test]
fn clap_validation_failure_reports_retry_base_delay_error() {
    run_and_expect_clap_validation_error(
        &[
            "libmdbx2rocksdb",
            "--genesis",
            "g.json",
            "--store.old",
            "old",
            "--store.new",
            "new",
            "--retry-base-delay-ms",
            "60001",
        ],
        "retry-base-delay-ms",
    );
}

#[test]
fn report_file_captures_json_failure_output() {
    let bin = env!("CARGO_BIN_EXE_migrations");
    let old_path = unique_test_path("old-report-json");
    let new_path = unique_test_path("new-report-json");
    let report_path = unique_test_path("report-json").join("migration.jsonl");

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
            "--report-file",
            report_path.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("failed to execute migrations binary");

    assert!(!output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf-8");
    let report_content =
        fs::read_to_string(&report_path).expect("report file should be created and readable");
    let line = report_content
        .lines()
        .next()
        .expect("report file should contain one line");
    let payload: serde_json::Value =
        serde_json::from_str(line).expect("report line should be valid json");

    assert_eq!(
        stdout.trim(),
        line,
        "stdout json and report-file json line should match"
    );
    assert_eq!(payload["schema_version"], 1);
    assert_eq!(payload["status"], "failed");
    assert_eq!(payload["phase"], "execution");
    assert!(payload.get("error_type").is_some());
    assert!(payload.get("error_classification").is_some());
    assert!(payload.get("retryable").is_some());
    assert!(payload.get("retry_attempts").is_some());
    assert!(payload.get("retry_attempts_used").is_some());
    assert!(payload.get("error").is_some());
    assert!(payload.get("elapsed_ms").is_some());

    let _ = fs::remove_dir_all(&old_path);
    let _ = fs::remove_dir_all(&new_path);
    if let Some(parent) = report_path.parent() {
        let _ = fs::remove_dir_all(parent);
    }
}

#[test]
fn report_file_appends_across_multiple_json_failures() {
    let bin = env!("CARGO_BIN_EXE_migrations");
    let old_path = unique_test_path("old-report-append");
    let new_path = unique_test_path("new-report-append");
    let report_path = unique_test_path("report-append").join("migration.jsonl");

    for _ in 0..2 {
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
                "--report-file",
                report_path.to_string_lossy().as_ref(),
            ])
            .output()
            .expect("failed to execute migrations binary");

        assert!(!output.status.success());
    }

    let report_content =
        fs::read_to_string(&report_path).expect("report file should be created and readable");
    let lines: Vec<&str> = report_content.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "report file should append one json line per run"
    );

    for line in lines {
        let payload: serde_json::Value =
            serde_json::from_str(line).expect("report line should be valid json");
        assert_eq!(payload["schema_version"], 1);
        assert_eq!(payload["status"], "failed");
        assert_eq!(payload["phase"], "execution");
        assert!(payload.get("error_type").is_some());
        assert!(payload.get("error_classification").is_some());
        assert!(payload.get("retryable").is_some());
        assert!(payload.get("retry_attempts").is_some());
        assert!(payload.get("retry_attempts_used").is_some());
        assert!(payload.get("error").is_some());
        assert!(payload.get("elapsed_ms").is_some());
    }

    let _ = fs::remove_dir_all(&old_path);
    let _ = fs::remove_dir_all(&new_path);
    if let Some(parent) = report_path.parent() {
        let _ = fs::remove_dir_all(parent);
    }
}

#[test]
fn report_file_creates_parent_directories_for_json_failure() {
    let bin = env!("CARGO_BIN_EXE_migrations");
    let old_path = unique_test_path("old-report-parent-dirs");
    let new_path = unique_test_path("new-report-parent-dirs");
    let report_root = unique_test_path("report-parent-dirs");
    let report_path = report_root.join("nested/reports/migration.jsonl");

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
            "--report-file",
            report_path.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("failed to execute migrations binary");

    assert!(!output.status.success());
    assert!(
        report_path.exists(),
        "report file should be created with parent directories"
    );

    let report_content =
        fs::read_to_string(&report_path).expect("report file should be created and readable");
    let line = report_content
        .lines()
        .next()
        .expect("report file should contain one line");
    let payload: serde_json::Value =
        serde_json::from_str(line).expect("report line should be valid json");
    assert_eq!(payload["status"], "failed");

    let _ = fs::remove_dir_all(&old_path);
    let _ = fs::remove_dir_all(&new_path);
    let _ = fs::remove_dir_all(&report_root);
}

#[test]
fn report_file_creates_parent_directories_for_human_failure() {
    let bin = env!("CARGO_BIN_EXE_migrations");
    let old_path = unique_test_path("old-report-parent-dirs-human");
    let new_path = unique_test_path("new-report-parent-dirs-human");
    let report_root = unique_test_path("report-parent-dirs-human");
    let report_path = report_root.join("nested/reports/migration.log");

    let output = Command::new(bin)
        .args([
            "libmdbx2rocksdb",
            "--genesis",
            "./does-not-exist-genesis.json",
            "--store.old",
            old_path.to_string_lossy().as_ref(),
            "--store.new",
            new_path.to_string_lossy().as_ref(),
            "--report-file",
            report_path.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("failed to execute migrations binary");

    assert!(!output.status.success());
    assert!(
        report_path.exists(),
        "report file should be created with parent directories"
    );

    let report_content =
        fs::read_to_string(&report_path).expect("report file should be created and readable");
    assert!(report_content.contains("Migration failed after"));

    let _ = fs::remove_dir_all(&old_path);
    let _ = fs::remove_dir_all(&new_path);
    let _ = fs::remove_dir_all(&report_root);
}

#[test]
fn report_file_appends_across_multiple_human_failures() {
    let bin = env!("CARGO_BIN_EXE_migrations");
    let old_path = unique_test_path("old-report-append-human");
    let new_path = unique_test_path("new-report-append-human");
    let report_path = unique_test_path("report-append-human").join("migration.log");

    for run in 1..=2 {
        let output = Command::new(bin)
            .args([
                "libmdbx2rocksdb",
                "--genesis",
                "./does-not-exist-genesis.json",
                "--store.old",
                old_path.to_string_lossy().as_ref(),
                "--store.new",
                new_path.to_string_lossy().as_ref(),
                "--report-file",
                report_path.to_string_lossy().as_ref(),
            ])
            .output()
            .expect("failed to execute migrations binary");

        assert!(!output.status.success());

        let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");
        let stderr_line = stderr
            .lines()
            .find(|line| line.contains("Migration failed after"))
            .expect("stderr should contain migration failure line");

        let report_content =
            fs::read_to_string(&report_path).expect("report file should be created and readable");
        let lines: Vec<&str> = report_content
            .lines()
            .filter(|line| line.contains("Migration failed after"))
            .collect();
        assert_eq!(
            lines.len(),
            run,
            "report file should append one failure line per run"
        );

        let latest_report_line = lines
            .last()
            .expect("report file should contain at least one failure line");
        assert_eq!(
            *latest_report_line, stderr_line,
            "latest appended report line should match current stderr failure line"
        );
    }

    let _ = fs::remove_dir_all(&old_path);
    let _ = fs::remove_dir_all(&new_path);
    if let Some(parent) = report_path.parent() {
        let _ = fs::remove_dir_all(parent);
    }
}

#[test]
fn report_file_captures_human_failure_output() {
    let bin = env!("CARGO_BIN_EXE_migrations");
    let old_path = unique_test_path("old-report-human");
    let new_path = unique_test_path("new-report-human");
    let report_path = unique_test_path("report-human").join("migration.log");

    let output = Command::new(bin)
        .args([
            "libmdbx2rocksdb",
            "--genesis",
            "./does-not-exist-genesis.json",
            "--store.old",
            old_path.to_string_lossy().as_ref(),
            "--store.new",
            new_path.to_string_lossy().as_ref(),
            "--report-file",
            report_path.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("failed to execute migrations binary");

    assert!(!output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");
    let report_content =
        fs::read_to_string(&report_path).expect("report file should be created and readable");
    assert!(report_content.contains("Migration failed after"));

    let stderr_line = stderr
        .lines()
        .find(|line| line.contains("Migration failed after"))
        .expect("stderr should contain migration failure line");
    let report_line = report_content
        .lines()
        .find(|line| line.contains("Migration failed after"))
        .expect("report should contain migration failure line");
    assert_eq!(
        stderr_line, report_line,
        "stderr and report failure lines should match"
    );

    let _ = fs::remove_dir_all(&old_path);
    let _ = fs::remove_dir_all(&new_path);
    if let Some(parent) = report_path.parent() {
        let _ = fs::remove_dir_all(parent);
    }
}
