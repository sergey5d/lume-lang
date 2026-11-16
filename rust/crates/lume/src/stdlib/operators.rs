//! Trusted intrinsic capability entries attached to standard-library types.

use crate::ast::{BinaryOp, UnaryOp};

use super::OperatorDescriptor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeRepresentation {
    Bool,
    Char,
    Int,
    Float,
    Decimal,
    Str,
}

pub(super) struct TypeCapabilities {
    pub name: &'static str,
    pub representation: Option<NativeRepresentation>,
    pub operations: &'static [OperatorDescriptor],
}

use BinaryOp::{Add, And, Div, Eq, Greater, GreaterEq, Less, LessEq, Mod, Mul, NotEq, Or, Sub};
use NativeRepresentation as Representation;
use OperatorDescriptor::{Binary, Extract, Index, IndexAssign, Iterate, Unary};

const NUMERIC: &[OperatorDescriptor] = &[
    Binary(Add),
    Binary(Sub),
    Binary(Mul),
    Binary(Div),
    Binary(Mod),
    Binary(Eq),
    Binary(NotEq),
    Binary(Less),
    Binary(LessEq),
    Binary(Greater),
    Binary(GreaterEq),
    Unary(UnaryOp::Neg),
];
const STRING: &[OperatorDescriptor] = &[
    Binary(Add),
    Binary(Eq),
    Binary(NotEq),
    Binary(Less),
    Binary(LessEq),
    Binary(Greater),
    Binary(GreaterEq),
];
const BOOLEAN: &[OperatorDescriptor] = &[
    Binary(And),
    Binary(Or),
    Binary(Eq),
    Binary(NotEq),
    Unary(UnaryOp::Not),
];
const EQUALITY: &[OperatorDescriptor] = &[Binary(Eq), Binary(NotEq)];
const MUTABLE_SEQUENCE: &[OperatorDescriptor] = &[Index, IndexAssign, Iterate];
const LIFTED: &[OperatorDescriptor] = &[Extract, Unary(UnaryOp::UnsafeExtract)];
const OPTIONAL: &[OperatorDescriptor] = &[Extract, Unary(UnaryOp::UnsafeExtract), Iterate];

const TYPES: &[TypeCapabilities] = &[
    TypeCapabilities {
        name: "Bool",
        representation: Some(Representation::Bool),
        operations: BOOLEAN,
    },
    TypeCapabilities {
        name: "Char",
        representation: Some(Representation::Char),
        operations: EQUALITY,
    },
    TypeCapabilities {
        name: "Int",
        representation: Some(Representation::Int),
        operations: NUMERIC,
    },
    TypeCapabilities {
        name: "Float",
        representation: Some(Representation::Float),
        operations: NUMERIC,
    },
    TypeCapabilities {
        name: "Decimal",
        representation: Some(Representation::Decimal),
        operations: NUMERIC,
    },
    TypeCapabilities {
        name: "Str",
        representation: Some(Representation::Str),
        operations: STRING,
    },
    TypeCapabilities {
        name: "Vector",
        representation: None,
        operations: MUTABLE_SEQUENCE,
    },
    TypeCapabilities {
        name: "Array",
        representation: None,
        operations: MUTABLE_SEQUENCE,
    },
    TypeCapabilities {
        name: "LinkedList",
        representation: None,
        operations: MUTABLE_SEQUENCE,
    },
    TypeCapabilities {
        name: "Map",
        representation: None,
        operations: MUTABLE_SEQUENCE,
    },
    TypeCapabilities {
        name: "Set",
        representation: None,
        operations: &[Iterate],
    },
    TypeCapabilities {
        name: "IntRange",
        representation: None,
        operations: &[Iterate],
    },
    TypeCapabilities {
        name: "Option",
        representation: None,
        operations: OPTIONAL,
    },
    TypeCapabilities {
        name: "Result",
        representation: None,
        operations: LIFTED,
    },
    TypeCapabilities {
        name: "Either",
        representation: None,
        operations: LIFTED,
    },
    TypeCapabilities {
        name: "ReferenceId",
        representation: None,
        operations: EQUALITY,
    },
];

pub(super) fn capabilities(name: &str) -> Option<&'static TypeCapabilities> {
    TYPES.iter().find(|entry| entry.name == name)
}
