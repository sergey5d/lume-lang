#[derive(Debug, Clone, Default)]
pub struct Program {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Function(FunctionDecl),
    Class(TypeDecl),
    Record(TypeDecl),
    Object(TypeDecl),
    Interface(TypeDecl),
    Enum(TypeDecl),
}

#[derive(Debug, Clone)]
pub struct FunctionDecl {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct TypeDecl {
    pub name: String,
}
