use std::{
    fs,
    path::{Path, PathBuf},
};

mod emit;

use crate::{backend::build_backend_bundle, resolver::LocatedDiagnostic};

#[derive(Debug, Clone)]
pub struct JavaBackendOptions {
    pub output_dir: PathBuf,
}

impl JavaBackendOptions {
    pub fn new(output_dir: impl Into<PathBuf>) -> Self {
        Self {
            output_dir: output_dir.into(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct JavaBackendResult {
    pub diagnostics: Vec<LocatedDiagnostic>,
    pub written_files: Vec<PathBuf>,
}

pub fn generate_java_path(
    path: impl AsRef<Path>,
    options: JavaBackendOptions,
) -> Result<JavaBackendResult, String> {
    let bundled = build_backend_bundle(path)?;
    if !bundled.diagnostics.is_empty() {
        return Ok(JavaBackendResult {
            diagnostics: bundled.diagnostics,
            written_files: Vec::new(),
        });
    }

    let bundle = bundled
        .bundle
        .expect("backend bundle after successful build");
    let mut written_files = Vec::new();
    for source in emit::render_declaration_skeletons(&bundle) {
        let path = options.output_dir.join(source.relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("create {}: {err}", parent.display()))?;
        }
        fs::write(&path, source.contents)
            .map_err(|err| format!("write {}: {err}", path.display()))?;
        written_files.push(path);
    }

    Ok(JavaBackendResult {
        diagnostics: Vec::new(),
        written_files,
    })
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::run_path;

    #[test]
    fn generates_declaration_skeletons_for_checked_program() {
        let temp = temp_path("lume-java-generate");
        let source = temp.join("main.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/app

shape Point {
    x Int
    y Int
}

class User {
    name Str
    age Int
}

class RuntimeBox {
    items [Int]
    names Set[Str]
    index Map[Str, [Int]]
    maybe Option[Str]
    result Result[Int, Str]
    either Either[Str, Int]
    pair (Int, Str)
}

single Routes {
    health Str = "/health"

    def healthPath() Str = this.health
}

interface Named {
    def name() Str
}

enum Maybe[T] {
    case None
    case Some {
        value T
    }
}

annotation Route {
    path Str
}

def main() Unit {
    println("hello")
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(result.diagnostics.is_empty());
        assert!(
            result
                .written_files
                .iter()
                .any(|path| path.ends_with("demo/app/AppModule.java"))
        );
        assert!(
            result
                .written_files
                .iter()
                .any(|path| path.ends_with("demo/app/Point.java"))
        );
        assert!(
            result
                .written_files
                .iter()
                .any(|path| path.ends_with("demo/app/User.java"))
        );
        assert!(
            result
                .written_files
                .iter()
                .any(|path| path.ends_with("demo/app/RuntimeBox.java"))
        );
        assert!(
            result
                .written_files
                .iter()
                .any(|path| path.ends_with("demo/app/Routes.java"))
        );
        assert!(
            result
                .written_files
                .iter()
                .any(|path| path.ends_with("demo/app/Named.java"))
        );
        assert!(
            result
                .written_files
                .iter()
                .any(|path| path.ends_with("demo/app/Maybe.java"))
        );
        assert!(
            result
                .written_files
                .iter()
                .any(|path| path.ends_with("demo/app/Route.java"))
        );

        let module = fs::read_to_string(out.join("demo/app/AppModule.java")).expect("read module");
        assert!(module.contains("package demo.app;"));
        assert!(module.contains("final class AppModule"));
        assert!(module.contains("static void main()"));

        let shape = fs::read_to_string(out.join("demo/app/Point.java")).expect("read shape");
        assert!(shape.contains("record Point(Long x, Long y)"));

        let class = fs::read_to_string(out.join("demo/app/User.java")).expect("read class");
        assert!(class.contains("class User"));
        assert!(class.contains("String name;"));
        assert!(class.contains("Long age;"));

        let runtime_box =
            fs::read_to_string(out.join("demo/app/RuntimeBox.java")).expect("read runtime box");
        assert!(runtime_box.contains("lume.runtime.LumeList<Long> items;"));
        assert!(runtime_box.contains("lume.runtime.LumeSet<String> names;"));
        assert!(
            runtime_box
                .contains("lume.runtime.LumeMap<String, lume.runtime.LumeList<Long>> index;")
        );
        assert!(runtime_box.contains("lume.runtime.Option<String> maybe;"));
        assert!(runtime_box.contains("lume.runtime.Result<Long, String> result;"));
        assert!(runtime_box.contains("lume.runtime.Either<String, Long> either;"));
        assert!(runtime_box.contains("lume.runtime.Tuple2<Long, String> pair;"));

        let single = fs::read_to_string(out.join("demo/app/Routes.java")).expect("read single");
        assert!(single.contains("final class Routes"));
        assert!(single.contains("static final Routes INSTANCE"));
        assert!(single.contains("String healthPath()"));

        let interface =
            fs::read_to_string(out.join("demo/app/Named.java")).expect("read interface");
        assert!(interface.contains("interface Named"));
        assert!(interface.contains("String name();"));

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn does_not_write_java_when_lume_has_diagnostics() {
        let temp = temp_path("lume-java-invalid");
        let source = temp.join("broken.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(&source, "def main() { missing() }").expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(!result.diagnostics.is_empty());
        assert!(result.written_files.is_empty());
        assert!(!out.exists());

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn resolves_java_type_imports_for_generation() {
        let temp = temp_path("lume-java-imports");
        let source = temp.join("external.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/external

use java/time/Instant
use java/time/{Duration as JDuration}

class Event {
    at Instant
    duration JDuration
}

def main() Unit {
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(result.diagnostics.is_empty());
        let event = fs::read_to_string(out.join("demo/external/Event.java")).expect("read event");
        assert!(event.contains("java.time.Instant at;"));
        assert!(event.contains("java.time.Duration duration;"));
        assert!(
            event.contains("Event(java.time.Instant at_arg0, java.time.Duration duration_arg1)")
        );
        assert!(!out.join("demo/external/Instant.java").exists());
        assert!(!out.join("demo/external/JDuration.java").exists());

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn emits_mvp_function_bodies_for_supported_ir() {
        let temp = temp_path("lume-java-bodies");
        let source = temp.join("body.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/body

def add(left Int, right Int) Int {
    result Int = left + right
    result
}

def choose(flag Bool) Int {
    if flag {
        10
    } else {
        20
    }
}

def main() Unit {
    value Int = add(2, 3)
    println(value)
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(result.diagnostics.is_empty());
        let module =
            fs::read_to_string(out.join("demo/body/BodyModule.java")).expect("read module");
        assert!(!module.contains("UnsupportedOperationException"));
        assert!(module.contains("static Long add(Long left_0, Long right_1)"));
        assert!(module.contains("tmp3_3 = (left_0 + right_1);"));
        assert!(module.contains("result_2 = ((Long) tmp3_3);"));
        assert!(module.contains("return result_2;"));
        assert!(module.contains("if (flag_0)"));
        assert!(module.contains("return ((Long) tmp1_1);"));
        assert!(module.contains("tmp1_1 = add(2L, 3L);"));
        assert!(module.contains("value_0 = ((Long) tmp1_1);"));
        assert!(module.contains("tmp2_2 = lume.runtime.LumeRuntime.println(value_0);"));

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_matches_interpreter_for_supported_program() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java parity test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-parity");
        let source = temp.join("parity.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/parity

def add(left Int, right Int) Int {
    result Int = left + right
    result
}

def main() Int {
    value Int = add(2, 3)
    println(value)

    if value > 4 {
        println("bigger")
    } else {
        println("smaller")
    }

    0
}
"#,
        )
        .expect("write source");

        let interpreted = run_path(&source, None).expect("run interpreter");
        assert!(interpreted.diagnostics.is_empty());
        let expected = interpreter_stdout(interpreted);

        let generated =
            generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate java");
        assert!(generated.diagnostics.is_empty());

        let runner = out.join("demo/parity/JavaParityRunner.java");
        fs::write(
            &runner,
            r#"
package demo.parity;

final class JavaParityRunner {
    public static void main(String[] args) {
        System.out.println(ParityModule.main());
    }
}
"#,
        )
        .expect("write runner");

        let mut sources = java_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );

        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.parity.JavaParityRunner"),
            "java",
        );
        let actual = String::from_utf8(output.stdout).expect("java stdout utf8");
        assert_eq!(actual, expected);

        let _ = fs::remove_dir_all(temp);
    }

    fn temp_path(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time after epoch")
            .as_nanos();
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest_dir
            .parent()
            .and_then(|crates_dir| crates_dir.parent())
            .expect("rust workspace root")
            .join("target")
            .join(format!("{prefix}-{nanos}"))
    }

    fn interpreter_stdout(result: crate::PathRunResult) -> String {
        let mut output = result.output;
        if let Some(value) = result.return_value {
            output.push_str(&value);
            output.push('\n');
        }
        output
    }

    fn command_available(name: &str) -> bool {
        Command::new(name).arg("-version").output().is_ok()
    }

    fn java_runtime_sources() -> Vec<PathBuf> {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(|crates_dir| crates_dir.parent())
            .and_then(|rust_dir| rust_dir.parent())
            .expect("repo root");
        let runtime_dir = repo_root.join("java_runtime/src/main/java/lume/runtime");
        let mut sources = Vec::new();
        collect_java_sources(&runtime_dir, &mut sources).expect("collect runtime java");
        sources
    }

    fn collect_java_sources(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                collect_java_sources(&path, out)?;
            } else if path.extension().is_some_and(|ext| ext == "java") {
                out.push(path);
            }
        }
        Ok(())
    }

    fn run_checked(command: &mut Command, name: &str) -> std::process::Output {
        let output = command
            .output()
            .unwrap_or_else(|err| panic!("run {name}: {err}"));
        if !output.status.success() {
            panic!(
                "{name} failed\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        output
    }
}
