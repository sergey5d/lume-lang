use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

fn temp_directory(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "lume-definition-{name}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("temporary directory should be writable");
    path
}

#[test]
fn definition_command_uses_unsaved_source_and_follows_imported_types() {
    let directory = temp_directory("imported-type");
    let models = directory.join("models.lum");
    fs::write(
        &models,
        "module models\n\npublic shape User {\n    name Str\n}\n",
    )
    .expect("models module should be writable");
    let source_path = directory.join("app.lum");
    fs::write(&source_path, "module app\n\nfn main() {}\n")
        .expect("root module should be writable");

    let source = concat!(
        "module app\n",
        "use models/{User}\n\n",
        "fn main() {\n",
        "    user User = User { name: \"Ada\" }\n",
        "}\n",
    );
    let offset = source.find("user User").expect("type use") + "user ".len();
    let mut child = Command::new(env!("CARGO_BIN_EXE_lume"))
        .arg("definition")
        .arg(&source_path)
        .arg("--offset")
        .arg(offset.to_string())
        .arg("--stdin")
        .env(
            "LUME_STDLIB",
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../stdlib"),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("lume definition should run");
    child
        .stdin
        .take()
        .expect("definition stdin")
        .write_all(source.as_bytes())
        .expect("unsaved source should be writable");
    let output = child.wait_with_output().expect("definition should finish");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("definition output should be JSON");
    assert_eq!(
        PathBuf::from(value["path"].as_str().expect("definition path")),
        fs::canonicalize(models).expect("models path should resolve")
    );
    assert_eq!(value["line"], 3);
    assert_eq!(value["column"], 8);

    fs::remove_dir_all(directory).expect("temporary directory should be removable");
}

#[test]
fn definition_command_finds_imports_from_an_enclosing_source_root() {
    let directory = temp_directory("nested-module");
    let source_root = directory.join("src/main/lume");
    let nested = source_root.join("store");
    fs::create_dir_all(&nested).expect("nested module directory should be writable");
    let config = source_root.join("Config.lum");
    fs::write(
        &config,
        "module app/config\n\npublic shape ServiceConfig {\n    enabled Bool\n}\n",
    )
    .expect("config module should be writable");
    let source_path = nested.join("api.lum");
    let source = concat!(
        "module app/store\n\n",
        "use Config/*\n\n",
        "use unavailable/db/{Database}\n\n",
        "public interface Store {\n",
        "    fn open(db Database, config ServiceConfig) Unit\n",
        "}\n",
    );
    fs::write(&source_path, source).expect("nested module should be writable");
    let offset = source.find("ServiceConfig").expect("type use");
    let output = Command::new(env!("CARGO_BIN_EXE_lume"))
        .arg("definition")
        .arg(&source_path)
        .arg("--offset")
        .arg(offset.to_string())
        .env(
            "LUME_STDLIB",
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../stdlib"),
        )
        .output()
        .expect("lume definition should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("definition output should be JSON");
    assert_eq!(
        PathBuf::from(value["path"].as_str().expect("definition path")),
        fs::canonicalize(config).expect("config path should resolve")
    );
    assert_eq!(value["line"], 3);
    assert_eq!(value["column"], 8);

    fs::remove_dir_all(directory).expect("temporary directory should be removable");
}

#[test]
fn definition_command_follows_aliased_module_members() {
    let directory = temp_directory("module-alias");
    let customers = directory.join("customers");
    fs::create_dir_all(&customers).expect("create module directory");
    let storage = customers.join("storage.lum");
    fs::write(
        &storage,
        concat!(
            "module customers/storage\n\n",
            "public shape Customer { id Int }\n\n",
            "public fn load(id Int) Customer = Customer(id)\n",
        ),
    )
    .expect("write storage module");
    let source_path = directory.join("app.lum");
    let source = concat!(
        "use customers/storage as customerStore\n\n",
        "fn main() {\n",
        "    customer customerStore.Customer = customerStore.load(1)\n",
        "}\n",
    );
    fs::write(&source_path, source).expect("write root module");
    for (symbol, line) in [("Customer", 3usize), ("load", 5usize)] {
        let reference = format!("customerStore.{symbol}");
        let offset = source.find(&reference).expect("reference") + "customerStore.".len();
        let output = Command::new(env!("CARGO_BIN_EXE_lume"))
            .arg("definition")
            .arg(&source_path)
            .arg("--offset")
            .arg(offset.to_string())
            .env(
                "LUME_STDLIB",
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../stdlib"),
            )
            .output()
            .expect("run definition command");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("definition output should be JSON");
        assert_eq!(
            PathBuf::from(value["path"].as_str().expect("definition path")),
            fs::canonicalize(&storage).expect("storage path should resolve"),
            "wrong target for {symbol}"
        );
        assert_eq!(value["line"], line, "wrong target line for {symbol}");
    }
    fs::remove_dir_all(directory).expect("remove temporary directory");
}

#[test]
fn definition_command_returns_null_when_cursor_is_not_on_a_type() {
    let directory = temp_directory("no-target");
    let source_path = directory.join("app.lum");
    let source = "module app\n\nfn main() {\n    value = 42\n}\n";
    fs::write(&source_path, source).expect("source should be writable");
    let offset = source.find("42").expect("literal");
    let output = Command::new(env!("CARGO_BIN_EXE_lume"))
        .arg("definition")
        .arg(&source_path)
        .arg("--offset")
        .arg(offset.to_string())
        .env(
            "LUME_STDLIB",
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../stdlib"),
        )
        .output()
        .expect("lume definition should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "null");

    fs::remove_dir_all(directory).expect("temporary directory should be removable");
}

#[test]
fn definition_command_opens_native_library_sources() {
    let directory = temp_directory("native-library");
    let source_path = directory.join("app.lum");
    let source = concat!(
        "module app\n",
        "use lume/json/{JsonValue}\n\n",
        "fn identity(value JsonValue) JsonValue = value\n",
    );
    fs::write(&source_path, source).expect("source should be writable");
    let offset = source.find("value JsonValue").expect("type use") + "value ".len();
    let output = Command::new(env!("CARGO_BIN_EXE_lume"))
        .arg("definition")
        .arg(&source_path)
        .arg("--offset")
        .arg(offset.to_string())
        .env(
            "LUME_STDLIB",
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../stdlib"),
        )
        .output()
        .expect("lume definition should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("definition output should be JSON");
    assert_eq!(
        PathBuf::from(value["path"].as_str().expect("definition path")),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/native_json.lum")
    );
    assert_eq!(value["line"], 14);

    fs::remove_dir_all(directory).expect("temporary directory should be removable");
}

#[test]
fn definition_command_follows_values_functions_and_members() {
    let directory = temp_directory("values-and-members");
    let models = directory.join("models.lum");
    fs::write(
        &models,
        concat!(
            "module models\n\n",
            "public shape User {\n",
            "    name Str\n",
            "    get label Str = name\n",
            "}\n",
        ),
    )
    .expect("models module should be writable");
    let helpers = directory.join("helpers.lum");
    fs::write(
        &helpers,
        concat!(
            "module helpers\n",
            "use models/{User}\n\n",
            "public fn copyUser(value User) User = User { name: value.name }\n",
        ),
    )
    .expect("helpers module should be writable");
    let source_path = directory.join("app.lum");
    let source = concat!(
        "module app\n",
        "use models/{User}\n",
        "use helpers/{copyUser}\n\n",
        "fn render(value User) Str {\n",
        "    copied User = copyUser(value)\n",
        "    users [User] = [copied]\n",
        "    users.add(copied)\n",
        "    for user <- users {\n",
        "        println(user.name)\n",
        "    }\n",
        "    copied.name + copied.label\n",
        "}\n",
    );
    fs::write(&source_path, source).expect("root module should be writable");
    let stdlib = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../stdlib/vector.lum");

    let cases = [
        ("helpers/{copyUser", "copyUser", &helpers, 4usize),
        ("copyUser(value)", "copyUser", &helpers, 4usize),
        ("copyUser(value)", "value", &source_path, 5usize),
        ("copied.name", "copied", &source_path, 6usize),
        ("copied.name", "name", &models, 4usize),
        ("copied.label", "label", &models, 5usize),
        ("users.add", "add", &stdlib, 6usize),
        ("user.name", "name", &models, 4usize),
    ];

    for (context, symbol, expected_path, expected_line) in cases {
        let context_offset = source.find(context).expect("reference context");
        let symbol_offset = context.find(symbol).expect("symbol in context");
        let output = Command::new(env!("CARGO_BIN_EXE_lume"))
            .arg("definition")
            .arg(&source_path)
            .arg("--offset")
            .arg((context_offset + symbol_offset).to_string())
            .env(
                "LUME_STDLIB",
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../stdlib"),
            )
            .output()
            .expect("lume definition should run");

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("definition output should be JSON");
        assert_eq!(
            PathBuf::from(value["path"].as_str().expect("definition path")),
            fs::canonicalize(expected_path).expect("expected path should resolve"),
            "wrong target for {context} / {symbol}"
        );
        assert_eq!(
            value["line"], expected_line,
            "wrong target line for {context} / {symbol}"
        );
    }

    fs::remove_dir_all(directory).expect("temporary directory should be removable");
}
