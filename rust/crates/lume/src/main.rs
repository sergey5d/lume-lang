use std::{env, fs, path::Path, process::ExitCode};

use lume::{
    Diagnostic, JavaBackendOptions, LocatedDiagnostic, SourceFile, check_path, format_source,
    generate_java_path, lex, parse_program, render_diagnostic, render_path_diagnostic,
    run_path_with_args, test_path,
};

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_usage();
        return ExitCode::from(2);
    };

    match command.as_str() {
        "tokens" => tokens_command(&mut args),
        "parse" => parse_command(&mut args),
        "fmt" => fmt_command(&mut args),
        "check" => check_command(&mut args),
        "run" => run_command(&mut args),
        "test" => test_command(&mut args),
        "gen" => gen_command(&mut args),
        _ => {
            eprintln!("unknown command '{command}'");
            print_usage();
            ExitCode::from(2)
        }
    }
}

fn tokens_command(args: &mut impl Iterator<Item = String>) -> ExitCode {
    let file = match read_source_arg(args, "tokens") {
        Ok(file) => file,
        Err(code) => return code,
    };

    let result = lex(&file);
    for token in &result.tokens {
        println!(
            "{:>4}:{:<4} {:<16} {}",
            token.span.start_pos.line,
            token.span.start_pos.column,
            format!("{:?}", token.kind),
            token.lexeme.escape_default(),
        );
    }

    if result.has_errors() {
        print_source_diagnostics(&file, &result.diagnostics);
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}

fn parse_command(args: &mut impl Iterator<Item = String>) -> ExitCode {
    let file = match read_source_arg(args, "parse") {
        Ok(file) => file,
        Err(code) => return code,
    };

    let lexed = lex(&file);
    if lexed.has_errors() {
        print_source_diagnostics(&file, &lexed.diagnostics);
        return ExitCode::from(1);
    }

    let parsed = parse_program(&lexed.tokens);
    if !parsed.diagnostics.is_empty() {
        print_source_diagnostics(&file, &parsed.diagnostics);
        return ExitCode::from(1);
    }

    match parsed.program {
        Some(program) => {
            println!("{program:#?}");
            ExitCode::SUCCESS
        }
        None => ExitCode::from(1),
    }
}

fn fmt_command(args: &mut impl Iterator<Item = String>) -> ExitCode {
    let path = match read_path_arg(args, "fmt") {
        Ok(path) => path,
        Err(code) => return code,
    };
    if let Some(argument) = args.next() {
        eprintln!("unexpected argument '{argument}' for 'fmt'");
        print_usage();
        return ExitCode::from(2);
    }

    let file = match read_source_path(path.clone()) {
        Ok(file) => file,
        Err(code) => return code,
    };
    let result = format_source(&file);
    if result.has_errors() {
        print_source_diagnostics(&file, &result.diagnostics);
        return ExitCode::from(1);
    }

    if result.text != file.text
        && let Err(err) = fs::write(&path, result.text)
    {
        eprintln!("write {path}: {err}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn check_command(args: &mut impl Iterator<Item = String>) -> ExitCode {
    let path = match read_path_arg(args, "check") {
        Ok(path) => path,
        Err(code) => return code,
    };

    match check_path(&path) {
        Ok(result) => exit_with_path_diagnostics(&result.diagnostics),
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(1)
        }
    }
}

fn run_command(args: &mut impl Iterator<Item = String>) -> ExitCode {
    let path = match read_path_arg(args, "run") {
        Ok(path) => path,
        Err(code) => return code,
    };

    let remaining = args.collect::<Vec<_>>();
    let separator = remaining.iter().position(|arg| arg == "--");
    let (requested_entry, program_args) = match separator {
        Some(index) if index <= 1 => (
            remaining.first().filter(|_| index == 1).map(String::as_str),
            &remaining[index + 1..],
        ),
        Some(_) => {
            eprintln!("'run' accepts at most one entry name before '--'");
            print_usage();
            return ExitCode::from(2);
        }
        None if remaining.len() <= 1 => (remaining.first().map(String::as_str), &[][..]),
        None => {
            eprintln!("program arguments must follow '--'");
            print_usage();
            return ExitCode::from(2);
        }
    };

    match run_path_with_args(&path, requested_entry, program_args) {
        Ok(result) => {
            if !result.diagnostics.is_empty() {
                print_path_diagnostics(&result.diagnostics);
                return ExitCode::from(1);
            }
            print!("{}", result.output);
            if let Some(value) = result.return_value {
                println!("{value}");
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(1)
        }
    }
}

fn test_command(args: &mut impl Iterator<Item = String>) -> ExitCode {
    let path = match read_path_arg(args, "test") {
        Ok(path) => path,
        Err(code) => return code,
    };

    match test_path(&path) {
        Ok(result) => {
            if !result.diagnostics.is_empty() {
                print_path_diagnostics(&result.diagnostics);
                return ExitCode::from(1);
            }
            print!("{}", result.output);
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(1)
        }
    }
}

fn gen_command(args: &mut impl Iterator<Item = String>) -> ExitCode {
    let path = match read_path_arg(args, "gen") {
        Ok(path) => path,
        Err(code) => return code,
    };
    let options = match read_gen_options(args) {
        Ok(options) => options,
        Err(code) => return code,
    };

    match generate_java_path(&path, options) {
        Ok(result) => {
            if !result.diagnostics.is_empty() {
                print_path_diagnostics(&result.diagnostics);
                return ExitCode::from(1);
            }
            for written in result.written_files {
                println!("wrote {}", written.display());
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(1)
        }
    }
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!("  lume tokens <file>");
    eprintln!("  lume parse <file>");
    eprintln!("  lume fmt <file>");
    eprintln!("  lume check <file>");
    eprintln!("  lume run <file> [entry] [-- <args>...]");
    eprintln!("  lume test <file>");
    eprintln!("  lume gen <file> --out <dir> [--classpath <path>]");
}

fn read_source_arg(
    args: &mut impl Iterator<Item = String>,
    command: &str,
) -> Result<SourceFile, ExitCode> {
    let path = read_path_arg(args, command)?;
    read_source_path(path)
}

fn read_path_arg(
    args: &mut impl Iterator<Item = String>,
    command: &str,
) -> Result<String, ExitCode> {
    let Some(path) = args.next() else {
        eprintln!("missing source file for '{command}'");
        print_usage();
        return Err(ExitCode::from(2));
    };
    Ok(path)
}

fn read_source_path(path: String) -> Result<SourceFile, ExitCode> {
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("read {path}: {err}");
            return Err(ExitCode::from(1));
        }
    };

    Ok(SourceFile::new(path, text))
}

fn read_gen_options(
    args: &mut impl Iterator<Item = String>,
) -> Result<JavaBackendOptions, ExitCode> {
    let mut out = None;
    let mut classpath = Vec::new();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--out" => {
                let Some(value) = args.next() else {
                    eprintln!("missing directory after --out for 'gen'");
                    print_usage();
                    return Err(ExitCode::from(2));
                };
                out = Some(value);
            }
            "--classpath" | "--class-path" | "-cp" => {
                let Some(value) = args.next() else {
                    eprintln!("missing path after {flag} for 'gen'");
                    print_usage();
                    return Err(ExitCode::from(2));
                };
                classpath.extend(env::split_paths(&value));
            }
            _ => {
                eprintln!("unknown argument '{flag}' for 'gen'");
                print_usage();
                return Err(ExitCode::from(2));
            }
        }
    }

    let Some(out) = out else {
        eprintln!("missing --out <dir> for 'gen'");
        print_usage();
        return Err(ExitCode::from(2));
    };
    let mut options = JavaBackendOptions::new(out);
    for entry in classpath {
        options = options.with_classpath_entry(entry);
    }
    Ok(options)
}

fn exit_with_path_diagnostics(diagnostics: &[LocatedDiagnostic]) -> ExitCode {
    if diagnostics.is_empty() {
        ExitCode::SUCCESS
    } else {
        print_path_diagnostics(diagnostics);
        ExitCode::from(1)
    }
}

fn print_source_diagnostics(file: &SourceFile, diagnostics: &[Diagnostic]) {
    let rendered = diagnostics
        .iter()
        .map(|diagnostic| render_diagnostic(&file.name, Some(&file.text), diagnostic))
        .collect::<Vec<_>>()
        .join("\n\n");
    if !rendered.is_empty() {
        eprintln!("{rendered}");
    }
}

fn print_path_diagnostics(diagnostics: &[LocatedDiagnostic]) {
    let rendered = diagnostics
        .iter()
        .map(|located| render_path_diagnostic(Path::new(&located.path), &located.diagnostic))
        .collect::<Vec<_>>()
        .join("\n\n");
    if !rendered.is_empty() {
        eprintln!("{rendered}");
    }
}
