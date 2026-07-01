use crate::{
    ast::{ImportDecl, Program},
    backend::descriptors::{DescriptorOrigin, DescriptorType},
    resolver::{ImportedKind, ModuleGraph},
    source::Span,
};

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
    pub type_params: Vec<String>,
    pub source_path: String,
    pub span: Span,
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

    pub(crate) fn from_module_graph(graph: &ModuleGraph) -> Self {
        let symbols = graph
            .modules
            .values()
            .flat_map(|module| {
                module
                    .symbol_imports
                    .iter()
                    .filter_map(|(local_name, symbol)| {
                        let qualified_name = symbol.external_qualified_name.clone()?;
                        let kind = match symbol.kind {
                            ImportedKind::Type | ImportedKind::Interface | ImportedKind::Single => {
                                ExternalSymbolKind::Type
                            }
                            ImportedKind::Function => ExternalSymbolKind::Function,
                            ImportedKind::Value => ExternalSymbolKind::Value,
                        };
                        Some(ExternalSymbol {
                            local_name: local_name.clone(),
                            qualified_name,
                            kind,
                            type_params: external_type_params(graph, symbol),
                            source_path: module.display_path.clone(),
                            span: symbol.span,
                        })
                    })
            })
            .collect();
        Self { symbols }
    }

    pub fn type_descriptors(&self) -> Vec<DescriptorType> {
        self.symbols
            .iter()
            .filter(|symbol| symbol.kind == ExternalSymbolKind::Type)
            .map(|symbol| DescriptorType {
                name: symbol.local_name.clone(),
                kind: "class".to_string(),
                origin: DescriptorOrigin::Java {
                    qualified_name: symbol.qualified_name.clone(),
                },
                type_params: symbol.type_params.clone(),
                fields: Vec::new(),
                method_names: Vec::new(),
            })
            .collect()
    }
}

fn external_type_params(
    graph: &ModuleGraph,
    symbol: &crate::resolver::ImportedSymbol,
) -> Vec<String> {
    graph
        .modules
        .get(&symbol.module_path)
        .into_iter()
        .flat_map(|module| module.program.items.iter())
        .find_map(|item| match item {
            crate::ast::Item::Type(decl) if decl.name == symbol.original_name => Some(
                decl.type_params
                    .iter()
                    .map(|param| param.name.clone())
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default()
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
            type_params: Vec::new(),
            source_path: String::new(),
            span: symbol.span,
        })
        .collect()
}
