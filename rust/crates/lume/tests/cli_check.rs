use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn temp_source(name: &str, source: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "lume-check-{name}-{}-{nonce}.lum",
        std::process::id()
    ));
    fs::write(&path, source).expect("temporary source should be writable");
    path
}

#[test]
fn check_reports_unused_mutability_without_failing() {
    let path = temp_source(
        "unused-mutability",
        "def main() Unit {\n    var count = 1\n    println(count)\n}\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_lume"))
        .arg("check")
        .arg(&path)
        .env(
            "LUME_STDLIB",
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../stdlib"),
        )
        .output()
        .expect("lume check should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("warning[unused_mutability]"), "{stderr}");
    assert!(stderr.contains("binding 'count' is declared mutable but never reassigned"));

    let run = Command::new(env!("CARGO_BIN_EXE_lume"))
        .arg("run")
        .arg(&path)
        .env(
            "LUME_STDLIB",
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../stdlib"),
        )
        .output()
        .expect("lume run should run");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "1\n");
    assert!(String::from_utf8_lossy(&run.stderr).contains("warning[unused_mutability]"));
    fs::remove_file(path).expect("temporary source should be removable");
}
