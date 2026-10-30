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
        "fn main() Unit {\n    var count = 1\n    println(count)\n}\n",
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

#[test]
fn interface_methods_and_getters_cannot_have_bodies() {
    for member in [
        "fn answer() Int = 42",
        "fn answer() Int { 42 }",
        "get answer Int = 42",
        "get answer Int { 42 }",
    ] {
        let path = temp_source(
            "interface-body",
            &format!("interface Contract {{ {member} }}"),
        );
        for command in ["check", "run"] {
            let output = Command::new(env!("CARGO_BIN_EXE_lume"))
                .arg(command)
                .arg(&path)
                .env(
                    "LUME_STDLIB",
                    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../stdlib"),
                )
                .output()
                .expect("lume should run");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(!output.status.success(), "{command} accepted {member}");
            assert!(stderr.contains("error[interface_member_body]"), "{stderr}");
            assert!(
                stderr.contains("move this implementation into a mixin"),
                "{stderr}"
            );
            assert!(output.stdout.is_empty());
        }
        fs::remove_file(path).expect("remove temporary source");
    }
}

#[test]
fn interfaces_cannot_declare_stored_fields() {
    for field in ["id Int", "id Int = 1", "var id Int"] {
        let path = temp_source(
            "interface-field",
            &format!("interface Contract {{ {field} }}"),
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
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "accepted {field}");
        assert!(stderr.contains("error[interface_field]"), "{stderr}");
        assert!(stderr.contains("get id Type"), "{stderr}");
        fs::remove_file(path).expect("remove temporary source");
    }
}

#[test]
fn interfaces_cannot_contain_nested_declarations() {
    for declaration in [
        "class Nested {}",
        "shape Nested {}",
        "interface Nested {}",
        "object Nested {}",
        "annotation Nested {}",
        "mixin Nested {}",
        "type Nested = Str",
        "type Nested = class Present { value Str } | object Missing {}",
    ] {
        let path = temp_source(
            "interface-nested-declaration",
            &format!("interface Contract {{ {declaration} }}"),
        );
        for command in ["check", "run"] {
            let output = Command::new(env!("CARGO_BIN_EXE_lume"))
                .arg(command)
                .arg(&path)
                .env(
                    "LUME_STDLIB",
                    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../stdlib"),
                )
                .output()
                .expect("lume should run");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(!output.status.success(), "{command} accepted {declaration}");
            assert!(
                stderr.contains("error[interface_nested_declaration]"),
                "{stderr}"
            );
            assert!(stderr.contains("'get' and 'fn' signatures"), "{stderr}");
            assert!(output.stdout.is_empty());
        }
        fs::remove_file(path).expect("remove temporary source");
    }
}

#[test]
fn named_unpacking_requires_braces() {
    for body in [
        "match result {\ncase Ok(value) => println(value)\ncase Err { error } => println(error)\n}",
        "if let Ok(value) = result { println(value) }",
        "let Ok(value) = result else return\nprintln(value)",
        "while let Ok(value) = result { println(value)\nbreak }",
        "for let Ok(value) <- [result] { println(value) }",
    ] {
        let path = temp_source(
            "parenthesized-unpacking",
            &format!("fn main() {{\nresult Result[Int, Str] = Ok(1)\n{body}\n}}"),
        );
        for command in ["check", "run"] {
            let output = Command::new(env!("CARGO_BIN_EXE_lume"))
                .arg(command)
                .arg(&path)
                .env(
                    "LUME_STDLIB",
                    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../stdlib"),
                )
                .output()
                .expect("lume should run");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(!output.status.success(), "{command} accepted {body}");
            assert!(stderr.contains("error[parenthesized_pattern]"), "{stderr}");
            assert!(stderr.contains("Ok { value as binding }"), "{stderr}");
            assert!(output.stdout.is_empty());
        }
        fs::remove_file(path).expect("remove temporary source");
    }
}
