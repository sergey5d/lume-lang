use std::{env, fs, process::ExitCode};

use lume::{Diagnostic, Severity, SourceFile, check_path, lex, parse_program, run_path};

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_usage();
        return ExitCode::from(2);
    };

    match command.as_str() {
        "tokens" => {
            let file = match read_source(&mut args, "tokens") {
                Ok(file) => file,
                Err(code) => return code,
            };
            let path = file.name.clone();
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
                for diagnostic in &result.diagnostics {
                    print_diagnostic(&path, diagnostic);
                }
                return ExitCode::from(1);
            }
            ExitCode::SUCCESS
        }
        "parse" => {
            let file = match read_source(&mut args, "parse") {
                Ok(file) => file,
                Err(code) => return code,
            };
            let path = file.name.clone();
            let lexed = lex(&file);
            if lexed.has_errors() {
                for diagnostic in &lexed.diagnostics {
                    print_diagnostic(&path, diagnostic);
                }
                return ExitCode::from(1);
            }

            let parsed = parse_program(&lexed.tokens);
            if !parsed.diagnostics.is_empty() {
                for diagnostic in &parsed.diagnostics {
                    print_diagnostic(&path, diagnostic);
                }
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
        "check" => {
            let Some(path) = args.next() else {
                eprintln!("missing source file for 'check'");
                print_usage();
                return ExitCode::from(2);
            };

            match check_path(&path) {
                Ok(result) => {
                    if result.diagnostics.is_empty() {
                        ExitCode::SUCCESS
                    } else {
                        for located in &result.diagnostics {
                            print_diagnostic(&located.path, &located.diagnostic);
                        }
                        ExitCode::from(1)
                    }
                }
                Err(err) => {
                    eprintln!("{err}");
                    ExitCode::from(1)
                }
            }
        }
        "run" => {
            let Some(path) = args.next() else {
                eprintln!("missing source file for 'run'");
                print_usage();
                return ExitCode::from(2);
            };
            let requested_entry = args.next();
            match run_path(&path, requested_entry.as_deref()) {
                Ok(result) => {
                    if !result.diagnostics.is_empty() {
                        for located in &result.diagnostics {
                            print_diagnostic(&located.path, &located.diagnostic);
                        }
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
        _ => {
            eprintln!("unknown command '{command}'");
            print_usage();
            ExitCode::from(2)
        }
    }
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!("  lume tokens <file>");
    eprintln!("  lume parse <file>");
    eprintln!("  lume check <file>");
    eprintln!("  lume run <file> [entry]");
}

fn read_source(
    args: &mut impl Iterator<Item = String>,
    command: &str,
) -> Result<SourceFile, ExitCode> {
    let Some(path) = args.next() else {
        eprintln!("missing source file for '{command}'");
        print_usage();
        return Err(ExitCode::from(2));
    };

    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("read {path}: {err}");
            return Err(ExitCode::from(1));
        }
    };

    Ok(SourceFile::new(path, text))
}

fn print_diagnostic(path: &str, diagnostic: &Diagnostic) {
    let severity = match diagnostic.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    eprintln!(
        "{path}:{}:{}: {severity}[{}]: {}",
        diagnostic.span.start_pos.line,
        diagnostic.span.start_pos.column,
        diagnostic.code,
        diagnostic.message,
    );
}
