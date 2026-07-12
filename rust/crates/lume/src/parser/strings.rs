use super::*;

impl<'a> Parser<'a> {
    pub(super) fn parse_string_expr(&mut self, token: Token) -> Option<Expr> {
        let Some(literal) = string_literal(&token.lexeme) else {
            self.diagnostics.push(Diagnostic::error(
                "invalid_string_literal",
                "invalid string literal",
                token.span,
            ));
            return None;
        };

        if literal.is_raw {
            return Some(Expr::String {
                raw: token.lexeme,
                span: token.span,
            });
        }

        if !string_has_interpolation(literal.body) {
            return Some(Expr::String {
                raw: token.lexeme,
                span: token.span,
            });
        }

        let parts = match parse_interpolated_string_parts(literal.body) {
            Ok(parts) => parts,
            Err(message) => {
                self.diagnostics.push(Diagnostic::error(
                    "invalid_string_interpolation",
                    format!("invalid string interpolation: {message}"),
                    token.span,
                ));
                return None;
            }
        };

        let mut exprs = Vec::with_capacity(parts.len() * 2 + 1);
        if parts.first().is_none_or(|part| !part.is_literal) {
            exprs.push(Expr::String {
                raw: encode_string_literal(""),
                span: token.span,
            });
        }

        for part in parts {
            if part.is_literal {
                let decoded = match decode_string_contents(&part.text) {
                    Ok(decoded) => decoded,
                    Err(message) => {
                        self.diagnostics.push(Diagnostic::error(
                            "invalid_string_literal",
                            format!("invalid string literal: {message}"),
                            token.span,
                        ));
                        return None;
                    }
                };
                exprs.push(Expr::String {
                    raw: encode_string_literal(&decoded),
                    span: token.span,
                });
                continue;
            }

            let expr = match parse_embedded_expr(&part.text) {
                Ok(expr) => expr,
                Err(message) => {
                    self.diagnostics.push(Diagnostic::error(
                        "invalid_string_interpolation",
                        format!("invalid string interpolation: {message}"),
                        token.span,
                    ));
                    return None;
                }
            };
            exprs.push(expr);
        }

        let mut iter = exprs.into_iter();
        let mut left = iter.next()?;
        for right in iter {
            left = Expr::Binary {
                left: Box::new(left),
                op: BinaryOp::Add,
                right: Box::new(right),
                span: token.span,
            };
        }
        Some(left)
    }
}

#[derive(Debug, Clone, Copy)]
struct StringLiteral<'a> {
    is_raw: bool,
    body: &'a str,
}

#[derive(Debug, Clone)]
struct InterpolatedStringPart {
    is_literal: bool,
    text: String,
}

fn string_literal(raw: &str) -> Option<StringLiteral<'_>> {
    let (is_raw, quoted) = raw
        .strip_prefix("raw")
        .map_or((false, raw), |quoted| (true, quoted));
    let body = if quoted.starts_with("\"\"\"") && quoted.ends_with("\"\"\"") && quoted.len() >= 6 {
        &quoted[3..quoted.len() - 3]
    } else {
        quoted.strip_prefix('"')?.strip_suffix('"')?
    };
    Some(StringLiteral { is_raw, body })
}

fn string_has_interpolation(body: &str) -> bool {
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            continue;
        }
        if chars.peek().is_some() {
            return true;
        }
    }
    false
}

fn decode_string_contents(raw: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            return Err("dangling escape".to_string());
        };
        match next {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '$' => out.push('$'),
            other => return Err(format!("unsupported escape \\{other}")),
        }
    }
    Ok(out)
}

fn encode_string_literal(value: &str) -> String {
    let mut raw = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\n' => raw.push_str("\\n"),
            '\t' => raw.push_str("\\t"),
            '\r' => raw.push_str("\\r"),
            '\\' => raw.push_str("\\\\"),
            '"' => raw.push_str("\\\""),
            other => raw.push(other),
        }
    }
    raw.push('"');
    raw
}

fn parse_interpolated_string_parts(body: &str) -> Result<Vec<InterpolatedStringPart>, String> {
    let runes: Vec<char> = body.chars().collect();
    let mut parts = Vec::new();
    let mut literal = String::new();
    let flush_literal = |parts: &mut Vec<InterpolatedStringPart>, literal: &mut String| {
        if literal.is_empty() {
            return;
        }
        parts.push(InterpolatedStringPart {
            is_literal: true,
            text: std::mem::take(literal),
        });
    };

    let mut i = 0usize;
    while i < runes.len() {
        match runes[i] {
            '\\' if i + 1 < runes.len() && runes[i + 1] == '$' => {
                literal.push('$');
                i += 2;
            }
            '\\' => {
                literal.push('\\');
                i += 1;
            }
            '$' => {
                if i + 1 >= runes.len() {
                    return Err("dangling '$'".to_string());
                }
                flush_literal(&mut parts, &mut literal);
                if runes[i + 1] == '{' {
                    let start = i + 2;
                    let end = find_interpolated_expr_end(&runes, start)?;
                    let expr: String = runes[start..end].iter().collect();
                    if expr.is_empty() {
                        return Err("empty interpolation".to_string());
                    }
                    parts.push(InterpolatedStringPart {
                        is_literal: false,
                        text: expr,
                    });
                    i = end + 1;
                    continue;
                }
                if !is_ascii_identifier_start(runes[i + 1]) {
                    return Err("expected identifier or '{' after '$'".to_string());
                }
                let start = i + 1;
                let mut end = start + 1;
                while end < runes.len() && is_ascii_identifier_continue(runes[end]) {
                    end += 1;
                }
                parts.push(InterpolatedStringPart {
                    is_literal: false,
                    text: runes[start..end].iter().collect(),
                });
                i = end;
            }
            ch => {
                literal.push(ch);
                i += 1;
            }
        }
    }

    flush_literal(&mut parts, &mut literal);
    Ok(parts)
}

fn is_ascii_identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ascii_identifier_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn find_interpolated_expr_end(runes: &[char], start: usize) -> Result<usize, String> {
    let mut depth = 1usize;
    let mut i = start;
    while i < runes.len() {
        match runes[i] {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(i);
                }
            }
            '"' => {
                i += 1;
                while i < runes.len() {
                    if runes[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    if runes[i] == '"' {
                        break;
                    }
                    i += 1;
                }
                if i >= runes.len() {
                    return Err("unterminated string inside interpolation".to_string());
                }
            }
            '\'' => {
                i += 1;
                while i < runes.len() {
                    if runes[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    if runes[i] == '\'' {
                        break;
                    }
                    i += 1;
                }
                if i >= runes.len() {
                    return Err("unterminated rune inside interpolation".to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }
    Err("unterminated '${...}'".to_string())
}

fn parse_embedded_expr(source: &str) -> Result<Expr, String> {
    let file = crate::source::SourceFile::new("<interpolation>", source);
    let lexed = crate::lexer::lex(&file);
    if let Some(diag) = lexed.diagnostics.first() {
        return Err(diag.message.clone());
    }

    let mut parser = Parser::new(&lexed.tokens);
    let Some(expr) = parser.parse_expr() else {
        return Err("empty interpolation".to_string());
    };
    parser.skip_newlines();
    if !parser.at(TokenKind::Eof) {
        return Err("unexpected trailing tokens in interpolation".to_string());
    }
    if let Some(diag) = parser.diagnostics.first() {
        return Err(diag.message.clone());
    }
    Ok(expr)
}
