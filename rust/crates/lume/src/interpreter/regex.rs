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
    pub(super) fn invoke_regex_static(
        &mut self,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        expect_runtime_arity(self, &format!("Regex.{method}"), &args, 1, span)?;
        let Value::String(pattern) = &args[0] else {
            return Err(self.runtime_error(span, "Regex expects a Str argument"));
        };
        match method {
            "compile" => {
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
            "escape" => Ok(Value::String(::regex::escape(pattern))),
            _ => Err(self.runtime_error(span, format!("unknown Regex method '{method}'"))),
        }
    }

    pub(super) fn invoke_regex_method(
        &mut self,
        regex: Rc<RegexValue>,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        expect_runtime_arity(self, &format!("Regex.{method}"), &args, 1, span)?;
        let Value::String(text) = &args[0] else {
            return Err(self.runtime_error(span, "Regex input must be Str"));
        };
        match method {
            "contains" => Ok(Value::Bool(regex.regex.is_match(text))),
            "matches" => Ok(Value::Bool(regex.full.is_match(text))),
            "find" | "fullMatch" => {
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
            "findAll" => Ok(Value::list(
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
            )),
            _ => Err(self.runtime_error(span, format!("unknown Regex method '{method}'"))),
        }
    }

    pub(super) fn invoke_regex_match_method(
        &mut self,
        hit: Rc<RegexMatchValue>,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        expect_runtime_arity(
            self,
            &format!("RegexMatch.{method}"),
            &args,
            usize::from(method == "group"),
            span,
        )?;
        match method {
            "text" => Ok(Value::String(hit.text.clone())),
            "start" => Ok(Value::Int(hit.start)),
            "end" => Ok(Value::Int(hit.end)),
            "groups" => Ok(Value::list(
                hit.groups
                    .iter()
                    .map(|capture| {
                        capture
                            .as_ref()
                            .map(|text| self.option_some(Value::String(text.clone())))
                            .unwrap_or_else(|| self.option_none())
                    })
                    .collect(),
            )),
            "groupMap" => Ok(Value::map(
                hit.names
                    .iter()
                    .filter_map(|(name, index)| {
                        hit.groups[*index]
                            .as_ref()
                            .map(|text| (Value::String(name.clone()), Value::String(text.clone())))
                    })
                    .collect(),
            )),
            "group" => {
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
            _ => Err(self.runtime_error(span, format!("unknown RegexMatch member '{method}'"))),
        }
    }
}
