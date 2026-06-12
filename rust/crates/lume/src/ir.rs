//! Lowered Lume intermediate representation.
//!
//! This file defines the smaller execution-oriented form produced after
//! parsing, resolution, and typechecking. The IR strips away most surface
//! syntax and models programs as functions, locals, basic blocks, statements,
//! terminators, and typed values that the interpreter can execute directly.

use crate::{
    ast::{TypeKind, Visibility},
    source::Span,
};

/// Stable id for a lowered type definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId(pub usize);

/// Stable id for a lowered global slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GlobalId(pub usize);

/// Stable id for a lowered function body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FunctionId(pub usize);

/// Stable id for a local slot inside one function frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocalId(pub usize);

/// Stable id for a basic block inside one function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub usize);

/// A lowered module containing globals, functions, types, and entrypoints.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Program {
    pub module: Option<String>,
    pub globals: Vec<Global>,
    pub functions: Vec<Function>,
    pub types: Vec<TypeDef>,
    pub global_init: Option<FunctionId>,
    pub entry: Option<FunctionId>,
}

impl Program {
    pub fn new(module: Option<String>) -> Self {
        Self {
            module,
            ..Self::default()
        }
    }

    // add_type assigns a stable type id so later lowering and interpretation can
    // reference declarations without redoing name lookup.
    pub fn add_type(&mut self, mut ty: TypeDef) -> TypeId {
        let id = TypeId(self.types.len());
        ty.id = id;
        self.types.push(ty);
        id
    }

    // add_global records a top-level storage slot and returns its id for use by
    // functions or future module linking.
    pub fn add_global(&mut self, mut global: Global) -> GlobalId {
        let id = GlobalId(self.globals.len());
        global.id = id;
        self.globals.push(global);
        id
    }

    // add_function installs a function body into the program table and returns
    // a stable id that calls, methods, and closures can reference.
    pub fn add_function(&mut self, mut function: Function) -> FunctionId {
        let id = FunctionId(self.functions.len());
        function.id = id;
        self.functions.push(function);
        id
    }

    pub fn set_entry(&mut self, function: FunctionId) {
        self.entry = Some(function);
    }

    pub fn set_global_init(&mut self, function: FunctionId) {
        self.global_init = Some(function);
    }

    pub fn function(&self, id: FunctionId) -> Option<&Function> {
        self.functions.get(id.0)
    }

    pub fn function_mut(&mut self, id: FunctionId) -> Option<&mut Function> {
        self.functions.get_mut(id.0)
    }
}

/// A lowered type definition with fields, methods, and enum-case metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeDef {
    pub id: TypeId,
    pub visibility: Visibility,
    pub kind: TypeKind,
    pub name: String,
    pub type_params: Vec<String>,
    pub with_bounds: Vec<Type>,
    pub fields: Vec<Field>,
    pub methods: Vec<FunctionId>,
    pub enum_cases: Vec<EnumCase>,
    pub span: Option<Span>,
}

impl TypeDef {
    pub fn new(kind: TypeKind, name: impl Into<String>) -> Self {
        Self {
            id: TypeId(usize::MAX),
            visibility: Visibility::Default,
            kind,
            name: name.into(),
            type_params: Vec::new(),
            with_bounds: Vec::new(),
            fields: Vec::new(),
            methods: Vec::new(),
            enum_cases: Vec::new(),
            span: None,
        }
    }
}

/// A lowered enum case payload description.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumCase {
    pub name: String,
    pub fields: Vec<Field>,
    pub span: Option<Span>,
}

/// A lowered field description shared by types and enum cases.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub visibility: Visibility,
    pub mutable: bool,
    pub name: String,
    pub ty: Type,
    pub initializer: Option<Constant>,
    pub span: Option<Span>,
}

/// A top-level storage slot visible across lowered functions.
#[derive(Debug, Clone, PartialEq)]
pub struct Global {
    pub id: GlobalId,
    pub visibility: Visibility,
    pub mutable: bool,
    pub name: String,
    pub ty: Type,
    pub initializer: Option<RValue>,
    pub span: Option<Span>,
}

impl Global {
    pub fn new(name: impl Into<String>, ty: Type) -> Self {
        Self {
            id: GlobalId(usize::MAX),
            visibility: Visibility::Default,
            mutable: false,
            name: name.into(),
            ty,
            initializer: None,
            span: None,
        }
    }
}

/// The role a lowered function plays in the program.
#[derive(Debug, Clone, PartialEq)]
pub enum FunctionKind {
    TopLevel,
    Method { owner: TypeId },
    Local { parent: FunctionId },
    Lambda,
    Synthetic,
}

/// A lowered function body with locals and control-flow blocks.
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub id: FunctionId,
    pub visibility: Visibility,
    pub kind: FunctionKind,
    pub name: String,
    pub type_params: Vec<String>,
    pub params: Vec<LocalId>,
    pub locals: Vec<Local>,
    pub return_ty: Type,
    pub blocks: Vec<BasicBlock>,
    pub entry: BlockId,
    pub span: Option<Span>,
}

impl Function {
    pub fn new(name: impl Into<String>, kind: FunctionKind, return_ty: Type) -> Self {
        let entry = BlockId(0);
        Self {
            id: FunctionId(usize::MAX),
            visibility: Visibility::Default,
            kind,
            name: name.into(),
            type_params: Vec::new(),
            params: Vec::new(),
            locals: Vec::new(),
            return_ty,
            blocks: vec![BasicBlock::new(entry)],
            entry,
            span: None,
        }
    }

    // add_param reserves a local slot that is part of the public callable
    // signature and records it in declaration order.
    pub fn add_param(&mut self, name: impl Into<String>, ty: Type) -> LocalId {
        let id = self.add_local(name, ty, false, LocalKind::Param);
        self.params.push(id);
        id
    }

    // add_local reserves a named slot in the function frame. The returned id is
    // stable for the life of the function and is the primary storage handle in
    // statements and operands.
    pub fn add_local(
        &mut self,
        name: impl Into<String>,
        ty: Type,
        mutable: bool,
        kind: LocalKind,
    ) -> LocalId {
        let id = LocalId(self.locals.len());
        self.locals.push(Local {
            id,
            name: name.into(),
            ty,
            mutable,
            kind,
            span: None,
        });
        id
    }

    pub fn add_capture(&mut self, name: impl Into<String>, ty: Type) -> LocalId {
        self.add_local(name, ty, false, LocalKind::Capture)
    }

    pub fn add_temp(&mut self, ty: Type) -> LocalId {
        self.add_local(
            format!("tmp{}", self.locals.len()),
            ty,
            true,
            LocalKind::Temp,
        )
    }

    pub fn add_block(&mut self) -> BlockId {
        let id = BlockId(self.blocks.len());
        self.blocks.push(BasicBlock::new(id));
        id
    }

    pub fn block(&self, id: BlockId) -> Option<&BasicBlock> {
        self.blocks.get(id.0)
    }

    pub fn block_mut(&mut self, id: BlockId) -> Option<&mut BasicBlock> {
        self.blocks.get_mut(id.0)
    }
}

/// The purpose of a local slot inside one lowered function.
#[derive(Debug, Clone, PartialEq)]
pub enum LocalKind {
    Param,
    Binding,
    Capture,
    Temp,
}

/// A lowered local slot in a function frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Local {
    pub id: LocalId,
    pub name: String,
    pub ty: Type,
    pub mutable: bool,
    pub kind: LocalKind,
    pub span: Option<Span>,
}

/// A basic block of straight-line lowered statements ending in one terminator.
#[derive(Debug, Clone, PartialEq)]
pub struct BasicBlock {
    pub id: BlockId,
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

impl BasicBlock {
    pub fn new(id: BlockId) -> Self {
        Self {
            id,
            statements: Vec::new(),
            terminator: Terminator::unreachable(),
        }
    }

    pub fn push(&mut self, statement: Statement) {
        self.statements.push(statement);
    }

    pub fn set_terminator(&mut self, terminator: Terminator) {
        self.terminator = terminator;
    }
}

/// A lowered statement executed within a basic block.
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub span: Option<Span>,
    pub kind: StatementKind,
}

impl Statement {
    pub fn assign(target: Place, value: RValue) -> Self {
        Self {
            span: None,
            kind: StatementKind::Assign { target, value },
        }
    }

    pub fn eval(value: RValue) -> Self {
        Self {
            span: None,
            kind: StatementKind::Eval { value },
        }
    }
}

/// The two statement shapes used by the current IR.
#[derive(Debug, Clone, PartialEq)]
pub enum StatementKind {
    Assign { target: Place, value: RValue },
    Eval { value: RValue },
}

/// The final control-flow action for a basic block.
#[derive(Debug, Clone, PartialEq)]
pub struct Terminator {
    pub span: Option<Span>,
    pub kind: TerminatorKind,
}

impl Terminator {
    pub fn goto(target: BlockId) -> Self {
        Self {
            span: None,
            kind: TerminatorKind::Goto(target),
        }
    }

    pub fn branch(condition: Operand, then_block: BlockId, else_block: BlockId) -> Self {
        Self {
            span: None,
            kind: TerminatorKind::Branch {
                condition,
                then_block,
                else_block,
            },
        }
    }

    pub fn ret(value: Option<Operand>) -> Self {
        Self {
            span: None,
            kind: TerminatorKind::Return(value),
        }
    }

    pub fn unreachable() -> Self {
        Self {
            span: None,
            kind: TerminatorKind::Unreachable,
        }
    }
}

/// The CFG edge kinds used by the lowered interpreter.
#[derive(Debug, Clone, PartialEq)]
pub enum TerminatorKind {
    Goto(BlockId),
    Branch {
        condition: Operand,
        then_block: BlockId,
        else_block: BlockId,
    },
    Switch {
        scrutinee: Operand,
        arms: Vec<SwitchArm>,
        default: BlockId,
    },
    Return(Option<Operand>),
    Unreachable,
}

/// One arm inside a lowered `Switch` terminator.
#[derive(Debug, Clone, PartialEq)]
pub struct SwitchArm {
    pub value: SwitchValue,
    pub target: BlockId,
}

/// The simple discriminant values a lowered switch can branch on directly.
#[derive(Debug, Clone, PartialEq)]
pub enum SwitchValue {
    Bool(bool),
    Int(i64),
    String(String),
    EnumCase(String),
}

/// A writable storage location in lowered code.
#[derive(Debug, Clone, PartialEq)]
pub enum Place {
    Local(LocalId),
    Global(GlobalId),
    Field {
        base: Box<Operand>,
        name: String,
    },
    Index {
        base: Box<Operand>,
        index: Box<Operand>,
    },
}

/// A by-value or by-reference input to lowered statements and rvalues.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Copy(Box<Place>),
    Move(Box<Place>),
    Const(Constant),
}

/// An interpreter-ready constant value embedded directly in IR.
#[derive(Debug, Clone, PartialEq)]
pub enum Constant {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
}

/// The callable target of a lowered call expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Callee {
    Direct(FunctionId),
    Indirect(Operand),
    Method { receiver: Operand, method: String },
    Intrinsic(Intrinsic),
    Named { path: Vec<String> },
}

/// Built-in runtime operations that lowering models without user code.
#[derive(Debug, Clone, PartialEq)]
pub enum Intrinsic {
    Print,
    Println,
    Printf,
    Panic,
    IterInit,
    IterHasNext,
    IterNext,
    ListAppend,
    VariantIs(String),
    VariantField(String),
}

/// A value-producing lowered operation.
#[derive(Debug, Clone, PartialEq)]
pub enum RValue {
    Use(Operand),
    Unary {
        op: UnaryOp,
        operand: Operand,
    },
    Binary {
        op: BinaryOp,
        left: Operand,
        right: Operand,
    },
    Call {
        callee: Callee,
        args: Vec<Operand>,
        structural: bool,
    },
    Tuple(Vec<Operand>),
    List(Vec<Operand>),
    Record(Vec<NamedOperand>),
    RecordUpdate {
        base: Operand,
        updates: Vec<NamedOperand>,
    },
    Construct {
        ty: Type,
        fields: Vec<NamedOperand>,
    },
    Variant {
        enum_name: String,
        case_name: String,
        fields: Vec<NamedOperand>,
    },
    Field {
        base: Operand,
        name: String,
    },
    Index {
        base: Operand,
        index: Operand,
    },
    Cast {
        operand: Operand,
        ty: Type,
    },
    TypeTest {
        operand: Operand,
        ty: Type,
    },
    Closure {
        function: FunctionId,
        captures: Vec<Operand>,
    },
}

/// A named operand used for records, constructors, and enum payload fields.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedOperand {
    pub name: String,
    pub value: Operand,
}

/// Supported unary operators in lowered code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

/// Supported binary operators in lowered code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    NotEq,
    Less,
    LessEq,
    Greater,
    GreaterEq,
    And,
    Or,
    BitAnd,
    BitOr,
    Concat,
}

/// The lowered type vocabulary used by typechecking, lowering, and interpretation.
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Unknown,
    Never,
    Unit,
    Bool,
    Int,
    Float,
    Str,
    Named { name: String, args: Vec<Type> },
    Tuple(Vec<Type>),
    Record(Vec<NamedType>),
    Function { params: Vec<Type>, ret: Box<Type> },
    TypeParam(String),
}

impl Type {
    pub fn named(name: impl Into<String>) -> Self {
        Self::Named {
            name: name.into(),
            args: Vec::new(),
        }
    }

    pub fn list(item: Type) -> Self {
        Self::Named {
            name: "List".to_string(),
            args: vec![item],
        }
    }

    pub fn option(item: Type) -> Self {
        Self::Named {
            name: "Option".to_string(),
            args: vec![item],
        }
    }
}

/// A named field inside a lowered record type.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedType {
    pub name: String,
    pub ty: Type,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_assigns_stable_ids() {
        let mut program = Program::new(Some("demo".to_string()));

        let ty_id = program.add_type(TypeDef::new(TypeKind::Record, "Pair"));
        let global_id = program.add_global(Global::new("answer", Type::Int));
        let fn_id = program.add_function(Function::new("main", FunctionKind::TopLevel, Type::Int));

        assert_eq!(ty_id, TypeId(0));
        assert_eq!(global_id, GlobalId(0));
        assert_eq!(fn_id, FunctionId(0));
        assert_eq!(program.types[0].id, ty_id);
        assert_eq!(program.globals[0].id, global_id);
        assert_eq!(program.functions[0].id, fn_id);
    }

    #[test]
    fn function_tracks_params_locals_and_blocks() {
        let mut function = Function::new("sum", FunctionKind::TopLevel, Type::Int);
        let lhs = function.add_param("lhs", Type::Int);
        let rhs = function.add_param("rhs", Type::Int);
        let tmp = function.add_temp(Type::Int);
        let exit = function.add_block();

        assert_eq!(function.entry, BlockId(0));
        assert_eq!(function.params, vec![lhs, rhs]);
        assert_eq!(tmp, LocalId(2));
        assert_eq!(exit, BlockId(1));
        assert_eq!(function.locals.len(), 3);
        assert_eq!(function.blocks.len(), 2);
    }

    #[test]
    fn blocks_hold_lowered_statements_and_terminators() {
        let mut function = Function::new("main", FunctionKind::TopLevel, Type::Int);
        let value = function.add_local("value", Type::Int, true, LocalKind::Binding);
        let then_block = function.add_block();
        let else_block = function.add_block();

        let entry = function.block_mut(function.entry).expect("entry block");
        entry.push(Statement::assign(
            Place::Local(value),
            RValue::Binary {
                op: BinaryOp::Add,
                left: Operand::Const(Constant::Int(1)),
                right: Operand::Const(Constant::Int(2)),
            },
        ));
        entry.set_terminator(Terminator::branch(
            Operand::Const(Constant::Bool(true)),
            then_block,
            else_block,
        ));

        assert_eq!(entry.statements.len(), 1);
        assert!(matches!(
            entry.terminator.kind,
            TerminatorKind::Branch {
                then_block: BlockId(1),
                else_block: BlockId(2),
                ..
            }
        ));
    }
}
