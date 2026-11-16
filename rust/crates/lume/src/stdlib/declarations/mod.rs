//! Rust-authored public contracts for the native standard library.

use super::schema::*;
use crate::ast;

mod array_type;
mod char_type;
mod comparable_type;
mod decimal_type;
mod either_type;
mod eq_type;
mod file_type;
mod float_type;
mod int_type;
mod invalid_index_type;
mod iterable_type;
mod json;
mod linked_list_type;
mod map_type;
mod math_type;
mod option_type;
mod os_type;
mod predef;
mod primitives;
mod printer_type;
mod range_type;
mod reference_id_type;
mod regex;
mod result_type;
mod rounding_type;
mod runtime_type;
mod set_type;
mod str_type;
mod tuple_type;
mod vector_type;

pub(super) fn program() -> ast::Program {
    super::schema::program(
        [
            primitives::items(),
            eq_type::items(),
            comparable_type::items(),
            iterable_type::items(),
            str_type::items(),
            char_type::items(),
            int_type::items(),
            float_type::items(),
            decimal_type::items(),
            rounding_type::items(),
            vector_type::items(),
            array_type::items(),
            linked_list_type::items(),
            set_type::items(),
            map_type::items(),
            invalid_index_type::items(),
            option_type::items(),
            result_type::items(),
            either_type::items(),
            printer_type::items(),
            os_type::items(),
            math_type::items(),
            file_type::items(),
            reference_id_type::items(),
            runtime_type::items(),
            tuple_type::items(),
            range_type::items(),
            predef::items(),
        ]
        .into_iter()
        .flatten()
        .collect(),
    )
}

pub(super) fn module_program(path: &str) -> Option<ast::Program> {
    let items = match path {
        "lume/json" => json::items(),
        "lume/regex" => regex::items(),
        _ => return None,
    };
    let mut program = super::schema::program(items);
    program.module = Some(ast::ModuleDecl {
        name: path.into(),
        span: span(),
    });
    Some(program)
}

#[cfg(test)]
mod tests;
