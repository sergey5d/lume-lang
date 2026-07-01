use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

mod emit;

use crate::{
    Diagnostic,
    backend::{ExternalDescriptors, bundle::build_backend_bundle_with_load_options},
    resolver::{LocatedDiagnostic, ModuleLoadOptions, load_module_graph_with_options},
};

#[derive(Debug, Clone)]
pub struct JavaBackendOptions {
    pub output_dir: PathBuf,
    pub classpath: Vec<PathBuf>,
}

impl JavaBackendOptions {
    pub fn new(output_dir: impl Into<PathBuf>) -> Self {
        Self {
            output_dir: output_dir.into(),
            classpath: Vec::new(),
        }
    }

    pub fn with_classpath_entry(mut self, entry: impl Into<PathBuf>) -> Self {
        self.classpath.push(entry.into());
        self
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
    let path = path.as_ref();
    let discovery_options = ModuleLoadOptions {
        allow_unresolved_java_imports: true,
        java_external_type_params: HashMap::new(),
    };
    let (discovery_graph, _) = load_module_graph_with_options(path, &discovery_options)?;
    let discovered_externals = ExternalDescriptors::from_module_graph(&discovery_graph);
    let external_resolution = resolve_external_classes(&discovered_externals, &options)?;
    if !external_resolution.diagnostics.is_empty() {
        return Ok(JavaBackendResult {
            diagnostics: external_resolution.diagnostics,
            written_files: Vec::new(),
        });
    }

    let load_options = ModuleLoadOptions {
        allow_unresolved_java_imports: true,
        java_external_type_params: external_resolution.type_params,
    };
    let bundled = build_backend_bundle_with_load_options(path, &load_options)?;
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

#[derive(Debug, Clone, Default)]
struct ExternalClassResolution {
    diagnostics: Vec<LocatedDiagnostic>,
    type_params: HashMap<String, Vec<String>>,
}

fn resolve_external_classes(
    externals: &ExternalDescriptors,
    options: &JavaBackendOptions,
) -> Result<ExternalClassResolution, String> {
    let index = JavaClasspathIndex::from_entries(&options.classpath)?;
    let classpath = java_classpath(&options.classpath)?;
    let mut diagnostics = Vec::new();
    let mut type_params = HashMap::new();
    for symbol in &externals.symbols {
        if !matches!(symbol.kind, crate::backend::ExternalSymbolKind::Type) {
            continue;
        }
        if !index.could_contain(&symbol.qualified_name) {
            diagnostics.push(missing_java_class_diagnostic(symbol));
            continue;
        }
        let Some(descriptor) = inspect_java_class(classpath.as_deref(), &symbol.qualified_name)?
        else {
            diagnostics.push(missing_java_class_diagnostic(symbol));
            continue;
        };
        type_params.insert(symbol.qualified_name.clone(), descriptor.type_params);
    }
    Ok(ExternalClassResolution {
        diagnostics,
        type_params,
    })
}

fn missing_java_class_diagnostic(symbol: &crate::backend::ExternalSymbol) -> LocatedDiagnostic {
    LocatedDiagnostic {
        path: symbol.source_path.clone(),
        diagnostic: Diagnostic::error(
            "missing_java_class",
            format!(
                "Java class '{}' is not available on the provided classpath",
                symbol.qualified_name
            ),
            symbol.span,
        )
        .with_label("class imported here")
        .with_help("add the jar or classes directory with --classpath <path>"),
    }
}

#[derive(Debug, Clone, Default)]
struct JavaClasspathIndex {
    classes: HashSet<String>,
    indexed_entries: bool,
}

impl JavaClasspathIndex {
    fn from_entries(entries: &[PathBuf]) -> Result<Self, String> {
        let mut index = Self::default();
        for entry in entries {
            index.indexed_entries = true;
            if entry.is_dir() {
                index_class_dir(entry, entry, &mut index.classes)?;
            } else if entry.extension().is_some_and(|ext| ext == "jar") {
                index_jar(entry, &mut index.classes)?;
            }
        }
        Ok(index)
    }

    fn could_contain(&self, qualified_name: &str) -> bool {
        !self.indexed_entries
            || qualified_name.starts_with("java.")
            || self.classes.contains(qualified_name)
    }
}

#[derive(Debug, Clone)]
struct JavaClassDescriptor {
    type_params: Vec<String>,
}

fn inspect_java_class(
    classpath: Option<&std::ffi::OsStr>,
    qualified_name: &str,
) -> Result<Option<JavaClassDescriptor>, String> {
    let mut command = Command::new("javap");
    if let Some(classpath) = classpath {
        command.arg("-classpath").arg(classpath);
    }
    let output = command
        .arg("-public")
        .arg(qualified_name)
        .output()
        .map_err(|err| format!("run javap to inspect Java classpath: {err}"))?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(Some(JavaClassDescriptor {
        type_params: parse_javap_type_params(&stdout, qualified_name),
    }))
}

fn java_classpath(entries: &[PathBuf]) -> Result<Option<std::ffi::OsString>, String> {
    if entries.is_empty() {
        return Ok(None);
    }
    env::join_paths(entries)
        .map(Some)
        .map_err(|err| format!("build Java classpath: {err}"))
}

fn parse_javap_type_params(output: &str, qualified_name: &str) -> Vec<String> {
    let prefix = format!("{qualified_name}<");
    let Some(line) = output
        .lines()
        .find(|line| line.contains(" class ") || line.contains(" interface "))
    else {
        return Vec::new();
    };
    let Some(start) = line.find(&prefix).map(|index| index + prefix.len()) else {
        return Vec::new();
    };
    let Some(end) = line[start..].find('>').map(|index| start + index) else {
        return Vec::new();
    };
    line[start..end]
        .split(',')
        .filter_map(|param| {
            param
                .split_whitespace()
                .next()
                .filter(|name| !name.is_empty())
                .map(str::to_string)
        })
        .collect()
}

fn index_jar(path: &Path, classes: &mut HashSet<String>) -> Result<(), String> {
    let output = Command::new("jar")
        .arg("tf")
        .arg(path)
        .output()
        .map_err(|err| format!("run jar to inspect {}: {err}", path.display()))?;
    if !output.status.success() {
        return Err(format!(
            "inspect jar {}\n{}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(class_name) = class_name_from_relative_path(line) {
            classes.insert(class_name);
        }
    }
    Ok(())
}

fn index_class_dir(root: &Path, dir: &Path, classes: &mut HashSet<String>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|err| format!("read {}: {err}", dir.display()))? {
        let entry = entry.map_err(|err| format!("read {} entry: {err}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            index_class_dir(root, &path, classes)?;
        } else if path.extension().is_some_and(|ext| ext == "class") {
            let relative = path
                .strip_prefix(root)
                .map_err(|err| format!("index class {}: {err}", path.display()))?;
            let relative = relative.to_string_lossy().replace('\\', "/");
            if let Some(class_name) = class_name_from_relative_path(&relative) {
                classes.insert(class_name);
            }
        }
    }
    Ok(())
}

fn class_name_from_relative_path(path: &str) -> Option<String> {
    let path = path.strip_suffix(".class")?;
    if path == "module-info" || path.ends_with("/module-info") {
        return None;
    }
    Some(path.replace('/', "."))
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
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
                .any(|path| path.ends_with("demo/app/AppMain.java"))
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

        let runner = fs::read_to_string(out.join("demo/app/AppMain.java")).expect("read runner");
        assert!(runner.contains("public static void main(String[] args)"));
        assert!(runner.contains("AppModule.main();"));

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
    fn validates_and_compiles_third_party_jar_imports() {
        if !command_available("javac")
            || !command_available("java")
            || !command_available("jar")
            || !command_available("javap")
        {
            eprintln!("skipping Java jar import test because a JDK tool is not available");
            return;
        }

        let temp = temp_path("lume-java-jar-import");
        let source = temp.join("jar_import.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        let jar = create_widget_jar(&temp);
        fs::write(
            &source,
            r#"
module demo/jaruse

use java/util/ArrayList
use third/party/{Widget, GenericBox}

class Holder {
    widget Widget
    generic GenericBox[Str]
    list ArrayList[Str]
}

def main() Unit {
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(
            &source,
            JavaBackendOptions::new(&out).with_classpath_entry(&jar),
        )
        .expect("generate java");

        assert!(result.diagnostics.is_empty());
        let holder = fs::read_to_string(out.join("demo/jaruse/Holder.java")).expect("read holder");
        assert!(holder.contains("third.party.Widget widget;"));
        assert!(holder.contains("third.party.GenericBox<String> generic;"));
        assert!(holder.contains("java.util.ArrayList<String> list;"));

        let mut sources = java_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac")
                .arg("-cp")
                .arg(&jar)
                .arg("-d")
                .arg(&classes)
                .args(&sources),
            "javac",
        );

        let runtime_classpath =
            env::join_paths([classes.as_path(), jar.as_path()]).expect("join runtime classpath");
        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(runtime_classpath)
                .arg("demo.jaruse.JaruseMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            ""
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn reports_missing_java_class_from_classpath() {
        if !command_available("javap") || !command_available("javac") || !command_available("jar") {
            eprintln!("skipping missing Java class test because a JDK tool is not available");
            return;
        }

        let temp = temp_path("lume-java-missing-class");
        let source = temp.join("missing_import.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        let jar = create_widget_jar(&temp);
        fs::write(
            &source,
            r#"
module demo/missing

use third/party/Missing

class Holder {
    missing Missing
}

def main() Unit {
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(
            &source,
            JavaBackendOptions::new(&out).with_classpath_entry(&jar),
        )
        .expect("generate java");

        assert!(result.written_files.is_empty());
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].diagnostic.code, "missing_java_class");
        assert!(
            result.diagnostics[0]
                .diagnostic
                .message
                .contains("third.party.Missing")
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_executable_runs_array_of_rune() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java executable test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-array-rune");
        let source = temp.join("array_rune.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/runarray

def main() Int {
    runes Array[Rune] = Array.ofRune(2)
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

        let module =
            fs::read_to_string(out.join("demo/runarray/RunarrayModule.java")).expect("read module");
        assert!(!module.contains("UnsupportedOperationException"));
        assert!(module.contains("lume.runtime.LumeArray.ofRune(2L)"));

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
                .arg("demo.runarray.RunarrayMain"),
            "java",
        );
        let actual = String::from_utf8(output.stdout).expect("java stdout utf8");
        assert_eq!(actual, expected);

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
                .arg("demo.parity.ParityMain"),
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
        Command::new(name).arg("--version").output().is_ok()
            || Command::new(name).arg("-version").output().is_ok()
    }

    fn create_widget_jar(temp: &Path) -> PathBuf {
        let src_dir = temp.join("java-src/third/party");
        let classes = temp.join("java-classes");
        let jar = temp.join("widget.jar");
        fs::create_dir_all(&src_dir).expect("create java src dir");
        fs::create_dir_all(&classes).expect("create java classes dir");
        let source = src_dir.join("Widget.java");
        fs::write(
            &source,
            r#"
package third.party;

public final class Widget {
}
"#,
        )
        .expect("write widget java source");
        let generic_source = src_dir.join("GenericBox.java");
        fs::write(
            &generic_source,
            r#"
package third.party;

public final class GenericBox<T> {
}
"#,
        )
        .expect("write generic box java source");
        run_checked(
            Command::new("javac")
                .arg("-d")
                .arg(&classes)
                .arg(&source)
                .arg(&generic_source),
            "javac",
        );
        run_checked(
            Command::new("jar")
                .arg("cf")
                .arg(&jar)
                .arg("-C")
                .arg(&classes)
                .arg("."),
            "jar",
        );
        jar
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
