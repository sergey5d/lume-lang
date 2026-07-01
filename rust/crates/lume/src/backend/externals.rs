/// Target-provided symbols loaded from outside Lume source files.
#[derive(Debug, Clone, Default)]
pub struct ExternalDescriptors {
    pub symbols: Vec<ExternalSymbol>,
}

#[derive(Debug, Clone)]
pub struct ExternalSymbol {
    pub qualified_name: String,
    pub kind: ExternalSymbolKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalSymbolKind {
    Type,
    Function,
    Value,
}
