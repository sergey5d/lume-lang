use std::{env, fs, process::ExitCode};

use alang::{Diagnostic, Severity, SourceFile, lex};

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_usage();
        return ExitCode::from(2);
    };

    match command.as_str() {
        "tokens" => {
            let Some(path) = args.next() else {
                eprintln!("missing source file for 'tokens'");
                print_usage();
                return ExitCode::from(2);
            };

            let text = match fs::read_to_string(&path) {
                Ok(text) => text,
                Err(err) => {
                    eprintln!("read {path}: {err}");
                    return ExitCode::from(1);
                }
            };

            let file = SourceFile::new(path.clone(), text);
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
        "parse" | "check" | "run" => {
            eprintln!(
                "'{command}' is not implemented yet in Rust; the current Rust implementation starts with lexing"
            );
            ExitCode::from(2)
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
    eprintln!("  alang tokens <file>");
    eprintln!("  alang parse <file>   # planned");
    eprintln!("  alang check <file>   # planned");
    eprintln!("  alang run <file>     # planned");
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
