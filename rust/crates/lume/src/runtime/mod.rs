//! Runtime metadata derived from lowered IR.
//!
//! The interpreter executes IR, but it benefits from a denser runtime view of
//! user-defined types: fields, enum cases, interface bounds, and method slots.
//! This module builds that execution-oriented metadata once from `ir::Program`
//! so the interpreter does not have to rediscover type structure through
//! repeated scans of the IR tables.

mod builtins;
mod types;

pub use types::{
    RuntimeEnumCase,
    RuntimeEnumCaseId,
    RuntimeField,
    RuntimeFieldSlot,
    RuntimeMethod,
    RuntimeMethodSlot,
    RuntimeProgram,
    RuntimeType,
    RuntimeTypeId,
};

pub(crate) use types::RuntimeMethodTarget;
