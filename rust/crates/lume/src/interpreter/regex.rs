use ::regex::Regex;

use super::*;

pub(crate) struct RegexValue {
    regex: Regex,
    full: Regex,
}

impl RegexValue {
    pub(super) fn pattern(&self) -> &str {
        self.regex.as_str()
    }
}

pub(crate) struct RegexMatchValue {
    text: String,
    start: i64,
    end: i64,
    groups: Vec<Option<String>>,
    names: Vec<(String, usize)>,
}

impl RegexMatchValue {
    pub(super) fn text(&self) -> &str {
        &self.text
    }

    fn capture(regex: &Regex, text: &str, captures: ::regex::Captures<'_>) -> Self {
        let matched = captures.get(0).expect("successful regex capture");
        Self {
            text: matched.as_str().to_string(),
            start: text[..matched.start()].chars().count() as i64,
            end: text[..matched.end()].chars().count() as i64,
            groups: captures
                .iter()
                .map(|capture| capture.map(|capture| capture.as_str().to_string()))
                .collect(),
            names: regex
                .capture_names()
                .enumerate()
                .filter_map(|(index, name)| name.map(|name| (name.to_string(), index)))
                .collect(),
        }
    }
}

impl Interpreter<'_> {
    pub(crate) fn native_regex_static_compile(
        &mut self,
        receiver: Value,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let _ = receiver;
        let method = "compile";

        expect_runtime_arity(self, &format!("Regex.{method}"), &args, 1, span)?;
        let Value::String(pattern) = &args[0] else {
            return Err(self.runtime_error(span, "Regex expects a Str argument"));
        };
        
                let compiled = Regex::new(pattern).and_then(|regex| {
                    Regex::new(&format!(r"\A(?:{pattern})\z"))
                        .map(|full| RegexValue { regex, full })
                });
                Ok(match compiled {
                    Ok(regex) => self.result_ok(Value::Regex(Rc::new(regex))),
                    Err(error) => {
                        let error = Value::Aggregate(Rc::new(RefCell::new(AggregateValue {
                            runtime_type_id: self
                                .runtime
                                .type_id_by_name_kind("RegexError", ast::TypeKind::Record),
                            type_name: "RegexError".to_string(),
                            kind: ast::TypeKind::Record,
                            case_id: None,
                            case_name: None,
                            field_names: vec!["message".to_string(), "offset".to_string()],
                            fields: vec![Value::String(error.to_string()), self.option_none()],
                        })));
                        self.result_err(error)
                    }
                })
            
    }

    pub(crate) fn native_regex_static_escape(
        &mut self,
        receiver: Value,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let _ = receiver;
        let method = "escape";

        expect_runtime_arity(self, &format!("Regex.{method}"), &args, 1, span)?;
        let Value::String(pattern) = &args[0] else {
            return Err(self.runtime_error(span, "Regex expects a Str argument"));
        };
        Ok(Value::String(::regex::escape(pattern)))
    }

    pub(crate) fn native_regex_contains(
        &mut self,
        receiver: Value,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let Value::Regex(regex) = receiver else { unreachable!() };
        let method = "contains";

        expect_runtime_arity(self, &format!("Regex.{method}"), &args, 1, span)?;
        let Value::String(text) = &args[0] else {
            return Err(self.runtime_error(span, "Regex input must be Str"));
        };
        Ok(Value::Bool(regex.regex.is_match(text)))
    }

    pub(crate) fn native_regex_matches(
        &mut self,
        receiver: Value,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let Value::Regex(regex) = receiver else { unreachable!() };
        let method = "matches";

        expect_runtime_arity(self, &format!("Regex.{method}"), &args, 1, span)?;
        let Value::String(text) = &args[0] else {
            return Err(self.runtime_error(span, "Regex input must be Str"));
        };
        Ok(Value::Bool(regex.full.is_match(text)))
    }

    pub(crate) fn native_regex_find(
        &mut self,
        receiver: Value,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let Value::Regex(regex) = receiver else { unreachable!() };
        let method = "find";

        expect_runtime_arity(self, &format!("Regex.{method}"), &args, 1, span)?;
        let Value::String(text) = &args[0] else {
            return Err(self.runtime_error(span, "Regex input must be Str"));
        };
        
                let compiled = if method == "fullMatch" {
                    &regex.full
                } else {
                    &regex.regex
                };
                Ok(match compiled.captures(text) {
                    Some(captures) => self.option_some(Value::RegexMatch(Rc::new(
                        RegexMatchValue::capture(compiled, text, captures),
                    ))),
                    None => self.option_none(),
                })
            
    }

    pub(crate) fn native_regex_full_match(
        &mut self,
        receiver: Value,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let Value::Regex(regex) = receiver else { unreachable!() };
        let method = "fullMatch";

        expect_runtime_arity(self, &format!("Regex.{method}"), &args, 1, span)?;
        let Value::String(text) = &args[0] else {
            return Err(self.runtime_error(span, "Regex input must be Str"));
        };
        
                let compiled = if method == "fullMatch" {
                    &regex.full
                } else {
                    &regex.regex
                };
                Ok(match compiled.captures(text) {
                    Some(captures) => self.option_some(Value::RegexMatch(Rc::new(
                        RegexMatchValue::capture(compiled, text, captures),
                    ))),
                    None => self.option_none(),
                })
            
    }

    pub(crate) fn native_regex_find_all(
        &mut self,
        receiver: Value,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let Value::Regex(regex) = receiver else { unreachable!() };
        let method = "findAll";

        expect_runtime_arity(self, &format!("Regex.{method}"), &args, 1, span)?;
        let Value::String(text) = &args[0] else {
            return Err(self.runtime_error(span, "Regex input must be Str"));
        };
        Ok(Value::list(
                regex
                    .regex
                    .captures_iter(text)
                    .map(|captures| {
                        Value::RegexMatch(Rc::new(RegexMatchValue::capture(
                            &regex.regex,
                            text,
                            captures,
                        )))
                    })
                    .collect(),
            ))
    }

    pub(crate) fn native_regex_match_text(
        &mut self,
        receiver: Value,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let Value::RegexMatch(hit) = receiver else { unreachable!() };
        let method = "text";

        expect_runtime_arity(
            self,
            &format!("RegexMatch.{method}"),
            &args,
            usize::from(method == "group"),
            span,
        )?;
        Ok(Value::String(hit.text.clone()))
    }

    pub(crate) fn native_regex_match_start(
        &mut self,
        receiver: Value,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let Value::RegexMatch(hit) = receiver else { unreachable!() };
        let method = "start";

        expect_runtime_arity(
            self,
            &format!("RegexMatch.{method}"),
            &args,
            usize::from(method == "group"),
            span,
        )?;
        Ok(Value::Int(hit.start))
    }

    pub(crate) fn native_regex_match_end(
        &mut self,
        receiver: Value,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let Value::RegexMatch(hit) = receiver else { unreachable!() };
        let method = "end";

        expect_runtime_arity(
            self,
            &format!("RegexMatch.{method}"),
            &args,
            usize::from(method == "group"),
            span,
        )?;
        Ok(Value::Int(hit.end))
    }

    pub(crate) fn native_regex_match_groups(
        &mut self,
        receiver: Value,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let Value::RegexMatch(hit) = receiver else { unreachable!() };
        let method = "groups";

        expect_runtime_arity(
            self,
            &format!("RegexMatch.{method}"),
            &args,
            usize::from(method == "group"),
            span,
        )?;
        Ok(Value::list(
                hit.groups
                    .iter()
                    .map(|capture| {
                        capture
                            .as_ref()
                            .map(|text| self.option_some(Value::String(text.clone())))
                            .unwrap_or_else(|| self.option_none())
                    })
                    .collect(),
            ))
    }

    pub(crate) fn native_regex_match_group_map(
        &mut self,
        receiver: Value,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let Value::RegexMatch(hit) = receiver else { unreachable!() };
        let method = "groupMap";

        expect_runtime_arity(
            self,
            &format!("RegexMatch.{method}"),
            &args,
            usize::from(method == "group"),
            span,
        )?;
        Ok(Value::map(
                hit.names
                    .iter()
                    .filter_map(|(name, index)| {
                        hit.groups[*index]
                            .as_ref()
                            .map(|text| (Value::String(name.clone()), Value::String(text.clone())))
                    })
                    .collect(),
            ))
    }

    pub(crate) fn native_regex_match_group(
        &mut self,
        receiver: Value,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let Value::RegexMatch(hit) = receiver else { unreachable!() };
        let method = "group";

        expect_runtime_arity(
            self,
            &format!("RegexMatch.{method}"),
            &args,
            usize::from(method == "group"),
            span,
        )?;
        
                let index = match &args[0] {
                    Value::Int(index) => usize::try_from(*index).ok(),
                    Value::String(name) => hit
                        .names
                        .iter()
                        .find(|(candidate, _)| candidate == name)
                        .map(|(_, index)| *index),
                    _ => {
                        return Err(self.runtime_error(span, "RegexMatch.group expects Int or Str"));
                    }
                };
                Ok(index
                    .and_then(|index| hit.groups.get(index))
                    .and_then(|capture| capture.as_ref())
                    .map(|text| self.option_some(Value::String(text.clone())))
                    .unwrap_or_else(|| self.option_none()))
    }
}
