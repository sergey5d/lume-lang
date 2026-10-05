use crate::{Diagnostic, SourceFile, Token, TokenKind, lex, parse_program};

const INDENT: &str = "    ";

#[derive(Debug, Clone)]
pub struct FormatResult {
    pub text: String,
    pub diagnostics: Vec<Diagnostic>,
}

impl FormatResult {
    pub fn has_errors(&self) -> bool {
        !self.diagnostics.is_empty()
    }
}

/// Formats one syntactically valid Lume source file without rewriting tokens.
///
/// This deliberately preserves comments, token spelling, and multiline string
/// contents. Invalid input is returned unchanged so callers can safely avoid
/// overwriting the source file.
pub fn format_source(file: &SourceFile) -> FormatResult {
    let lexed = lex(file);
    if lexed.has_errors() {
        return FormatResult {
            text: file.text.clone(),
            diagnostics: lexed.diagnostics,
        };
    }

    let parsed = parse_program(&lexed.tokens);
    if !parsed.diagnostics.is_empty() {
        return FormatResult {
            text: file.text.clone(),
            diagnostics: parsed.diagnostics,
        };
    }

    FormatResult {
        text: format_valid_source(&file.text, &lexed.tokens),
        diagnostics: Vec::new(),
    }
}

#[derive(Clone, Copy)]
struct SourceLine<'a> {
    start: usize,
    content: &'a str,
    ending: &'a str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DelimiterKind {
    Parenthesis,
    Bracket,
}

#[derive(Clone, Copy)]
struct OpenDelimiter {
    kind: DelimiterKind,
    opened_line: usize,
    suspended_by_brace_depth: Option<usize>,
}

fn format_valid_source(source: &str, tokens: &[Token]) -> String {
    let lines = source_lines(source);
    if lines.is_empty() {
        return String::new();
    }

    let mut tokens_by_line = vec![Vec::new(); lines.len()];
    let mut protected_lines = vec![false; lines.len()];
    for token in tokens {
        if matches!(token.kind, TokenKind::Newline | TokenKind::Eof) {
            continue;
        }

        let line_index = token.span.start_pos.line.saturating_sub(1);
        if let Some(line_tokens) = tokens_by_line.get_mut(line_index) {
            line_tokens.push(token);
        }

        if token.kind == TokenKind::String && token.span.end_pos.line > token.span.start_pos.line {
            let start = token.span.start_pos.line.saturating_sub(1);
            let end = token.span.end_pos.line.min(lines.len());
            for protected in &mut protected_lines[start..end] {
                *protected = true;
            }
        }
    }

    let final_newline = lines
        .iter()
        .find_map(|line| (!line.ending.is_empty()).then_some(line.ending))
        .unwrap_or("\n");
    let mut output = String::with_capacity(source.len() + INDENT.len());
    let mut brace_depth = 0usize;
    let mut delimiters = Vec::new();
    let mut continuation_pending = false;

    for (index, line) in lines.iter().enumerate() {
        let line_tokens = &tokens_by_line[index];

        if protected_lines[index] {
            output.push_str(line.content);
            output.push_str(line.ending);
            update_nesting(line_tokens, index, &mut brace_depth, &mut delimiters);
            continuation_pending = line_requests_continuation(line_tokens);
            continue;
        }

        let without_indent = line.content.trim_start_matches([' ', '\t']);
        let body = without_indent.trim_end_matches([' ', '\t']);
        if body.is_empty() {
            output.push_str(line.ending);
            continue;
        }
        let body_start = line.start + line.content.len() - without_indent.len();
        let body_end = body_start + body.len();
        let body = normalize_horizontal_spacing(source, body_start, body_end, line_tokens);

        let (line_brace_depth, line_delimiters) =
            nesting_after_leading_closers(line_tokens, brace_depth, &delimiters);
        let has_active_delimiter = line_delimiters.iter().any(|delimiter| {
            delimiter
                .suspended_by_brace_depth
                .is_none_or(|depth| brace_depth < depth)
        });
        let starts_continuation = line_tokens
            .first()
            .is_some_and(|token| starts_with_continuation(token.kind));
        let starts_with_closer = line_tokens.first().is_some_and(|token| {
            matches!(
                token.kind,
                TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket
            )
        });
        let continuation = has_active_delimiter
            || (!starts_with_closer && (continuation_pending || starts_continuation));

        for _ in 0..line_brace_depth + usize::from(continuation) {
            output.push_str(INDENT);
        }
        output.push_str(&body);
        output.push_str(line.ending);

        update_nesting(line_tokens, index, &mut brace_depth, &mut delimiters);
        continuation_pending = line_requests_continuation(line_tokens);
    }

    if !output.is_empty() && !output.ends_with(['\n', '\r']) {
        output.push_str(final_newline);
    }
    output
}

fn source_lines(source: &str) -> Vec<SourceLine<'_>> {
    let mut lines = Vec::new();
    let mut start = 0usize;
    for (newline, _) in source.match_indices('\n') {
        let (content_end, ending_start) =
            if newline > start && source.as_bytes()[newline - 1] == b'\r' {
                (newline - 1, newline - 1)
            } else {
                (newline, newline)
            };
        lines.push(SourceLine {
            start,
            content: &source[start..content_end],
            ending: &source[ending_start..=newline],
        });
        start = newline + 1;
    }
    if start < source.len() {
        lines.push(SourceLine {
            start,
            content: &source[start..],
            ending: "",
        });
    }
    lines
}

fn normalize_horizontal_spacing(
    source: &str,
    start: usize,
    end: usize,
    tokens: &[&Token],
) -> String {
    let mut output = String::with_capacity(end.saturating_sub(start));
    let mut cursor = start;

    for token in tokens {
        if token.span.start < start || token.span.end > end {
            continue;
        }

        let gap = &source[cursor..token.span.start];
        if gap.chars().all(|ch| matches!(ch, ' ' | '\t')) {
            if !gap.is_empty() && token.kind != TokenKind::Colon {
                output.push(' ');
            }
        } else {
            output.push_str(gap);
        }
        output.push_str(&source[token.span.start..token.span.end]);
        cursor = token.span.end;
    }

    output.push_str(&source[cursor..end]);
    output
}

fn nesting_after_leading_closers(
    tokens: &[&Token],
    mut braces: usize,
    delimiters: &[OpenDelimiter],
) -> (usize, Vec<OpenDelimiter>) {
    let mut delimiters = delimiters.to_vec();
    for token in tokens {
        match token.kind {
            TokenKind::RBrace => braces = braces.saturating_sub(1),
            TokenKind::RParen => pop_delimiter(&mut delimiters, DelimiterKind::Parenthesis),
            TokenKind::RBracket => pop_delimiter(&mut delimiters, DelimiterKind::Bracket),
            _ => break,
        }
    }
    (braces, delimiters)
}

fn update_nesting(
    tokens: &[&Token],
    line: usize,
    braces: &mut usize,
    delimiters: &mut Vec<OpenDelimiter>,
) {
    for token in tokens {
        match token.kind {
            TokenKind::LBrace => {
                *braces += 1;
                for delimiter in delimiters.iter_mut().rev() {
                    if delimiter.opened_line != line {
                        break;
                    }
                    delimiter.suspended_by_brace_depth.get_or_insert(*braces);
                }
            }
            TokenKind::RBrace => *braces = braces.saturating_sub(1),
            TokenKind::LParen => delimiters.push(OpenDelimiter {
                kind: DelimiterKind::Parenthesis,
                opened_line: line,
                suspended_by_brace_depth: None,
            }),
            TokenKind::RParen => pop_delimiter(delimiters, DelimiterKind::Parenthesis),
            TokenKind::LBracket => delimiters.push(OpenDelimiter {
                kind: DelimiterKind::Bracket,
                opened_line: line,
                suspended_by_brace_depth: None,
            }),
            TokenKind::RBracket => pop_delimiter(delimiters, DelimiterKind::Bracket),
            _ => {}
        }
    }
}

fn pop_delimiter(delimiters: &mut Vec<OpenDelimiter>, kind: DelimiterKind) {
    if delimiters
        .last()
        .is_some_and(|delimiter| delimiter.kind == kind)
    {
        delimiters.pop();
    }
}

fn line_requests_continuation(tokens: &[&Token]) -> bool {
    let Some(last) = tokens.last() else {
        return false;
    };
    ends_with_continuation(last.kind) && !ends_with_braced_lambda_head(tokens)
}

fn ends_with_braced_lambda_head(tokens: &[&Token]) -> bool {
    if !tokens
        .last()
        .is_some_and(|token| token.kind == TokenKind::FatArrow)
    {
        return false;
    }
    let Some(brace) = tokens
        .iter()
        .rposition(|token| token.kind == TokenKind::LBrace)
    else {
        return false;
    };
    !tokens[brace + 1..]
        .iter()
        .any(|token| token.kind == TokenKind::Keyword(crate::Keyword::Case))
}

fn starts_with_continuation(kind: TokenKind) -> bool {
    matches!(kind, TokenKind::Dot | TokenKind::Pipe)
}

fn ends_with_continuation(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Dot
            | TokenKind::Colon
            | TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Percent
            | TokenKind::Eq
            | TokenKind::FatArrow
            | TokenKind::LeftArrow
            | TokenKind::QuestionQuestion
            | TokenKind::ColonAssign
            | TokenKind::EqEq
            | TokenKind::NotEq
            | TokenKind::StrictEq
            | TokenKind::StrictNotEq
            | TokenKind::Less
            | TokenKind::LessEq
            | TokenKind::Greater
            | TokenKind::GreaterEq
            | TokenKind::PlusEq
            | TokenKind::MinusEq
            | TokenKind::StarEq
            | TokenKind::SlashEq
            | TokenKind::PercentEq
            | TokenKind::AndAnd
            | TokenKind::OrOr
            | TokenKind::Pipe
            | TokenKind::Keyword(crate::Keyword::As)
            | TokenKind::Keyword(crate::Keyword::Else)
            | TokenKind::Keyword(crate::Keyword::Is)
            | TokenKind::Keyword(crate::Keyword::When)
            | TokenKind::Keyword(crate::Keyword::With)
            | TokenKind::Keyword(crate::Keyword::Yield)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(text: &str) -> FormatResult {
        format_source(&SourceFile::new("test.lum", text))
    }

    #[test]
    fn formats_blocks_continuations_and_trailing_whitespace() {
        let result = format(
            "def main() Unit {\nprintln(\"start\")  \nvalue = 1 +\n2\nif true {\nprintln(value)\n}\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "def main() Unit {\n    println(\"start\")\n    value = 1 +\n        2\n    if true {\n        println(value)\n    }\n}\n"
        );
    }

    #[test]
    fn indentation_does_not_extend_an_unbraced_lambda() {
        let result = format(
            "def main() Unit {\nmapper fn(Int) Int = value =>\n        value + 1\n        next = 2\nprintln(mapper(next))\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "def main() Unit {\n    mapper fn(Int) Int = value =>\n        value + 1\n    next = 2\n    println(mapper(next))\n}\n"
        );
    }

    #[test]
    fn preserves_multiline_string_contents() {
        let result = format(
            "def main() Unit {\n    text = \"\"\"first\n  second   \nthird\"\"\"\nprintln(text)\n}",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "def main() Unit {\n    text = \"\"\"first\n  second   \nthird\"\"\"\n    println(text)\n}\n"
        );
    }

    #[test]
    fn aligns_comma_separated_entries_and_closing_delimiters() {
        let result = format(
            "def main() Unit {\nvalues = Vector(\n1,\n2,\n)\npoint = {\nx: 1,\ny: 2\n}\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "def main() Unit {\n    values = Vector(\n        1,\n        2,\n    )\n    point = {\n        x: 1,\n        y: 2\n    }\n}\n"
        );
    }

    #[test]
    fn formats_nested_trailing_lambdas_inside_calls() {
        let result = format(
            "class Aggregator {\nprivate def get_rollup(key Str, range IntRange?) Rollup? = {\ndef notInRange(record Record) = {\nlet r <- range else return false\nrecord.timestamp < r.start || record.timestamp >= r.end\n}\nmaybeRollup = rollupMap[key].map { records =>\nrecords.fold(Rollup(0, 0), (acc, record) => {\nif record.void || notInRange(record) {\nreturn acc\n}\nRollup {\ntotal: record.amount + acc.total\ncount: acc.count + 1\n}\n})\n}\nif let rollup <- maybeRollup && rollup.count == 0 {\nreturn None\n}\nreturn maybeRollup\n}\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "class Aggregator {\n    private def get_rollup(key Str, range IntRange?) Rollup? = {\n        def notInRange(record Record) = {\n            let r <- range else return false\n            record.timestamp < r.start || record.timestamp >= r.end\n        }\n        maybeRollup = rollupMap[key].map { records =>\n            records.fold(Rollup(0, 0), (acc, record) => {\n                if record.void || notInRange(record) {\n                    return acc\n                }\n                Rollup {\n                    total: record.amount + acc.total\n                    count: acc.count + 1\n                }\n            })\n        }\n        if let rollup <- maybeRollup && rollup.count == 0 {\n            return None\n        }\n        return maybeRollup\n    }\n}\n"
        );
    }

    #[test]
    fn formats_braced_lambda_arguments_without_extra_call_indentation() {
        let result = format(
            "class Aggregator {\ndef all_rollups(range IntRange? = None) [Str] {\nrollups = this.rollups(range)\nrollups.sort((x, y) => {\nxRollup = x[1]\nyRollup = y[1]\nif xRollup.total == yRollup.total {\nreturn x[0].compare(y[0])\n}\nreturn yRollup.total - xRollup.total\n})\nrollups.map(x => \"${x[0]}(${x[1].count},${x[1].total})\")\n}\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "class Aggregator {\n    def all_rollups(range IntRange? = None) [Str] {\n        rollups = this.rollups(range)\n        rollups.sort((x, y) => {\n            xRollup = x[1]\n            yRollup = y[1]\n            if xRollup.total == yRollup.total {\n                return x[0].compare(y[0])\n            }\n            return yRollup.total - xRollup.total\n        })\n        rollups.map(x => \"${x[0]}(${x[1].count},${x[1].total})\")\n    }\n}\n"
        );
    }

    #[test]
    fn normalizes_horizontal_spacing_without_changing_literals_or_comments() {
        let result = format(
            "class Aggregator {\nprivate rollupMap [Str :  [Record]] = []\nprivate  recordMap [Str: Record] = []\ndef coordinates  (Int, Int) = (1, 2)\ndef label() Str {\nmessage  =  \"keep  spaces\" # keep  comment spacing\nmessage\n}\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "class Aggregator {\n    private rollupMap [Str: [Record]] = []\n    private recordMap [Str: Record] = []\n    def coordinates (Int, Int) = (1, 2)\n    def label() Str {\n        message = \"keep  spaces\" # keep  comment spacing\n        message\n    }\n}\n"
        );
    }

    #[test]
    fn invalid_source_is_not_rewritten() {
        let source = "def main( Unit {\n";
        let result = format(source);
        assert!(result.has_errors());
        assert_eq!(result.text, source);
    }

    #[test]
    fn formatting_is_idempotent() {
        let first = format("def main() Unit {\nprintln(\"ok\")\n}");
        assert!(!first.has_errors());
        let second = format(&first.text);
        assert!(!second.has_errors());
        assert_eq!(second.text, first.text);
    }
}
