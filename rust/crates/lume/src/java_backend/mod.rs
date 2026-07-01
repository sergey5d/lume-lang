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
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

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
}
