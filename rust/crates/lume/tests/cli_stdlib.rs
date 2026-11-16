use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn isolated_workspace() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("lume-native-stdlib-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn interpreter_contracts_do_not_require_a_stdlib_directory() {
    let directory = isolated_workspace();
    let source = directory.join("main.lum");
    fs::write(
        &source,
        r#"
fn main() {
    values = Vector.from([1, 2, 3])
    println(values.size, "  hi  ".trim().toUpper())
    println(Int.parse("7")! + 1, Decimal.parse("1.20")! + 0.3d)
    rows [[Int]] = Vector.generate(2, index => [])
    rows[0].add(7)
    println(rows[0].size, rows[1].size)
    filled [[Int]] = Vector.fill(2, [])
    filled[0].add(7)
    println(filled[1].size)
    println(Option.when(true, 9)!, OS.args.size)
}
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_lume"))
        .args(["run", source.to_str().unwrap(), "--", "argument"])
        .env_remove("LUME_STDLIB")
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "3 HI\n8 1.50\n1 0\n1\n9 1\n"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn invalid_native_calls_use_normal_argument_validation() {
    let directory = isolated_workspace();
    let source = directory.join("main.lum");
    for (expression, code) in [
        (
            "Vector.fill(misspelled = 2, value = 1)",
            "unknown_argument_name",
        ),
        ("Vector.fill(\"two\", 1)", "invalid_argument_type"),
        ("Option.when(true, 1, 2)", "invalid_argument_count"),
        ("Int.parse(misspelled = \"1\")", "unknown_argument_name"),
        ("\"text\".trim(1)", "invalid_argument_count"),
        ("Regex.compile(\"a\")", "undefined_name"),
        ("Json.decode[Int](\"1\")", "undefined_name"),
    ] {
        fs::write(
            &source,
            format!("fn main() {{\n    value = {expression}\n}}\n"),
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_lume"))
            .args(["check", source.to_str().unwrap()])
            .env_remove("LUME_STDLIB")
            .current_dir(&directory)
            .output()
            .unwrap();
        assert!(!output.status.success(), "accepted {expression}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(code),
            "{expression}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn imported_native_modules_use_registered_contracts_without_lum_files() {
    let directory = isolated_workspace();
    let source = directory.join("main.lum");
    fs::write(
        &source,
        r#"
use lume/json/{Json}
use lume/regex/{Regex}

fn main() {
    pattern = Regex.compile("a+")!
    println(pattern.matches("aaa"), Json.decode[Int]("5")!)
}
"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lume"))
        .args(["run", source.to_str().unwrap()])
        .env_remove("LUME_STDLIB")
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "true 5\n");
    fs::remove_dir_all(directory).unwrap();
}
