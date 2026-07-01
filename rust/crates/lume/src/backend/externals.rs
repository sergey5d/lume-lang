use std::collections::HashMap;

use crate::ast::{ImportDecl, Program};

/// Target-provided symbols loaded from outside Lume source files.
#[derive(Debug, Clone, Default)]
pub struct ExternalDescriptors {
    pub symbols: Vec<ExternalSymbol>,
}

#[derive(Debug, Clone)]
pub struct ExternalSymbol {
    pub local_name: String,
    pub qualified_name: String,
    pub kind: ExternalSymbolKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalSymbolKind {
    Type,
    Function,
    Value,
}

impl ExternalDescriptors {
    pub fn from_program(program: &Program) -> Self {
        Self::from_programs([program])
    }

    pub fn from_programs<'a>(programs: impl IntoIterator<Item = &'a Program>) -> Self {
        let symbols = programs
            .into_iter()
            .flat_map(|program| program.imports.iter())
            .filter(|import| import.path.starts_with("java/"))
            .flat_map(java_import_symbols)
            .collect();
        Self { symbols }
    }

    pub fn type_name_map(&self) -> HashMap<String, String> {
        self.symbols
            .iter()
            .filter(|symbol| symbol.kind == ExternalSymbolKind::Type)
            .map(|symbol| (symbol.local_name.clone(), symbol.qualified_name.clone()))
            .collect()
    }
}

fn java_import_symbols(import: &ImportDecl) -> Vec<ExternalSymbol> {
    if !import.path.starts_with("java/") {
        return Vec::new();
    }
    let package = import.path.replace('/', ".");
    import
        .symbols
        .iter()
        .map(|symbol| ExternalSymbol {
            local_name: symbol.alias.clone().unwrap_or_else(|| symbol.name.clone()),
            qualified_name: format!("{package}.{}", symbol.name),
            kind: ExternalSymbolKind::Type,
        })
        .collect()
}
