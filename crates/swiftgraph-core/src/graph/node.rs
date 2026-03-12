use serde::{Deserialize, Serialize};

/// Unified Symbol Resolution ID or synthetic identifier.
pub type SymbolId = String;

/// A node in the code graph representing a Swift symbol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    /// USR (Unified Symbol Resolution) or synthetic ID.
    pub id: SymbolId,
    /// Short name, e.g. "AppRouter".
    pub name: String,
    /// Fully qualified name, e.g. "MyApp.AppRouter".
    pub qualified_name: String,
    /// Symbol kind.
    pub kind: SymbolKind,
    /// Optional sub-kind for finer classification.
    pub sub_kind: Option<SymbolSubKind>,
    /// Source location.
    pub location: Location,
    /// Full signature, e.g. "func perform(request: IHTTPRequest) async throws -> Data".
    pub signature: Option<String>,
    /// Attributes like @MainActor, @Published.
    pub attributes: Vec<String>,
    /// Access level.
    pub access_level: AccessLevel,
    /// USR of the containing symbol (parent type, file, etc.).
    pub container_usr: Option<SymbolId>,
    /// Documentation comment.
    pub doc_comment: Option<String>,
    /// Computed metrics.
    pub metrics: Option<NodeMetrics>,
}

/// Source location of a symbol or edge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub end_line: Option<u32>,
    pub end_column: Option<u32>,
}

/// Primary classification of a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SymbolKind {
    Class,
    Struct,
    Enum,
    Protocol,
    Method,
    Property,
    Function,
    TypeAlias,
    Extension,
    EnumCase,
    Macro,
    AssociatedType,
    Module,
    Import,
    File,
}

impl SymbolKind {
    /// Returns the string label used in storage and MCP responses.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Protocol => "protocol",
            Self::Method => "method",
            Self::Property => "property",
            Self::Function => "function",
            Self::TypeAlias => "typeAlias",
            Self::Extension => "extension",
            Self::EnumCase => "enumCase",
            Self::Macro => "macro",
            Self::AssociatedType => "associatedType",
            Self::Module => "module",
            Self::Import => "import",
            Self::File => "file",
        }
    }
}

/// Sub-kind for finer symbol classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SymbolSubKind {
    Getter,
    Setter,
    Subscript,
    Initializer,
    Deinitializer,
}

/// Access level of a symbol.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccessLevel {
    Open,
    Public,
    Package,
    #[default]
    Internal,
    FilePrivate,
    Private,
}

/// Computed metrics for a node.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeMetrics {
    pub lines: Option<u32>,
    pub complexity: Option<u32>,
    pub parameter_count: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_kind_as_str() {
        assert_eq!(SymbolKind::Class.as_str(), "class");
        assert_eq!(SymbolKind::AssociatedType.as_str(), "associatedType");
    }

    #[test]
    fn node_serialization_roundtrip() {
        let node = GraphNode {
            id: "s:5MyApp9AppRouterC".into(),
            name: "AppRouter".into(),
            qualified_name: "MyApp.AppRouter".into(),
            kind: SymbolKind::Class,
            sub_kind: None,
            location: Location {
                file: "Sources/AppRouter.swift".into(),
                line: 5,
                column: 1,
                end_line: Some(50),
                end_column: Some(1),
            },
            signature: None,
            attributes: vec!["@MainActor".into()],
            access_level: AccessLevel::Internal,
            container_usr: None,
            doc_comment: None,
            metrics: None,
        };

        let json = serde_json::to_string(&node).unwrap();
        let deserialized: GraphNode = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "AppRouter");
        assert_eq!(deserialized.kind, SymbolKind::Class);
    }
}
