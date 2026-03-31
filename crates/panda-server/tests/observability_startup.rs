use std::process::Command;

#[test]
fn startup_with_otlp_env_fails_only_on_missing_config() {
    let missing = tempfile::tempdir()
        .expect("tempdir")
        .path()
        .join("does-not-exist.yaml");

    let output = Command::new(env!("CARGO_BIN_EXE_panda"))
        .arg(missing)
        .env("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:4318/v1/traces")
        .env("PANDA_OTEL_SERVICE_NAME", "panda-test")
        .output()
        .expect("run panda binary");

    assert!(
        !output.status.success(),
        "expected non-zero status for missing config"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("failed to load config"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        !stderr.to_lowercase().contains("panicked"),
        "unexpected panic output: {stderr}"
    );
}
