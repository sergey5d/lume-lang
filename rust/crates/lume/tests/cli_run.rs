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

#[test]
fn run_command_supports_synchronous_file_io() {
    let path = temp_source(
        "file-io",
        r#"
def main() Unit {
    path = OS.args[0]
    println(File.readText(path)!.size)

    stream = File.open(path)!
    println(stream.path == path, stream.position, stream.closed)
    println(stream.read(5)!.size, stream.position)
    println(stream.seek(-5, SeekFrom.End)!, stream.readToEnd()!.size)
    streamClosed = stream.close()!
    println(stream.closed)
    match stream.read(1) {
        case Ok(_) => println("open")
        case Err(Closed { path: _ }) => println("closed")
        case Err(_) => println("error")
    }

    reader = File.openText(path)!
    first = reader.readLine()!!
    blank = reader.readLine()!!
    last = reader.readLine()!!
    end = reader.readLine()!
    println(first, blank.size, last, end.isEmpty)
    readerClosed = reader.close()!

    match File.readText(OS.args[1]) {
        case Err(InvalidEncoding { offset }) => println("invalid", offset)
        case _ => println("unexpected")
    }

    invalidReader = File.openText(OS.args[1])!
    match invalidReader.readLine() {
        case Err(InvalidEncoding { offset }) => println("stream invalid", offset)
        case _ => println("unexpected stream")
    }
}
"#,
    );
    let data = path.with_extension("txt");
    let invalid = path.with_extension("invalid");
    fs::write(&data, "alpha\n\nomega").expect("temporary data should be writable");
    fs::write(&invalid, [0xff]).expect("temporary invalid data should be writable");

    let output = Command::new(env!("CARGO_BIN_EXE_lume"))
        .arg("run")
        .arg(&path)
        .arg("main")
        .arg("--")
        .arg(&data)
        .arg(&invalid)
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
        "12\ntrue 0 false\n5 5\n7 5\ntrue\nclosed\nalpha 0 omega true\ninvalid 0\nstream invalid 0\n"
    );
    fs::remove_file(invalid).expect("temporary invalid data should be removable");
    fs::remove_file(data).expect("temporary data should be removable");
    fs::remove_file(path).expect("temporary source should be removable");
}

#[test]
fn run_command_supports_native_json() {
    let path = temp_source(
        "native-json",
        r#"
module demo/native_json

use lume/json/{Json, JsonIgnore, JsonName, JsonValue}

class User {
    @JsonName { value: "user_name" }
    name Str

    age Int
    tags [Str]
    note Str?

    @JsonIgnore
    ignored Str = "skip me"

    private token Str = "secret"
}

def main() Unit {
    user = User {
        name: "Ada"
        age: 42
        tags: ["admin", "owner"]
        note: Some("ready")
    }
    println(Json.stringify(user))

    manual = Json.obj(
        Json.field("ok", Json.bool(true)),
        Json.field("items", Json.array(Json.str("a"), Json.int(2)))
    )
    println(Json.stringify(manual))
    println(Json.stringify(["one": 1, "two": 2]))

    decoded = Json.decode[User](
        """{"user_name":"Bob","age":31,"tags":["a","b"],"note":null}"""
    )!
    println(decoded.name, decoded.age, decoded.tags.size, decoded.note.isEmpty)

    numbers = Json.decode[Vector[Int]]("[1,2,3]")!
    println(numbers.size, numbers[0], numbers[2])

    parsed JsonValue = Json.decode[JsonValue]("{\"nested\":[true,null]}")!
    println(Json.stringify(parsed))
    println(Json.decode[User]("{") is Err)
}
"#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_lume"))
        .arg("run")
        .arg(&path)
        .arg("main")
        .env("LUME_STDLIB", stdlib_dir())
        .output()
        .expect("lume run should run native JSON");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        "{\"user_name\":\"Ada\",\"age\":42,\"tags\":[\"admin\",\"owner\"],\"note\":\"ready\"}\n\
{\"ok\":true,\"items\":[\"a\",2]}\n\
{\"one\":1,\"two\":2}\n\
Bob 31 2 true\n\
3 1 3\n\
{\"nested\":[true,null]}\n\
true\n"
    );
    fs::remove_file(path).expect("temporary source should be removable");
}
