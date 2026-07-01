use std::{env, fs, path::Path, process::ExitCode};

use lume::{
    Diagnostic, JavaBackendOptions, LocatedDiagnostic, SourceFile, check_path, generate_java_path,
    lex, parse_program, render_diagnostic, render_path_diagnostic, run_path,
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
        "check" => check_command(&mut args),
        "run" => run_command(&mut args),
        "java" => java_command(&mut args),
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

    let requested_entry = args.next();
    match run_path(&path, requested_entry.as_deref()) {
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

fn java_command(args: &mut impl Iterator<Item = String>) -> ExitCode {
    let path = match read_path_arg(args, "java") {
        Ok(path) => path,
        Err(code) => return code,
    };
    let out = match read_java_out_arg(args) {
        Ok(out) => out,
        Err(code) => return code,
    };

    match generate_java_path(&path, JavaBackendOptions::new(out)) {
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
    eprintln!("  lume check <file>");
    eprintln!("  lume run <file> [entry]");
    eprintln!("  lume java <file> --out <dir>");
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

fn read_java_out_arg(args: &mut impl Iterator<Item = String>) -> Result<String, ExitCode> {
    let Some(flag) = args.next() else {
        eprintln!("missing --out <dir> for 'java'");
        print_usage();
        return Err(ExitCode::from(2));
    };
    if flag != "--out" {
        eprintln!("unknown argument '{flag}' for 'java'");
        print_usage();
        return Err(ExitCode::from(2));
    }

    let Some(out) = args.next() else {
        eprintln!("missing directory after --out for 'java'");
        print_usage();
        return Err(ExitCode::from(2));
    };
    if let Some(extra) = args.next() {
        eprintln!("unknown argument '{extra}' for 'java'");
        print_usage();
        return Err(ExitCode::from(2));
    }

    Ok(out)
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
    for diagnostic in diagnostics {
        print_diagnostic(&file.name, Some(&file.text), diagnostic);
    }
}

fn print_path_diagnostics(diagnostics: &[LocatedDiagnostic]) {
    for located in diagnostics {
        eprintln!(
            "{}",
            render_path_diagnostic(Path::new(&located.path), &located.diagnostic)
        );
    }
}

fn print_diagnostic(path: &str, source: Option<&str>, diagnostic: &Diagnostic) {
    eprintln!("{}", render_diagnostic(path, source, diagnostic));
}
