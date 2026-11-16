use super::super::schema::*;
use crate::ast::{Item, TypeKind, TypeMember};

pub(super) fn items() -> Vec<Item> {
    const PARAMETERS: [&str; 10] = ["A", "B", "C", "D", "E", "F", "G", "H", "I", "J"];
    (2..=10)
        .map(|arity| {
            Item::Type(declaration(
                &format!("Tuple{arity}"),
                TypeKind::Class,
                PARAMETERS[..arity]
                    .iter()
                    .map(|name| type_param(name))
                    .collect(),
                vec![],
                PARAMETERS[..arity]
                    .iter()
                    .enumerate()
                    .map(|(index, name)| {
                        TypeMember::Field(field(&format!("_{}", index + 1), ty(name)))
                    })
                    .collect(),
            ))
        })
        .collect()
}
