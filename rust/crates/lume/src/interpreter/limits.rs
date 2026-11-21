use std::collections::BTreeMap;

use super::*;

/// Cooperative limits. A browser worker and a capped Wasm memory provide the
/// hard-stop boundary for allocations or native operations that cannot yield.
#[derive(Debug, Clone)]
pub struct ExecutionLimits {
    pub steps: u64,
    pub call_depth: usize,
    pub collection_items: usize,
    pub string_bytes: usize,
    pub output_bytes: usize,
    pub cleanup_steps: u64,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            steps: 1_000_000,
            call_depth: 128,
            collection_items: 100_000,
            string_bytes: 1_048_576,
            output_bytes: 65_536,
            cleanup_steps: 10_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub args: Vec<String>,
    /// `None` uses the native process environment; browsers always supply a map.
    pub environment: Option<BTreeMap<String, String>>,
    pub limits: Option<ExecutionLimits>,
    pub allow_files: bool,
    pub output: Option<fn(&str)>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            args: Vec::new(),
            environment: None,
            limits: None,
            allow_files: true,
            output: None,
        }
    }
}

impl RunOptions {
    pub fn browser() -> Self {
        Self {
            environment: Some(BTreeMap::new()),
            limits: Some(ExecutionLimits::default()),
            allow_files: false,
            ..Self::default()
        }
    }
}

impl Interpreter<'_> {
    pub(super) fn limit_error(&self, span: Option<Span>, message: impl Into<String>) -> Diagnostic {
        self.limit_exhausted.set(true);
        let mut diagnostic = self.runtime_error(span, message);
        diagnostic.code = "execution_limit";
        diagnostic
    }

    pub(crate) fn charge_work(&self, amount: u64, span: Option<Span>) -> Result<(), Diagnostic> {
        if self.options.limits.is_none() {
            return Ok(());
        }
        let remaining = if self.cleanup_mode {
            &self.cleanup_remaining
        } else {
            &self.steps_remaining
        };
        if let Some(next) = remaining.get().checked_sub(amount) {
            remaining.set(next);
            Ok(())
        } else {
            Err(self.limit_error(span, "execution step limit exceeded"))
        }
    }

    pub(crate) fn check_collection_size(
        &self,
        length: usize,
        span: Option<Span>,
    ) -> Result<(), Diagnostic> {
        if self
            .options
            .limits
            .as_ref()
            .is_some_and(|limits| length > limits.collection_items)
        {
            return Err(self.limit_error(span, "collection size limit exceeded"));
        }
        Ok(())
    }

    pub(crate) fn check_string_size(
        &self,
        length: usize,
        span: Option<Span>,
    ) -> Result<(), Diagnostic> {
        if self
            .options
            .limits
            .as_ref()
            .is_some_and(|limits| length > limits.string_bytes)
        {
            return Err(self.limit_error(span, "string size limit exceeded"));
        }
        Ok(())
    }

    pub(super) fn check_value_size(
        &self,
        value: &Value,
        span: Option<Span>,
    ) -> Result<(), Diagnostic> {
        match value {
            Value::String(text) => self.check_string_size(text.len(), span),
            Value::List(items) | Value::Set(items) => {
                self.check_collection_size(items.borrow().len(), span)
            }
            Value::Map(items) => self.check_collection_size(items.borrow().len(), span),
            _ => Ok(()),
        }
    }

    pub(super) fn append_output(
        &mut self,
        text: &str,
        span: Option<Span>,
    ) -> Result<(), Diagnostic> {
        if self.options.limits.as_ref().is_some_and(|limits| {
            self.output.len().saturating_add(text.len()) > limits.output_bytes
        }) {
            return Err(self.limit_error(span, "output limit exceeded"));
        }
        self.output.push_str(text);
        if let Some(output) = self.options.output {
            output(text);
        }
        Ok(())
    }

    pub(super) fn require_files(&self, span: Option<Span>) -> Result<(), Diagnostic> {
        if !self.options.allow_files {
            let mut diagnostic =
                self.runtime_error(span, "native file access is unavailable in the browser");
            diagnostic.code = "unsupported_target";
            return Err(diagnostic);
        }
        Ok(())
    }
}
