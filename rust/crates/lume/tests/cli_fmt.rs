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
        "lume-fmt-{name}-{}-{nonce}.lum",
        std::process::id()
    ));
    fs::write(&path, source).expect("temporary source should be writable");
    path
}

#[test]
fn fmt_command_rewrites_a_valid_file_in_place() {
    let path = temp_source("valid", "def main() Unit {\nprintln(\"ok\")  \n}");
    let output = Command::new(env!("CARGO_BIN_EXE_lume"))
        .arg("fmt")
        .arg(&path)
        .output()
        .expect("lume fmt should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert_eq!(
        fs::read_to_string(&path).expect("formatted source should be readable"),
        "def main() {\n    println(\"ok\")\n}\n"
    );
    fs::remove_file(path).expect("temporary source should be removable");
}

#[test]
fn fmt_command_uses_optional_type_shorthand() {
    let path = temp_source(
        "optional-types",
        "def echo(value Option[Int]) Option[Int] = value\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_lume"))
        .arg("fmt")
        .arg(&path)
        .output()
        .expect("lume fmt should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(&path).expect("formatted source should be readable"),
        "def echo(value Int?) Int? = value\n"
    );
    fs::remove_file(path).expect("temporary source should be removable");
}

#[test]
fn fmt_command_uses_extract_or_for_simple_extraction_bindings() {
    let path = temp_source(
        "extraction",
        "def parse(minutesText Str) Int? {\nlet minutes <- Int.parse(minutesText) else return None\nSome(minutes)\n}\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_lume"))
        .arg("fmt")
        .arg(&path)
        .output()
        .expect("lume fmt should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(&path).expect("formatted source should be readable"),
        "def parse(minutesText Str) Int? {\n    minutes = Int.parse(minutesText) ?? return None\n    Some(minutes)\n}\n"
    );
    fs::remove_file(path).expect("temporary source should be removable");
}

#[test]
fn fmt_command_preserves_invalid_source() {
    let source = "def main( Unit {\n";
    let path = temp_source("invalid", source);
    let output = Command::new(env!("CARGO_BIN_EXE_lume"))
        .arg("fmt")
        .arg(&path)
        .output()
        .expect("lume fmt should run");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("error["));
    assert_eq!(
        fs::read_to_string(&path).expect("invalid source should be readable"),
        source
    );
    fs::remove_file(path).expect("temporary source should be removable");
}
