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
        "lume-run-{name}-{}-{nonce}.lum",
        std::process::id()
    ));
    fs::write(&path, source).expect("temporary source should be writable");
    path
}

fn stdlib_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../stdlib")
}

#[test]
fn run_command_forwards_arguments_after_the_separator() {
    let path = temp_source(
        "args",
        r#"
def selected() Unit {
    received = OS.args
    received.add("local-only")
    fresh = OS.args
    println(fresh.size, fresh[0], fresh[1])
}
"#,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_lume"))
        .arg("run")
        .arg(&path)
        .arg("selected")
        .arg("--")
        .arg("alpha")
        .arg("two words")
        .env("LUME_STDLIB", stdlib_dir())
        .output()
        .expect("lume run should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        "2 alpha two words\n"
    );
    fs::remove_file(path).expect("temporary source should be removable");
}

#[test]
fn run_command_requires_the_argument_separator() {
    let path = temp_source("missing-separator", "def main() Unit = ()\n");
    let output = Command::new(env!("CARGO_BIN_EXE_lume"))
        .arg("run")
        .arg(&path)
        .arg("main")
        .arg("unexpected")
        .env("LUME_STDLIB", stdlib_dir())
        .output()
        .expect("lume run should run");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("program arguments must follow '--'"));
    fs::remove_file(path).expect("temporary source should be removable");
}
