use super::*;

fn project(source: &str) -> Project {
    Project {
        entry: "main.lum".into(),
        files: BTreeMap::from([("main.lum".into(), source.into())]),
    }
}

fn run(source: &str) -> PathRunResult {
    run_browser_project(&project(source), None, RunOptions::browser())
}

#[test]
fn virtual_modules_preserve_imports_visibility_and_aliases() {
    let mut project = project("use lib/math as math\nfn main() { println(math.double(21)) }");
    project.files.insert(
        "lib/math.lum".into(),
        "module lib/math\npublic fn double(value Int) Int = value * 2".into(),
    );
    let result = run_browser_project(&project, None, RunOptions::browser());
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    assert_eq!(result.output, "42\n");
    project.files.insert(
        "lib/math.lum".into(),
        "module lib/math\nfn double(value Int) Int = value * 2".into(),
    );
    assert!(!check_browser_project(&project).diagnostics.is_empty());
}

#[test]
fn virtual_project_errors_are_structured_and_isolated() {
    let mut project = project("use lib/math/{double}\nfn main() { println(double(21)) }");
    project.files.insert(
        "lib/math.lum".into(),
        "module lib/math\npublic fn double(value Int) Int = unknown".into(),
    );
    let result = check_browser_project(&project);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|error| error.path.ends_with("lib/math.lum")
                && error.diagnostic.code == "undefined_name"),
        "{result:#?}"
    );
    project
        .files
        .insert("lib/math.lum".into(), "fn broken( {".into());
    assert!(
        check_browser_project(&project)
            .diagnostics
            .iter()
            .any(|error| error.path == "lib/math.lum")
    );
    assert_eq!(
        check_browser_project(&Project {
            entry: "../main.lum".into(),
            files: BTreeMap::new()
        })
        .diagnostics[0]
            .diagnostic
            .code,
        "invalid_project_path"
    );
    assert_eq!(
        check_browser_project(&self::project("use missing\nfn main() {}")).diagnostics[0]
            .diagnostic
            .code,
        "module_load_error"
    );
}

#[test]
fn browser_stdlib_and_host_inputs_work_without_files() {
    let mut options = RunOptions::browser();
    options.args = vec!["first".into()];
    options.environment = Some(BTreeMap::from([("MODE".into(), "browser".into())]));
    let result = run_browser_project(
        &project(
            r#"
use lume/json/{Json}
use lume/regex/{Regex}
fn main() {
    println("  hello  ".trim().toUpper(), Int.parse("7")!, 1.25d + 0.25d)
    println(Regex.compile("a+")!.matches("aaa"), Json.decode[Int]("42")!)
    println(Vector.fill(3, 7).size, OS.args[0], OS.env["MODE"]!)
}
"#,
        ),
        None,
        options,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    assert_eq!(result.output, "HELLO 7 1.50\ntrue 42\n3 first browser\n");
    let empty = run("fn main() { println(OS.env.size) }");
    assert_eq!(empty.output, "0\n");
}

#[test]
fn browser_rejects_file_access_before_side_effects_but_not_source_names() {
    let result = run("fn main() {\n println(\"must not run\")\n File.readText(\"secret\")\n }");
    assert_eq!(result.output, "");
    assert!(
        result
            .diagnostics
            .iter()
            .any(|error| error.diagnostic.code == "unsupported_target"),
        "{result:#?}"
    );
    let result = run(
        "object File { fn readText(path Str) Str = path }\nfn main() { println(File.readText(\"local\")) }",
    );
    assert!(result.diagnostics.is_empty(), "{result:#?}");
    assert_eq!(result.output, "local\n");
}

#[test]
fn execution_limits_handle_loops_recursion_output_and_allocations() {
    for source in [
        "fn main() { while true {} }",
        "fn again() { again() }\nfn main() { again() }",
        "fn main() { while true { println(\"spam\") } }",
        "fn main() { values = Vector.fill(Int.MAX, 1) }",
        "fn main() { text = \"x\".repeat(Int.MAX) }",
        "fn main() { values = Vector.from(Range(0, Int.MAX)) }",
    ] {
        let mut options = RunOptions::browser();
        let limits = options.limits.as_mut().unwrap();
        limits.steps = 1000;
        limits.call_depth = 12;
        limits.output_bytes = 100;
        let result = run_browser_project(&project(source), None, options);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|error| error.diagnostic.code == "execution_limit"
                    || error.diagnostic.message.contains("too large")),
            "{source}: {result:#?}"
        );
        assert!(result.output.len() <= 100);
    }
    assert_eq!(run("fn main() { println(42) }").output, "42\n");
}

#[test]
fn limit_unwinding_runs_bounded_cleanup() {
    let mut options = RunOptions::browser();
    options.limits.as_mut().unwrap().steps = 50;
    let result = run_browser_project(
        &project("fn main() {\n defer println(\"cleanup\")\n while true {}\n }"),
        None,
        options,
    );
    assert_eq!(result.output, "cleanup\n");
    assert_eq!(result.diagnostics[0].diagnostic.code, "execution_limit");
    let result = run("fn main() {\n defer { while true {} }\n while true {}\n }");
    assert_eq!(result.diagnostics[0].diagnostic.code, "execution_limit");
}

#[test]
fn unicode_diagnostics_keep_utf8_offsets() {
    let source = "fn main() {\n text = \"😀\"\n println(missing)\n }";
    let result = check_browser_project(&project(source));
    let error = result
        .diagnostics
        .iter()
        .find(|error| error.diagnostic.code == "undefined_name")
        .unwrap();
    assert_eq!(
        &source[error.diagnostic.span.start..error.diagnostic.span.end],
        "missing"
    );
}

#[test]
fn runtime_errors_and_host_rejections_point_to_imported_sources() {
    let mut project = project("use helpers/{fail}\nfn main() { fail() }");
    project.files.insert(
        "helpers.lum".into(),
        "module helpers\npublic fn fail() { panic(\"from helpers\") }".into(),
    );
    let result = run_browser_project(&project, None, RunOptions::browser());
    assert_eq!(result.diagnostics[0].path, "/helpers.lum", "{result:#?}");
    project.files.insert(
        "helpers.lum".into(),
        "module helpers\npublic fn fail() { [1].forEach(value => panic(\"from closure\")) }".into(),
    );
    let result = run_browser_project(&project, None, RunOptions::browser());
    assert_eq!(result.diagnostics[0].path, "/helpers.lum", "{result:#?}");
    project.files.insert(
        "helpers.lum".into(),
        "module helpers\npublic fn fail() { File.readText(\"secret\") }".into(),
    );
    let result = check_browser_project(&project);
    assert_eq!(result.diagnostics[0].path, "/helpers.lum", "{result:#?}");
}
