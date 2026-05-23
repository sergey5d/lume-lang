use std::{
    cell::RefCell,
    fmt,
    fs,
    path::Path,
    rc::Rc,
};

use crate::{
    diagnostic::Diagnostic,
    ir,
    lex,
    lower::lower_program,
    parse_program,
    resolver::LocatedDiagnostic,
    source::{LineColumn, SourceFile, Span},
    typecheck::check_path,
};

#[derive(Debug, Clone, Default)]
pub struct RunResult {
    pub diagnostics: Vec<Diagnostic>,
    pub output: String,
    pub return_value: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PathRunResult {
    pub diagnostics: Vec<LocatedDiagnostic>,
    pub output: String,
    pub return_value: Option<String>,
}

pub fn run_program(program: &ir::Program) -> RunResult {
    run_program_entry(program, None)
}

pub fn run_program_entry(program: &ir::Program, requested_entry: Option<&str>) -> RunResult {
    let mut interpreter = Interpreter::new(program);
    match interpreter.run(requested_entry) {
        Ok(Some(value)) => RunResult {
            diagnostics: Vec::new(),
            output: interpreter.output,
            return_value: Some(value.render()),
        },
        Ok(None) => RunResult {
            diagnostics: Vec::new(),
            output: interpreter.output,
            return_value: None,
        },
        Err(diagnostic) => RunResult {
            diagnostics: vec![diagnostic],
            output: interpreter.output,
            return_value: None,
        },
    }
}

pub fn run_path(path: impl AsRef<Path>, requested_entry: Option<&str>) -> Result<PathRunResult, String> {
    let path = path.as_ref();
    let checked = check_path(path)?;
    if !checked.diagnostics.is_empty() {
        return Ok(PathRunResult {
            diagnostics: checked.diagnostics,
            output: String::new(),
            return_value: None,
        });
    }

    let text = fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let file = SourceFile::new(path.display().to_string(), text);
    let lexed = lex(&file);
    if !lexed.diagnostics.is_empty() {
        return Ok(PathRunResult {
            diagnostics: lexed
                .diagnostics
                .into_iter()
                .map(|diagnostic| LocatedDiagnostic {
                    path: file.name.clone(),
                    diagnostic,
                })
                .collect(),
            output: String::new(),
            return_value: None,
        });
    }

    let parsed = parse_program(&lexed.tokens);
    if !parsed.diagnostics.is_empty() {
        return Ok(PathRunResult {
            diagnostics: parsed
                .diagnostics
                .into_iter()
                .map(|diagnostic| LocatedDiagnostic {
                    path: file.name.clone(),
                    diagnostic,
                })
                .collect(),
            output: String::new(),
            return_value: None,
        });
    }

    let program = parsed.program.expect("program after successful parse");
    if !program.imports.is_empty() {
        return Ok(PathRunResult {
            diagnostics: vec![LocatedDiagnostic {
                path: file.name.clone(),
                diagnostic: Diagnostic::error(
                    "runtime_unsupported",
                    "the Rust IR interpreter currently executes one module at a time and does not run imported modules yet",
                    program.imports[0].span,
                ),
            }],
            output: String::new(),
            return_value: None,
        });
    }

    let lowered = lower_program(&program);
    if !lowered.diagnostics.is_empty() {
        return Ok(PathRunResult {
            diagnostics: lowered
                .diagnostics
                .into_iter()
                .map(|diagnostic| LocatedDiagnostic {
                    path: file.name.clone(),
                    diagnostic,
                })
                .collect(),
            output: String::new(),
            return_value: None,
        });
    }

    let lowered_program = lowered.program.expect("ir program after successful lowering");
    let run = run_program_entry(&lowered_program, requested_entry);
    Ok(PathRunResult {
        diagnostics: run
            .diagnostics
            .into_iter()
            .map(|diagnostic| LocatedDiagnostic {
                path: file.name.clone(),
                diagnostic,
            })
            .collect(),
        output: run.output,
        return_value: run.return_value,
    })
}

#[derive(Clone)]
enum Value {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Tuple(Vec<Value>),
    List(Rc<RefCell<Vec<Value>>>),
    Record(Rc<RefCell<Vec<(String, Value)>>>),
    Object(Rc<RefCell<ObjectValue>>),
    Variant(Rc<VariantValue>),
    Iterator(Rc<RefCell<IteratorState>>),
}

impl Value {
    fn default_for_type(ty: &ir::Type) -> Self {
        match ty {
            ir::Type::Unit => Self::Unit,
            ir::Type::Bool => Self::Bool(false),
            ir::Type::Int => Self::Int(0),
            ir::Type::Float => Self::Float(0.0),
            ir::Type::Str => Self::String(String::new()),
            ir::Type::Tuple(items) => {
                Self::Tuple(items.iter().map(Value::default_for_type).collect())
            }
            ir::Type::Record(fields) => Self::Record(Rc::new(RefCell::new(
                fields
                    .iter()
                    .map(|field| (field.name.clone(), Value::default_for_type(&field.ty)))
                    .collect(),
            ))),
            ir::Type::Named { name, args } if name == "List" || name == "Array" => {
                let _ = args;
                Self::List(Rc::new(RefCell::new(Vec::new())))
            }
            ir::Type::Named { name, args } if name == "Option" => {
                let _ = args;
                Value::option_none()
            }
            ir::Type::Named { name, args } if name == "Result" => {
                let _ = args;
                Value::result_err(Value::Unit)
            }
            ir::Type::Named { name, args } if name == "Either" => {
                let _ = args;
                Value::either_left(Value::Unit)
            }
            _ => Self::Unit,
        }
    }

    fn option_none() -> Self {
        Self::Variant(Rc::new(VariantValue {
            enum_name: "Option".to_string(),
            case_name: "None".to_string(),
            fields: Vec::new(),
        }))
    }

    fn option_some(value: Value) -> Self {
        Self::Variant(Rc::new(VariantValue {
            enum_name: "Option".to_string(),
            case_name: "Some".to_string(),
            fields: vec![("value".to_string(), value)],
        }))
    }

    fn result_ok(value: Value) -> Self {
        Self::Variant(Rc::new(VariantValue {
            enum_name: "Result".to_string(),
            case_name: "Ok".to_string(),
            fields: vec![("value".to_string(), value)],
        }))
    }

    fn result_err(error: Value) -> Self {
        Self::Variant(Rc::new(VariantValue {
            enum_name: "Result".to_string(),
            case_name: "Err".to_string(),
            fields: vec![("error".to_string(), error)],
        }))
    }

    fn either_left(value: Value) -> Self {
        Self::Variant(Rc::new(VariantValue {
            enum_name: "Either".to_string(),
            case_name: "Left".to_string(),
            fields: vec![("value".to_string(), value)],
        }))
    }

    fn either_right(value: Value) -> Self {
        Self::Variant(Rc::new(VariantValue {
            enum_name: "Either".to_string(),
            case_name: "Right".to_string(),
            fields: vec![("value".to_string(), value)],
        }))
    }

    fn render(&self) -> String {
        match self {
            Value::Unit => "()".to_string(),
            Value::Bool(value) => value.to_string(),
            Value::Int(value) => value.to_string(),
            Value::Float(value) => {
                let mut rendered = value.to_string();
                if !rendered.contains('.') && !rendered.contains('e') && !rendered.contains('E') {
                    rendered.push_str(".0");
                }
                rendered
            }
            Value::String(value) => value.clone(),
            Value::Tuple(items) => format!(
                "({})",
                items.iter().map(Value::render).collect::<Vec<_>>().join(",")
            ),
            Value::List(items) => format!(
                "[{}]",
                items.borrow().iter().map(Value::render).collect::<Vec<_>>().join(",")
            ),
            Value::Record(fields) => {
                let fields = fields.borrow();
                format!(
                    "record{{{}}}",
                    fields
                        .iter()
                        .map(|(name, value)| format!("{name}={}", value.render()))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
            Value::Object(object) => {
                let object = object.borrow();
                format!(
                    "{}{{{}}}",
                    object.type_name,
                    object
                        .fields
                        .iter()
                        .map(|(name, value)| format!("{name}={}", value.render()))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
            Value::Variant(variant) => {
                if variant.fields.is_empty() {
                    variant.case_name.clone()
                } else {
                    format!(
                        "{}({})",
                        variant.case_name,
                        variant
                            .fields
                            .iter()
                            .map(|(_, value)| value.render())
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                }
            }
            Value::Iterator(_) => "<iterator>".to_string(),
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

#[derive(Debug, Clone)]
struct ObjectValue {
    type_name: String,
    fields: Vec<(String, Value)>,
}

#[derive(Debug, Clone)]
struct VariantValue {
    enum_name: String,
    case_name: String,
    fields: Vec<(String, Value)>,
}

#[derive(Debug, Clone)]
enum IteratorState {
    List {
        items: Rc<RefCell<Vec<Value>>>,
        index: usize,
    },
    Range {
        current: i64,
        end: i64,
        step: i64,
    },
}

#[derive(Debug, Clone)]
struct Frame {
    function: ir::FunctionId,
    locals: Vec<Value>,
}

struct Interpreter<'a> {
    program: &'a ir::Program,
    globals: Vec<Value>,
    globals_ready: bool,
    object_singletons: Vec<Option<Value>>,
    output: String,
}

impl<'a> Interpreter<'a> {
    fn new(program: &'a ir::Program) -> Self {
        Self {
            program,
            globals: program
                .globals
                .iter()
                .map(|global| Value::default_for_type(&global.ty))
                .collect(),
            globals_ready: false,
            object_singletons: vec![None; program.types.len()],
            output: String::new(),
        }
    }

    fn run(&mut self, requested_entry: Option<&str>) -> Result<Option<Value>, Diagnostic> {
        self.ensure_globals()?;
        let entry = self.select_entry(requested_entry)?;
        let value = self.call_function(entry, None, Vec::new(), None)?;
        Ok((!matches!(value, Value::Unit)).then_some(value))
    }

    fn select_entry(&self, requested_entry: Option<&str>) -> Result<ir::FunctionId, Diagnostic> {
        if let Some(name) = requested_entry {
            return self
                .program
                .functions
                .iter()
                .find(|function| function.name == name)
                .map(|function| function.id)
                .ok_or_else(|| self.runtime_error(None, format!("unknown entry '{name}'")));
        }
        if let Some(entry) = self.program.entry {
            return Ok(entry);
        }
        self.program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .or_else(|| self.program.functions.iter().find(|function| function.name == "run"))
            .map(|function| function.id)
            .ok_or_else(|| {
                self.runtime_error(
                    None,
                    "no entry function found; expected lowered 'main' or a top-level 'run'",
                )
            })
    }

    fn ensure_globals(&mut self) -> Result<(), Diagnostic> {
        if self.globals_ready {
            return Ok(());
        }
        for global in &self.program.globals {
            if let Some(initializer) = &global.initializer {
                let value = self.eval_rvalue(initializer, None, None)?;
                self.globals[global.id.0] = value;
            }
        }
        self.globals_ready = true;
        Ok(())
    }

    fn call_function(
        &mut self,
        id: ir::FunctionId,
        receiver: Option<Value>,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let function = self
            .program
            .function(id)
            .cloned()
            .ok_or_else(|| self.runtime_error(span, format!("unknown function id {}", id.0)))?;
        let mut frame = Frame {
            function: function.id,
            locals: function
                .locals
                .iter()
                .map(|local| Value::default_for_type(&local.ty))
                .collect(),
        };

        if matches!(function.kind, ir::FunctionKind::Method { .. }) {
            let Some(receiver) = receiver else {
                return Err(self.runtime_error(
                    span,
                    format!("method '{}' was called without a receiver", function.name),
                ));
            };
            if let Some(first_local) = function.locals.first() {
                frame.locals[first_local.id.0] = receiver;
            }
        }

        if args.len() != function.params.len() {
            return Err(self.runtime_error(
                span,
                format!(
                    "function '{}' expects {} arguments, got {}",
                    function.name,
                    function.params.len(),
                    args.len()
                ),
            ));
        }

        for (param, value) in function.params.iter().zip(args) {
            frame.locals[param.0] = value;
        }

        let mut block_id = function.entry;
        loop {
            let block = function
                .block(block_id)
                .cloned()
                .ok_or_else(|| self.runtime_error(span, format!("unknown block id {}", block_id.0)))?;
            for statement in block.statements {
                self.exec_statement(&mut frame, statement)?;
            }

            match block.terminator.kind {
                ir::TerminatorKind::Goto(target) => block_id = target,
                ir::TerminatorKind::Branch {
                    condition,
                    then_block,
                    else_block,
                } => {
                    if self.eval_operand(&frame, &condition, block.terminator.span)?.as_bool(
                        self,
                        block.terminator.span,
                        "branch condition",
                    )? {
                        block_id = then_block;
                    } else {
                        block_id = else_block;
                    }
                }
                ir::TerminatorKind::Switch {
                    scrutinee,
                    arms,
                    default,
                } => {
                    let scrutinee = self.eval_operand(&frame, &scrutinee, block.terminator.span)?;
                    let mut matched = None;
                    for arm in arms {
                        if self.switch_matches(&scrutinee, &arm.value) {
                            matched = Some(arm.target);
                            break;
                        }
                    }
                    block_id = matched.unwrap_or(default);
                }
                ir::TerminatorKind::Return(value) => {
                    return value
                        .map(|operand| self.eval_operand(&frame, &operand, block.terminator.span))
                        .transpose()?
                        .map_or(Ok(Value::Unit), Ok);
                }
                ir::TerminatorKind::Unreachable => {
                    return Err(self.runtime_error(
                        block.terminator.span,
                        format!("entered unreachable block in '{}'", function.name),
                    ));
                }
            }
        }
    }

    fn exec_statement(&mut self, frame: &mut Frame, statement: ir::Statement) -> Result<(), Diagnostic> {
        match statement.kind {
            ir::StatementKind::Assign { target, value } => {
                let value = self.eval_rvalue(&value, Some(frame), statement.span)?;
                self.assign_place(frame, &target, value, statement.span)
            }
            ir::StatementKind::Eval { value } => {
                let _ = self.eval_rvalue(&value, Some(frame), statement.span)?;
                Ok(())
            }
        }
    }

    fn eval_rvalue(
        &mut self,
        value: &ir::RValue,
        frame: Option<&Frame>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match value {
            ir::RValue::Use(operand) => self.eval_operand_ref(frame, operand, span),
            ir::RValue::Unary { op, operand } => {
                let operand = self.eval_operand_ref(frame, operand, span)?;
                self.eval_unary(*op, operand, span)
            }
            ir::RValue::Binary { op, left, right } => {
                let left = self.eval_operand_ref(frame, left, span)?;
                let right = self.eval_operand_ref(frame, right, span)?;
                self.eval_binary(*op, left, right, span)
            }
            ir::RValue::Call { callee, args } => {
                let args = args
                    .iter()
                    .map(|arg| self.eval_operand_ref(frame, arg, span))
                    .collect::<Result<Vec<_>, _>>()?;
                self.invoke_callee(frame, callee, args, span)
            }
            ir::RValue::Tuple(items) => Ok(Value::Tuple(
                items
                    .iter()
                    .map(|item| self.eval_operand_ref(frame, item, span))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            ir::RValue::List(items) => Ok(Value::List(Rc::new(RefCell::new(
                items
                    .iter()
                    .map(|item| self.eval_operand_ref(frame, item, span))
                    .collect::<Result<Vec<_>, _>>()?,
            )))),
            ir::RValue::Record(fields) => Ok(Value::Record(Rc::new(RefCell::new(
                fields
                    .iter()
                    .map(|field| {
                        Ok((
                            field.name.clone(),
                            self.eval_operand_ref(frame, &field.value, span)?,
                        ))
                    })
                    .collect::<Result<Vec<_>, Diagnostic>>()?,
            )))),
            ir::RValue::Construct { ty, fields } => self.construct_value(frame, ty, fields, span),
            ir::RValue::Variant {
                enum_name,
                case_name,
                fields,
            } => self.construct_variant_from_named(frame, enum_name, case_name, fields, span),
            ir::RValue::Field { base, name } => {
                let base = self.eval_operand_ref(frame, base, span)?;
                self.get_member(base, name, span)
            }
            ir::RValue::Index { base, index } => {
                let base = self.eval_operand_ref(frame, base, span)?;
                let index = self.eval_operand_ref(frame, index, span)?;
                self.index_value(base, index, span)
            }
            ir::RValue::Cast { operand, .. } => self.eval_operand_ref(frame, operand, span),
            ir::RValue::TypeTest { operand, ty } => {
                let operand = self.eval_operand_ref(frame, operand, span)?;
                Ok(Value::Bool(self.value_matches_type(&operand, ty)))
            }
            ir::RValue::Closure { .. } => Err(self.runtime_error(
                span,
                "closure execution is not implemented in the Rust IR interpreter yet",
            )),
        }
    }

    fn eval_operand(
        &mut self,
        frame: &Frame,
        operand: &ir::Operand,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        self.eval_operand_ref(Some(frame), operand, span)
    }

    fn eval_operand_ref(
        &mut self,
        frame: Option<&Frame>,
        operand: &ir::Operand,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match operand {
            ir::Operand::Copy(place) | ir::Operand::Move(place) => self.read_place(frame, place, span),
            ir::Operand::Const(constant) => Ok(self.constant_value(constant)),
        }
    }

    fn constant_value(&self, constant: &ir::Constant) -> Value {
        match constant {
            ir::Constant::Unit => Value::Unit,
            ir::Constant::Bool(value) => Value::Bool(*value),
            ir::Constant::Int(value) => Value::Int(*value),
            ir::Constant::Float(value) => Value::Float(*value),
            ir::Constant::String(value) => Value::String(decode_string_literal(value)),
        }
    }

    fn read_place(
        &mut self,
        frame: Option<&Frame>,
        place: &ir::Place,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match place {
            ir::Place::Local(id) => frame
                .and_then(|frame| frame.locals.get(id.0).cloned())
                .ok_or_else(|| self.runtime_error(span, format!("unknown local {}", id.0))),
            ir::Place::Global(id) => self
                .globals
                .get(id.0)
                .cloned()
                .ok_or_else(|| self.runtime_error(span, format!("unknown global {}", id.0))),
            ir::Place::Field { base, name } => {
                let base = self.eval_operand_ref(frame, base, span)?;
                self.get_member(base, name, span)
            }
            ir::Place::Index { base, index } => {
                let base = self.eval_operand_ref(frame, base, span)?;
                let index = self.eval_operand_ref(frame, index, span)?;
                self.index_value(base, index, span)
            }
        }
    }

    fn assign_place(
        &mut self,
        frame: &mut Frame,
        place: &ir::Place,
        value: Value,
        span: Option<Span>,
    ) -> Result<(), Diagnostic> {
        match place {
            ir::Place::Local(id) => {
                let Some(slot) = frame.locals.get_mut(id.0) else {
                    return Err(self.runtime_error(span, format!("unknown local {}", id.0)));
                };
                *slot = value;
                Ok(())
            }
            ir::Place::Global(id) => {
                let Some(slot) = self.globals.get_mut(id.0) else {
                    return Err(self.runtime_error(span, format!("unknown global {}", id.0)));
                };
                *slot = value;
                Ok(())
            }
            ir::Place::Field { base, name } => {
                let base = self.eval_operand(frame, base, span)?;
                self.set_member(base, name, value, span)
            }
            ir::Place::Index { base, index } => {
                let base = self.eval_operand(frame, base, span)?;
                let index = self.eval_operand(frame, index, span)?;
                self.set_index(base, index, value, span)
            }
        }
    }

    fn invoke_callee(
        &mut self,
        frame: Option<&Frame>,
        callee: &ir::Callee,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match callee {
            ir::Callee::Direct(id) => self.call_function(*id, None, args, span),
            ir::Callee::Indirect(value) => {
                let callee = self.eval_operand_ref(frame, value, span)?;
                self.invoke_value(callee, args, span)
            }
            ir::Callee::Method { receiver, method } => {
                let receiver = self.eval_operand_ref(frame, receiver, span)?;
                self.invoke_method(receiver, method, args, span)
            }
            ir::Callee::Intrinsic(intrinsic) => self.invoke_intrinsic(intrinsic, args, span),
            ir::Callee::Named { path } => self.invoke_named_path(frame, path, args, span),
        }
    }

    fn invoke_value(
        &mut self,
        callee: Value,
        _args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match callee {
            Value::Object(object) => {
                let object = object.borrow();
                Err(self.runtime_error(
                    span,
                    format!("value '{}' is not directly callable", object.type_name),
                ))
            }
            _ => Err(self.runtime_error(span, "indirect callable values are not implemented yet")),
        }
    }

    fn invoke_named_path(
        &mut self,
        frame: Option<&Frame>,
        path: &[String],
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        if path.is_empty() {
            return Err(self.runtime_error(span, "empty callee path"));
        }

        if path.len() == 1 {
            let name = &path[0];
            if let Some(function) = self.lookup_function(name) {
                return self.call_function(function, None, args, span);
            }
            return self.invoke_root_named(name, args, span);
        }

        if let Some(receiver) = self.resolve_runtime_path(frame, &path[..path.len() - 1], span)? {
            return self.invoke_method(receiver, &path[path.len() - 1], args, span);
        }

        if path[0] == "OS" && path.len() == 2 {
            return self.invoke_os_method(&path[1], args, span);
        }

        if path.len() == 2 {
            if let Some(singleton) = self.lookup_object_singleton(&path[0], span)? {
                return self.invoke_method(singleton, &path[1], args, span);
            }
            if let Some(value) = self.construct_named_path(path, args.clone(), span)? {
                return Ok(value);
            }
        }

        Err(self.runtime_error(
            span,
            format!("unsupported named callee path '{}'", path.join(".")),
        ))
    }

    fn resolve_runtime_path(
        &mut self,
        frame: Option<&Frame>,
        path: &[String],
        span: Option<Span>,
    ) -> Result<Option<Value>, Diagnostic> {
        let Some(first) = path.first() else {
            return Ok(None);
        };
        let Some(mut value) = self.lookup_runtime_value(frame, first) else {
            return Ok(None);
        };
        for segment in &path[1..] {
            value = self.get_member(value, segment, span)?;
        }
        Ok(Some(value))
    }

    fn invoke_root_named(
        &mut self,
        name: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        if let Some(value) = self.construct_builtin(name, &args, span)? {
            return Ok(value);
        }
        if let Some(value) = self.construct_named_type(name, args.clone(), span)? {
            return Ok(value);
        }
        if let Some(value) = self.lookup_runtime_value(None, name) {
            return self.invoke_value(value, args, span);
        }
        Err(self.runtime_error(
            span,
            format!("unknown callable '{}'", name),
        ))
    }

    fn construct_named_path(
        &mut self,
        path: &[String],
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Option<Value>, Diagnostic> {
        if path.len() != 2 {
            return Ok(None);
        }
        let type_name = &path[0];
        let member = &path[1];

        if let Some(ty) = self.lookup_type(type_name) {
            if ty.kind == crate::ast::TypeKind::Enum
                && ty.enum_cases.iter().any(|case| case.name == *member)
            {
                return self
                    .construct_enum_case(Some(type_name), member, args, span)
                    .map(Some);
            }
        }
        Ok(None)
    }

    fn construct_builtin(
        &mut self,
        name: &str,
        args: &[Value],
        span: Option<Span>,
    ) -> Result<Option<Value>, Diagnostic> {
        let value = match name {
            "Range" => {
                if !(args.len() == 2 || args.len() == 3) {
                    return Err(self.runtime_error(
                        span,
                        format!("Range expects 2 or 3 arguments, got {}", args.len()),
                    ));
                }
                let start = args[0].as_int(self, span, "Range start")?;
                let end = args[1].as_int(self, span, "Range end")?;
                let step = if args.len() == 3 {
                    args[2].as_int(self, span, "Range step")?
                } else if start <= end {
                    1
                } else {
                    -1
                };
                Some(Value::Iterator(Rc::new(RefCell::new(IteratorState::Range {
                    current: start,
                    end,
                    step,
                }))))
            }
            "List" | "Array" => Some(Value::List(Rc::new(RefCell::new(args.to_vec())))),
            "Some" => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "Some expects 1 argument"));
                }
                Some(Value::option_some(args[0].clone()))
            }
            "None" => {
                if !args.is_empty() {
                    return Err(self.runtime_error(span, "None expects 0 arguments"));
                }
                Some(Value::option_none())
            }
            "Ok" => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "Ok expects 1 argument"));
                }
                Some(Value::result_ok(args[0].clone()))
            }
            "Err" => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "Err expects 1 argument"));
                }
                Some(Value::result_err(args[0].clone()))
            }
            "Left" => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "Left expects 1 argument"));
                }
                Some(Value::either_left(args[0].clone()))
            }
            "Right" => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "Right expects 1 argument"));
                }
                Some(Value::either_right(args[0].clone()))
            }
            _ => None,
        };
        Ok(value)
    }

    fn construct_named_type(
        &mut self,
        type_name: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Option<Value>, Diagnostic> {
        let Some(ty) = self.lookup_type(type_name).cloned() else {
            return Ok(None);
        };
        if ty.kind == crate::ast::TypeKind::Enum {
            return Err(self.runtime_error(
                span,
                format!("enum '{}' must be constructed through a case", type_name),
            ));
        }

        let object = Value::Object(Rc::new(RefCell::new(ObjectValue {
            type_name: type_name.to_string(),
            fields: ty
                .fields
                .iter()
                .map(|field| (field.name.clone(), Value::default_for_type(&field.ty)))
                .collect(),
        })));

        if let Some(init) = self.find_method(type_name, "init") {
            let receiver = object.clone();
            let _ = self.call_function(init, Some(receiver), args, span)?;
            return Ok(Some(object));
        }

        {
            let mut fields = match &object {
                Value::Object(object) => object.borrow_mut(),
                _ => unreachable!(),
            };
            if args.len() > fields.fields.len() {
                return Err(self.runtime_error(
                    span,
                    format!(
                        "constructor '{}' accepts at most {} positional fields, got {}",
                        type_name,
                        fields.fields.len(),
                        args.len()
                    ),
                ));
            }
            for (index, value) in args.into_iter().enumerate() {
                fields.fields[index].1 = value;
            }
        }

        Ok(Some(object))
    }

    fn construct_value(
        &mut self,
        frame: Option<&Frame>,
        ty: &ir::Type,
        fields: &[ir::NamedOperand],
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match ty {
            ir::Type::Named { name, .. } => {
                let args = fields
                    .iter()
                    .map(|field| self.eval_operand_ref(frame, &field.value, span))
                    .collect::<Result<Vec<_>, _>>()?;
                self.construct_named_type(name, args, span)?
                    .ok_or_else(|| self.runtime_error(span, format!("cannot construct type '{name}'")))
            }
            ir::Type::Record(field_types) => {
                let mut out = Vec::new();
                for field in field_types {
                    let value = fields
                        .iter()
                        .find(|named| named.name == field.name)
                        .map(|named| self.eval_operand_ref(frame, &named.value, span))
                        .transpose()?
                        .unwrap_or_else(|| Value::default_for_type(&field.ty));
                    out.push((field.name.clone(), value));
                }
                Ok(Value::Record(Rc::new(RefCell::new(out))))
            }
            _ => Err(self.runtime_error(
                span,
                "construct is only implemented for named and record types right now",
            )),
        }
    }

    fn construct_variant_from_named(
        &mut self,
        frame: Option<&Frame>,
        enum_name: &str,
        case_name: &str,
        fields: &[ir::NamedOperand],
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let values = fields
            .iter()
            .map(|field| {
                Ok((
                    field.name.clone(),
                    self.eval_operand_ref(frame, &field.value, span)?,
                ))
            })
            .collect::<Result<Vec<_>, Diagnostic>>()?;
        Ok(Value::Variant(Rc::new(VariantValue {
            enum_name: enum_name.to_string(),
            case_name: case_name.to_string(),
            fields: values,
        })))
    }

    fn construct_enum_case(
        &mut self,
        explicit_enum: Option<&str>,
        case_name: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let mut matches = self
            .program
            .types
            .iter()
            .filter(|ty| {
                ty.kind == crate::ast::TypeKind::Enum
                    && explicit_enum.is_none_or(|name| ty.name == name)
                    && ty.enum_cases.iter().any(|case| case.name == case_name)
            })
            .collect::<Vec<_>>();
        if matches.is_empty() {
            return Err(self.runtime_error(
                span,
                format!("unknown enum case '{}'", case_name),
            ));
        }
        if matches.len() > 1 {
            return Err(self.runtime_error(
                span,
                format!("enum case '{}' is ambiguous in this runtime", case_name),
            ));
        }
        let ty = matches.remove(0);
        let case = ty
            .enum_cases
            .iter()
            .find(|case| case.name == case_name)
            .expect("matched case");
        if args.len() != case.fields.len() {
            return Err(self.runtime_error(
                span,
                format!(
                    "enum case '{}.{}' expects {} arguments, got {}",
                    ty.name,
                    case_name,
                    case.fields.len(),
                    args.len()
                ),
            ));
        }
        Ok(Value::Variant(Rc::new(VariantValue {
            enum_name: ty.name.clone(),
            case_name: case_name.to_string(),
            fields: case
                .fields
                .iter()
                .zip(args)
                .map(|(field, value)| (field.name.clone(), value))
                .collect(),
        })))
    }

    fn invoke_intrinsic(
        &mut self,
        intrinsic: &ir::Intrinsic,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match intrinsic {
            ir::Intrinsic::Print => self.invoke_print(false, args),
            ir::Intrinsic::Println => self.invoke_print(true, args),
            ir::Intrinsic::Printf => self.invoke_printf(args, span),
            ir::Intrinsic::Panic => {
                let message = args.first().map(Value::render).unwrap_or_else(|| "panic".to_string());
                Err(self.runtime_error(span, message))
            }
            ir::Intrinsic::IterInit => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "IterInit expects 1 argument"));
                }
                self.iter_init(args.into_iter().next().expect("iter arg"), span)
            }
            ir::Intrinsic::IterHasNext => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "IterHasNext expects 1 argument"));
                }
                self.iter_has_next(args.into_iter().next().expect("iter arg"), span)
            }
            ir::Intrinsic::IterNext => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "IterNext expects 1 argument"));
                }
                self.iter_next(args.into_iter().next().expect("iter arg"), span)
            }
            ir::Intrinsic::ListAppend => {
                if args.len() != 2 {
                    return Err(self.runtime_error(span, "ListAppend expects 2 arguments"));
                }
                self.list_append(args[0].clone(), args[1].clone(), span)
            }
            ir::Intrinsic::UnwrapPresent => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "UnwrapPresent expects 1 argument"));
                }
                Ok(Value::Bool(self.unwrappable_present(&args[0])))
            }
            ir::Intrinsic::UnwrapValue => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "UnwrapValue expects 1 argument"));
                }
                self.unwrappable_value(&args[0], span)
            }
            ir::Intrinsic::VariantIs(case_name) => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "VariantIs expects 1 argument"));
                }
                Ok(Value::Bool(matches!(
                    &args[0],
                    Value::Variant(variant) if variant.case_name == *case_name
                )))
            }
            ir::Intrinsic::VariantField(field_name) => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "VariantField expects 1 argument"));
                }
                match &args[0] {
                    Value::Variant(variant) => variant
                        .fields
                        .iter()
                        .find(|(name, _)| name == field_name)
                        .map(|(_, value)| value.clone())
                        .ok_or_else(|| {
                            self.runtime_error(
                                span,
                                format!(
                                    "variant '{}.{}' has no field '{}'",
                                    variant.enum_name, variant.case_name, field_name
                                ),
                            )
                        }),
                    _ => Err(self.runtime_error(
                        span,
                        "VariantField expects an enum variant receiver",
                    )),
                }
            }
        }
    }

    fn invoke_os_method(
        &mut self,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match method {
            "print" => self.invoke_print(false, args),
            "println" => self.invoke_print(true, args),
            "printf" => self.invoke_printf(args, span),
            "panic" => {
                let message = args.first().map(Value::render).unwrap_or_else(|| "panic".to_string());
                Err(self.runtime_error(span, message))
            }
            _ => Err(self.runtime_error(
                span,
                format!("unknown OS method '{}'", method),
            )),
        }
    }

    fn invoke_print(&mut self, newline: bool, args: Vec<Value>) -> Result<Value, Diagnostic> {
        let rendered = args.iter().map(Value::render).collect::<Vec<_>>().join(" ");
        self.output.push_str(&rendered);
        if newline {
            self.output.push('\n');
        }
        Ok(Value::Unit)
    }

    fn invoke_printf(&mut self, args: Vec<Value>, span: Option<Span>) -> Result<Value, Diagnostic> {
        if args.is_empty() {
            return Err(self.runtime_error(span, "printf expects at least 1 argument"));
        }
        let mut text = args[0].render();
        for value in &args[1..] {
            if let Some(index) = text.find("{}") {
                text.replace_range(index..index + 2, &value.render());
            } else {
                text.push(' ');
                text.push_str(&value.render());
            }
        }
        self.output.push_str(&text);
        Ok(Value::Unit)
    }

    fn iter_init(&mut self, value: Value, span: Option<Span>) -> Result<Value, Diagnostic> {
        match value {
            Value::Iterator(iterator) => Ok(Value::Iterator(iterator)),
            Value::List(items) => Ok(Value::Iterator(Rc::new(RefCell::new(
                IteratorState::List { items, index: 0 },
            )))),
            _ => self.invoke_method(value, "iterator", Vec::new(), span),
        }
    }

    fn iter_has_next(&mut self, value: Value, span: Option<Span>) -> Result<Value, Diagnostic> {
        match value {
            Value::Iterator(iterator) => {
                let has_next = match &*iterator.borrow() {
                    IteratorState::List { items, index } => *index < items.borrow().len(),
                    IteratorState::Range { current, end, step } => {
                        if *step >= 0 {
                            *current < *end
                        } else {
                            *current > *end
                        }
                    }
                };
                Ok(Value::Bool(has_next))
            }
            _ => Err(self.runtime_error(span, "IterHasNext expects an iterator")),
        }
    }

    fn iter_next(&mut self, value: Value, span: Option<Span>) -> Result<Value, Diagnostic> {
        match value {
            Value::Iterator(iterator) => {
                let mut iterator = iterator.borrow_mut();
                match &mut *iterator {
                    IteratorState::List { items, index } => {
                        let items = items.borrow();
                        let Some(value) = items.get(*index).cloned() else {
                            return Err(self.runtime_error(span, "iterator is exhausted"));
                        };
                        *index += 1;
                        Ok(value)
                    }
                    IteratorState::Range { current, step, .. } => {
                        let value = *current;
                        *current += *step;
                        Ok(Value::Int(value))
                    }
                }
            }
            _ => Err(self.runtime_error(span, "IterNext expects an iterator")),
        }
    }

    fn list_append(
        &mut self,
        list: Value,
        value: Value,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match &list {
            Value::List(items) => {
                items.borrow_mut().push(value);
                Ok(list)
            }
            _ => Err(self.runtime_error(span, "ListAppend expects a List receiver")),
        }
    }

    fn unwrappable_present(&self, value: &Value) -> bool {
        match value {
            Value::Variant(variant) => match variant.enum_name.as_str() {
                "Option" => variant.case_name == "Some",
                "Result" => variant.case_name == "Ok",
                "Either" => variant.case_name == "Right",
                _ => false,
            },
            _ => false,
        }
    }

    fn unwrappable_value(&self, value: &Value, span: Option<Span>) -> Result<Value, Diagnostic> {
        match value {
            Value::Variant(variant) if self.unwrappable_present(value) => variant
                .fields
                .first()
                .map(|(_, value)| value.clone())
                .ok_or_else(|| self.runtime_error(span, "unwrappable value has no payload")),
            _ => Err(self.runtime_error(
                span,
                "attempted to unwrap a value without a success payload",
            )),
        }
    }

    fn invoke_method(
        &mut self,
        receiver: Value,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match &receiver {
            Value::List(items) => return self.invoke_list_method(receiver.clone(), items, method, args, span),
            Value::String(_) => return self.invoke_string_method(receiver.clone(), method, args, span),
            Value::Variant(variant) => {
                return self.invoke_variant_method(receiver.clone(), variant, method, args, span)
            }
            Value::Iterator(iterator) => {
                return self.invoke_iterator_method(receiver.clone(), iterator, method, args, span)
            }
            Value::Object(object) => {
                let type_name = object.borrow().type_name.clone();
                if let Some(function) = self.find_method(&type_name, method) {
                    return self.call_function(function, Some(receiver), args, span);
                }
            }
            _ => {}
        }

        Err(self.runtime_error(
            span,
            format!("method '{}' is not available on {}", method, receiver.render()),
        ))
    }

    fn invoke_list_method(
        &mut self,
        receiver: Value,
        items: &Rc<RefCell<Vec<Value>>>,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match method {
            "append" => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "List.append expects 1 argument"));
                }
                items.borrow_mut().push(args[0].clone());
                Ok(receiver)
            }
            "size" => {
                if !args.is_empty() {
                    return Err(self.runtime_error(span, "List.size expects 0 arguments"));
                }
                Ok(Value::Int(items.borrow().len() as i64))
            }
            "isEmpty" => {
                if !args.is_empty() {
                    return Err(self.runtime_error(span, "List.isEmpty expects 0 arguments"));
                }
                Ok(Value::Bool(items.borrow().is_empty()))
            }
            "get" => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "List.get expects 1 argument"));
                }
                let index = args[0].as_int(self, span, "List.get index")?;
                let value = items.borrow().get(index as usize).cloned();
                Ok(value.map_or_else(Value::option_none, Value::option_some))
            }
            "remove" => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "List.remove expects 1 argument"));
                }
                let index = args[0].as_int(self, span, "List.remove index")?;
                let mut items = items.borrow_mut();
                if index < 0 || index as usize >= items.len() {
                    return Ok(Value::option_none());
                }
                Ok(Value::option_some(items.remove(index as usize)))
            }
            "iterator" => {
                if !args.is_empty() {
                    return Err(self.runtime_error(span, "List.iterator expects 0 arguments"));
                }
                Ok(Value::Iterator(Rc::new(RefCell::new(IteratorState::List {
                    items: items.clone(),
                    index: 0,
                }))))
            }
            _ => Err(self.runtime_error(
                span,
                format!("unsupported List method '{}'", method),
            )),
        }
    }

    fn invoke_string_method(
        &mut self,
        receiver: Value,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let Value::String(text) = receiver else {
            unreachable!();
        };
        match method {
            "size" => {
                if !args.is_empty() {
                    return Err(self.runtime_error(span, "Str.size expects 0 arguments"));
                }
                Ok(Value::Int(text.chars().count() as i64))
            }
            _ => Err(self.runtime_error(
                span,
                format!("unsupported Str method '{}'", method),
            )),
        }
    }

    fn invoke_variant_method(
        &mut self,
        receiver: Value,
        variant: &Rc<VariantValue>,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match variant.enum_name.as_str() {
            "Option" => match method {
                "isSet" => Ok(Value::Bool(variant.case_name == "Some")),
                "isEmpty" => Ok(Value::Bool(variant.case_name != "Some")),
                "expect" => {
                    if !args.is_empty() {
                        return Err(self.runtime_error(span, "Option.expect expects 0 arguments"));
                    }
                    if variant.case_name != "Some" {
                        return Err(self.runtime_error(span, "Option has no value"));
                    }
                    Ok(variant.fields[0].1.clone())
                }
                "getOr" => {
                    if args.len() != 1 {
                        return Err(self.runtime_error(span, "Option.getOr expects 1 argument"));
                    }
                    if variant.case_name == "Some" {
                        Ok(variant.fields[0].1.clone())
                    } else {
                        Ok(args[0].clone())
                    }
                }
                _ => Err(self.runtime_error(
                    span,
                    format!("unsupported Option method '{}'", method),
                )),
            },
            "Result" => match method {
                "isOk" => Ok(Value::Bool(variant.case_name == "Ok")),
                "isErr" => Ok(Value::Bool(variant.case_name != "Ok")),
                "expect" => {
                    if variant.case_name == "Ok" {
                        Ok(variant.fields[0].1.clone())
                    } else {
                        Err(self.runtime_error(span, "Result has no success value"))
                    }
                }
                "getError" => {
                    if variant.case_name == "Err" {
                        Ok(variant.fields[0].1.clone())
                    } else {
                        Err(self.runtime_error(span, "Result has no error value"))
                    }
                }
                "getOr" => {
                    if args.len() != 1 {
                        return Err(self.runtime_error(span, "Result.getOr expects 1 argument"));
                    }
                    if variant.case_name == "Ok" {
                        Ok(variant.fields[0].1.clone())
                    } else {
                        Ok(args[0].clone())
                    }
                }
                _ => Err(self.runtime_error(
                    span,
                    format!("unsupported Result method '{}'", method),
                )),
            },
            "Either" => match method {
                "isLeft" => Ok(Value::Bool(variant.case_name == "Left")),
                "isRight" => Ok(Value::Bool(variant.case_name == "Right")),
                "expectLeft" => {
                    if variant.case_name == "Left" {
                        Ok(variant.fields[0].1.clone())
                    } else {
                        Err(self.runtime_error(span, "Either has no left value"))
                    }
                }
                "expectRight" => {
                    if variant.case_name == "Right" {
                        Ok(variant.fields[0].1.clone())
                    } else {
                        Err(self.runtime_error(span, "Either has no right value"))
                    }
                }
                "getOr" => {
                    if args.len() != 1 {
                        return Err(self.runtime_error(span, "Either.getOr expects 1 argument"));
                    }
                    if variant.case_name == "Right" {
                        Ok(variant.fields[0].1.clone())
                    } else {
                        Ok(args[0].clone())
                    }
                }
                _ => Err(self.runtime_error(
                    span,
                    format!("unsupported Either method '{}'", method),
                )),
            },
            _ => self.invoke_user_variant_method(receiver, method, args, span),
        }
    }

    fn invoke_user_variant_method(
        &mut self,
        receiver: Value,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let Value::Variant(variant) = &receiver else {
            unreachable!();
        };
        if let Some(function) = self.find_method(&variant.enum_name, method) {
            return self.call_function(function, Some(receiver), args, span);
        }
        Err(self.runtime_error(
            span,
            format!(
                "method '{}' is not available on variant '{}.{}'",
                method, variant.enum_name, variant.case_name
            ),
        ))
    }

    fn invoke_iterator_method(
        &mut self,
        receiver: Value,
        iterator: &Rc<RefCell<IteratorState>>,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        if !args.is_empty() {
            return Err(self.runtime_error(
                span,
                format!("iterator method '{}' expects 0 arguments", method),
            ));
        }
        match method {
            "hasNext" => self.iter_has_next(Value::Iterator(iterator.clone()), span),
            "next" => self.iter_next(receiver, span),
            _ => Err(self.runtime_error(
                span,
                format!("unsupported Iterator method '{}'", method),
            )),
        }
    }

    fn lookup_runtime_value(&self, frame: Option<&Frame>, name: &str) -> Option<Value> {
        frame
            .and_then(|frame| self.lookup_local_by_name(frame, name))
            .or_else(|| self.lookup_global_by_name(name))
    }

    fn lookup_local_by_name(&self, frame: &Frame, name: &str) -> Option<Value> {
        self.program
            .function(frame.function)?
            .locals
            .iter()
            .find(|local| local.name == name)
            .and_then(|local| frame.locals.get(local.id.0).cloned())
    }

    fn lookup_global_by_name(&self, name: &str) -> Option<Value> {
        self.program
            .globals
            .iter()
            .find(|global| global.name == name)
            .and_then(|global| self.globals.get(global.id.0).cloned())
    }

    fn lookup_function(&self, name: &str) -> Option<ir::FunctionId> {
        self.program
            .functions
            .iter()
            .find(|function| function.name == name)
            .map(|function| function.id)
    }

    fn lookup_type(&self, name: &str) -> Option<&ir::TypeDef> {
        self.program.types.iter().find(|ty| ty.name == name)
    }

    fn lookup_object_singleton(
        &mut self,
        name: &str,
        span: Option<Span>,
    ) -> Result<Option<Value>, Diagnostic> {
        let Some(ty) = self.lookup_type(name).cloned() else {
            return Ok(None);
        };
        if ty.kind != crate::ast::TypeKind::Object {
            return Ok(None);
        }
        if let Some(existing) = &self.object_singletons[ty.id.0] {
            return Ok(Some(existing.clone()));
        }
        let value = Value::Object(Rc::new(RefCell::new(ObjectValue {
            type_name: ty.name.clone(),
            fields: ty
                .fields
                .iter()
                .map(|field| (field.name.clone(), Value::default_for_type(&field.ty)))
                .collect(),
        })));
        if let Some(init) = self.find_method(&ty.name, "init") {
            let _ = self.call_function(init, Some(value.clone()), Vec::new(), span)?;
        }
        self.object_singletons[ty.id.0] = Some(value.clone());
        Ok(Some(value))
    }

    fn find_method(&self, owner: &str, method: &str) -> Option<ir::FunctionId> {
        let ty = self.lookup_type(owner)?;
        ty.methods.iter().copied().find(|id| {
            self.program
                .function(*id)
                .is_some_and(|function| function.name == method)
        })
    }

    fn get_member(
        &self,
        base: Value,
        name: &str,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match base {
            Value::Object(object) => {
                let object = object.borrow();
                lookup_named_field(&object.fields, name).ok_or_else(|| {
                    self.runtime_error(
                        span,
                        format!("object '{}' has no field '{}'", object.type_name, name),
                    )
                })
            }
            Value::Record(fields) => lookup_named_field(&fields.borrow(), name).ok_or_else(|| {
                self.runtime_error(span, format!("record has no field '{}'", name))
            }),
            Value::Variant(variant) => lookup_named_field(&variant.fields, name).ok_or_else(|| {
                self.runtime_error(
                    span,
                    format!(
                        "variant '{}.{}' has no field '{}'",
                        variant.enum_name, variant.case_name, name
                    ),
                )
            }),
            Value::Tuple(items) => tuple_member(&items, name).ok_or_else(|| {
                self.runtime_error(span, format!("tuple has no member '{}'", name))
            }),
            _ => Err(self.runtime_error(
                span,
                format!("cannot access field '{}' on {}", name, base.render()),
            )),
        }
    }

    fn set_member(
        &mut self,
        base: Value,
        name: &str,
        value: Value,
        span: Option<Span>,
    ) -> Result<(), Diagnostic> {
        match base {
            Value::Object(object) => set_named_field(&mut object.borrow_mut().fields, name, value)
                .ok_or_else(|| {
                    self.runtime_error(span, format!("object field '{}' does not exist", name))
                }),
            Value::Record(fields) => set_named_field(&mut fields.borrow_mut(), name, value)
                .ok_or_else(|| self.runtime_error(span, format!("record field '{}' does not exist", name))),
            _ => Err(self.runtime_error(
                span,
                format!("cannot assign field '{}' on {}", name, base.render()),
            )),
        }
    }

    fn index_value(
        &self,
        base: Value,
        index: Value,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let index = index.as_int(self, span, "index")?;
        if index < 0 {
            return Err(self.runtime_error(span, "index must be non-negative"));
        }
        let index = index as usize;
        match base {
            Value::List(items) => items
                .borrow()
                .get(index)
                .cloned()
                .ok_or_else(|| self.runtime_error(span, format!("list index {} out of bounds", index))),
            Value::Tuple(items) => items
                .get(index)
                .cloned()
                .ok_or_else(|| self.runtime_error(span, format!("tuple index {} out of bounds", index))),
            _ => Err(self.runtime_error(
                span,
                format!("cannot index into {}", base.render()),
            )),
        }
    }

    fn set_index(
        &mut self,
        base: Value,
        index: Value,
        value: Value,
        span: Option<Span>,
    ) -> Result<(), Diagnostic> {
        let index = index.as_int(self, span, "index")?;
        if index < 0 {
            return Err(self.runtime_error(span, "index must be non-negative"));
        }
        let index = index as usize;
        match base {
            Value::List(items) => {
                let mut items = items.borrow_mut();
                let Some(slot) = items.get_mut(index) else {
                    return Err(self.runtime_error(span, format!("list index {} out of bounds", index)));
                };
                *slot = value;
                Ok(())
            }
            _ => Err(self.runtime_error(
                span,
                format!("cannot assign index on {}", base.render()),
            )),
        }
    }

    fn eval_unary(
        &self,
        op: ir::UnaryOp,
        operand: Value,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match op {
            ir::UnaryOp::Neg => match operand {
                Value::Int(value) => Ok(Value::Int(-value)),
                Value::Float(value) => Ok(Value::Float(-value)),
                _ => Err(self.runtime_error(span, "unary '-' expects Int or Float")),
            },
            ir::UnaryOp::Not => Ok(Value::Bool(!operand.as_bool(self, span, "logical not")?)),
        }
    }

    fn eval_binary(
        &self,
        op: ir::BinaryOp,
        left: Value,
        right: Value,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match op {
            ir::BinaryOp::Add => match (&left, &right) {
                (Value::Int(lhs), Value::Int(rhs)) => Ok(Value::Int(lhs + rhs)),
                (Value::Float(lhs), Value::Float(rhs)) => Ok(Value::Float(lhs + rhs)),
                (Value::Int(lhs), Value::Float(rhs)) => Ok(Value::Float(*lhs as f64 + rhs)),
                (Value::Float(lhs), Value::Int(rhs)) => Ok(Value::Float(lhs + *rhs as f64)),
                (Value::String(_), _) | (_, Value::String(_)) => {
                    Ok(Value::String(format!("{}{}", left.render(), right.render())))
                }
                _ => Err(self.runtime_error(span, "binary '+' expects numeric or string values")),
            },
            ir::BinaryOp::Sub => numeric_binary(left, right, span, |lhs, rhs| lhs - rhs, |lhs, rhs| {
                lhs - rhs
            }, self),
            ir::BinaryOp::Mul => numeric_binary(left, right, span, |lhs, rhs| lhs * rhs, |lhs, rhs| {
                lhs * rhs
            }, self),
            ir::BinaryOp::Div => numeric_binary(left, right, span, |lhs, rhs| lhs / rhs, |lhs, rhs| {
                lhs / rhs
            }, self),
            ir::BinaryOp::Mod => match (left, right) {
                (Value::Int(lhs), Value::Int(rhs)) => Ok(Value::Int(lhs % rhs)),
                _ => Err(self.runtime_error(span, "binary '%' expects Int values")),
            },
            ir::BinaryOp::Eq => Ok(Value::Bool(values_equal(&left, &right))),
            ir::BinaryOp::NotEq => Ok(Value::Bool(!values_equal(&left, &right))),
            ir::BinaryOp::Less => compare_binary(left, right, span, |lhs, rhs| lhs < rhs, self),
            ir::BinaryOp::LessEq => compare_binary(left, right, span, |lhs, rhs| lhs <= rhs, self),
            ir::BinaryOp::Greater => compare_binary(left, right, span, |lhs, rhs| lhs > rhs, self),
            ir::BinaryOp::GreaterEq => compare_binary(left, right, span, |lhs, rhs| lhs >= rhs, self),
            ir::BinaryOp::And => Ok(Value::Bool(
                left.as_bool(self, span, "left side of &&")?
                    && right.as_bool(self, span, "right side of &&")?,
            )),
            ir::BinaryOp::Or => Ok(Value::Bool(
                left.as_bool(self, span, "left side of ||")?
                    || right.as_bool(self, span, "right side of ||")?,
            )),
            ir::BinaryOp::BitAnd => match (left, right) {
                (Value::Int(lhs), Value::Int(rhs)) => Ok(Value::Int(lhs & rhs)),
                _ => Err(self.runtime_error(span, "binary '&' expects Int values")),
            },
            ir::BinaryOp::BitOr => match (left, right) {
                (Value::Int(lhs), Value::Int(rhs)) => Ok(Value::Int(lhs | rhs)),
                _ => Err(self.runtime_error(span, "binary '|' expects Int values")),
            },
            ir::BinaryOp::Concat => match (left, right) {
                (Value::List(lhs), Value::List(rhs)) => {
                    let mut items = lhs.borrow().clone();
                    items.extend(rhs.borrow().iter().cloned());
                    Ok(Value::List(Rc::new(RefCell::new(items))))
                }
                _ => Err(self.runtime_error(span, "binary '++' expects List values")),
            },
        }
    }

    fn switch_matches(&self, value: &Value, arm: &ir::SwitchValue) -> bool {
        match arm {
            ir::SwitchValue::Bool(expected) => matches!(value, Value::Bool(actual) if actual == expected),
            ir::SwitchValue::Int(expected) => matches!(value, Value::Int(actual) if actual == expected),
            ir::SwitchValue::String(expected) => {
                matches!(value, Value::String(actual) if actual == expected)
            }
            ir::SwitchValue::EnumCase(expected) => {
                matches!(value, Value::Variant(variant) if variant.case_name == *expected)
            }
        }
    }

    fn value_matches_type(&self, value: &Value, ty: &ir::Type) -> bool {
        match ty {
            ir::Type::Unknown => true,
            ir::Type::Never => false,
            ir::Type::Unit => matches!(value, Value::Unit),
            ir::Type::Bool => matches!(value, Value::Bool(_)),
            ir::Type::Int => matches!(value, Value::Int(_)),
            ir::Type::Float => matches!(value, Value::Float(_)),
            ir::Type::Str => matches!(value, Value::String(_)),
            ir::Type::Named { name, .. } => match value {
                Value::List(_) => name == "List" || name == "Array",
                Value::Iterator(_) => name == "Iterator" || name == "IntRange",
                Value::Object(object) => object.borrow().type_name == *name,
                Value::Variant(variant) => variant.enum_name == *name,
                Value::String(_) => name == "Str",
                Value::Int(_) => name == "Int" || name == "Int64",
                Value::Float(_) => name == "Float" || name == "Float64",
                Value::Bool(_) => name == "Bool",
                Value::Unit => name == "Unit",
                _ => false,
            },
            ir::Type::Tuple(items) => match value {
                Value::Tuple(values) => {
                    values.len() == items.len()
                        && values
                            .iter()
                            .zip(items)
                            .all(|(value, ty)| self.value_matches_type(value, ty))
                }
                _ => false,
            },
            ir::Type::Record(fields) => match value {
                Value::Record(record) => fields.iter().all(|field| {
                    lookup_named_field(&record.borrow(), &field.name)
                        .is_some_and(|value| self.value_matches_type(&value, &field.ty))
                }),
                _ => false,
            },
            ir::Type::Function { .. } => false,
            ir::Type::TypeParam(_) => true,
        }
    }

    fn runtime_error(&self, span: Option<Span>, message: impl Into<String>) -> Diagnostic {
        Diagnostic::error(
            "runtime_error",
            message.into(),
            span.unwrap_or_else(default_span),
        )
    }
}

impl Value {
    fn as_bool(
        &self,
        in_: &Interpreter<'_>,
        span: Option<Span>,
        context: &str,
    ) -> Result<bool, Diagnostic> {
        match self {
            Value::Bool(value) => Ok(*value),
            _ => Err(in_.runtime_error(
                span,
                format!("{context} expects Bool, got {}", self.render()),
            )),
        }
    }

    fn as_int(
        &self,
        in_: &Interpreter<'_>,
        span: Option<Span>,
        context: &str,
    ) -> Result<i64, Diagnostic> {
        match self {
            Value::Int(value) => Ok(*value),
            _ => Err(in_.runtime_error(
                span,
                format!("{context} expects Int, got {}", self.render()),
            )),
        }
    }

    fn as_number(
        &self,
        in_: &Interpreter<'_>,
        span: Option<Span>,
        context: &str,
    ) -> Result<f64, Diagnostic> {
        match self {
            Value::Int(value) => Ok(*value as f64),
            Value::Float(value) => Ok(*value),
            _ => Err(in_.runtime_error(
                span,
                format!("{context} expects numeric value, got {}", self.render()),
            )),
        }
    }
}

fn lookup_named_field(fields: &[(String, Value)], name: &str) -> Option<Value> {
    fields
        .iter()
        .find(|(field_name, _)| field_name == name)
        .map(|(_, value)| value.clone())
}

fn set_named_field(fields: &mut [(String, Value)], name: &str, value: Value) -> Option<()> {
    let field = fields.iter_mut().find(|(field_name, _)| field_name == name)?;
    field.1 = value;
    Some(())
}

fn tuple_member(items: &[Value], name: &str) -> Option<Value> {
    let index = name.strip_prefix('_')?.parse::<usize>().ok()?;
    items.get(index.checked_sub(1)?).cloned()
}

fn values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Unit, Value::Unit) => true,
        (Value::Bool(lhs), Value::Bool(rhs)) => lhs == rhs,
        (Value::Int(lhs), Value::Int(rhs)) => lhs == rhs,
        (Value::Float(lhs), Value::Float(rhs)) => lhs == rhs,
        (Value::String(lhs), Value::String(rhs)) => lhs == rhs,
        (Value::Tuple(lhs), Value::Tuple(rhs)) => {
            lhs.len() == rhs.len() && lhs.iter().zip(rhs).all(|(lhs, rhs)| values_equal(lhs, rhs))
        }
        (Value::List(lhs), Value::List(rhs)) => {
            let lhs = lhs.borrow();
            let rhs = rhs.borrow();
            lhs.len() == rhs.len() && lhs.iter().zip(rhs.iter()).all(|(lhs, rhs)| values_equal(lhs, rhs))
        }
        (Value::Record(lhs), Value::Record(rhs)) => {
            let lhs = lhs.borrow();
            let rhs = rhs.borrow();
            lhs.len() == rhs.len()
                && lhs
                    .iter()
                    .zip(rhs.iter())
                    .all(|((ln, lv), (rn, rv))| ln == rn && values_equal(lv, rv))
        }
        (Value::Object(lhs), Value::Object(rhs)) => {
            let lhs = lhs.borrow();
            let rhs = rhs.borrow();
            lhs.type_name == rhs.type_name
                && lhs.fields.len() == rhs.fields.len()
                && lhs
                    .fields
                    .iter()
                    .zip(rhs.fields.iter())
                    .all(|((ln, lv), (rn, rv))| ln == rn && values_equal(lv, rv))
        }
        (Value::Variant(lhs), Value::Variant(rhs)) => {
            lhs.enum_name == rhs.enum_name
                && lhs.case_name == rhs.case_name
                && lhs.fields.len() == rhs.fields.len()
                && lhs
                    .fields
                    .iter()
                    .zip(rhs.fields.iter())
                    .all(|((ln, lv), (rn, rv))| ln == rn && values_equal(lv, rv))
        }
        _ => false,
    }
}

fn numeric_binary(
    left: Value,
    right: Value,
    span: Option<Span>,
    int_op: impl FnOnce(i64, i64) -> i64,
    float_op: impl FnOnce(f64, f64) -> f64,
    in_: &Interpreter<'_>,
) -> Result<Value, Diagnostic> {
    match (left, right) {
        (Value::Int(lhs), Value::Int(rhs)) => Ok(Value::Int(int_op(lhs, rhs))),
        (lhs, rhs) => Ok(Value::Float(float_op(
            lhs.as_number(in_, span, "numeric binary operator")?,
            rhs.as_number(in_, span, "numeric binary operator")?,
        ))),
    }
}

fn compare_binary(
    left: Value,
    right: Value,
    span: Option<Span>,
    op: impl FnOnce(f64, f64) -> bool,
    in_: &Interpreter<'_>,
) -> Result<Value, Diagnostic> {
    Ok(Value::Bool(op(
        left.as_number(in_, span, "comparison")?,
        right.as_number(in_, span, "comparison")?,
    )))
}

fn default_span() -> Span {
    let pos = LineColumn::new(1, 1);
    Span::new(0, 0, pos, pos)
}

fn decode_string_literal(raw: &str) -> String {
    let body = raw
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(raw);
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('0') => out.push('\0'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SourceFile, lex, parse_program};

    fn lower_inline(src: &str) -> ir::Program {
        let file = SourceFile::new("test.lum", src);
        let lexed = lex(&file);
        assert!(lexed.diagnostics.is_empty(), "{:#?}", lexed.diagnostics);
        let parsed = parse_program(&lexed.tokens);
        assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
        let program = parsed.program.expect("program");
        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        lowered.program.expect("lowered program")
    }

    #[test]
    fn runs_class_methods_and_globals() {
        let program = lower_inline(
            r#"
            class Counter {
                hidden var count Int
            }

            impl Counter {
                def init(count Int) {
                    this.count = count
                }

                def bump(delta Int) Int {
                    this.count += delta
                    return this.count
                }
            }

            seed Int = 1

            def run() Int {
                c Counter = Counter(seed)
                return c.bump(2)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.return_value.as_deref(), Some("3"));
        assert!(run.output.is_empty());
    }

    #[test]
    fn runs_range_loops_and_println() {
        let program = lower_inline(
            r#"
            def main() Unit {
                var total Int = 0
                for item <- Range(1, 4) {
                    OS.println("range", item)
                    total += item
                }
                OS.println("total", total)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "range 1\nrange 2\nrange 3\ntotal 6\n");
        assert_eq!(run.return_value, None);
    }

    #[test]
    fn runs_match_for_yield_and_unwrap() {
        let program = lower_inline(
            r#"
            def main() Int {
                items = for item <- [1, 2, 3] yield {
                    item + 1
                }

                unwrap count <- Some(items.size())

                total Int = 0
                for item <- items {
                    total += item
                }

                OS.println("size", count)
                OS.println("total", total)

                return match count {
                    case 3 => 10
                    case _ => 20
                }
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "size 3\ntotal 9\n");
        assert_eq!(run.return_value.as_deref(), Some("10"));
    }

    #[test]
    fn runs_option_and_result_methods() {
        let program = lower_inline(
            r#"
            def main() Unit {
                some = Some(5)
                none = None()
                ok = Ok(9)
                err = Err("missing")
                OS.println("some", some.getOr(0))
                OS.println("none", none.isEmpty())
                OS.println("ok", ok.getOr(0))
                OS.println("err", err.getError())
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "some 5\nnone true\nok 9\nerr missing\n");
    }
}
