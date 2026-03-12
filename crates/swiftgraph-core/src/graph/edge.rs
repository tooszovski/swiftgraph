use serde::{Deserialize, Serialize};

use super::node::{Location, SymbolId};

/// An edge in the code graph representing a relationship between two symbols.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    /// USR of the source symbol.
    pub source: SymbolId,
    /// USR of the target symbol.
    pub target: SymbolId,
    /// Kind of relationship.
    pub kind: EdgeKind,
    /// Where in the source code this relationship manifests.
    pub location: Option<Location>,
    /// Whether the relationship was synthesized by the compiler.
    pub is_implicit: bool,
}

/// Kind of relationship between two symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EdgeKind {
    // Calls
    /// Source calls target.
    Calls,

    // Type relationships
    /// struct/class X: Protocol
    ConformsTo,
    /// class X: BaseClass
    InheritsFrom,
    /// extension X { ... }
    ExtendsType,
    /// override func ...
    Overrides,
    /// Concrete method -> protocol requirement.
    ImplementsRequirement,

    // Dependencies
    /// Uses a symbol (read access).
    References,
    /// Modifies a symbol (write access).
    Mutates,
    /// import Module
    Imports,
    /// Module depends on module.
    DependsOn,

    // Containment
    /// Type contains method/property.
    Contains,

    // Data flow
    /// Function returns type.
    Returns,
    /// Type is a parameter of function.
    ParameterOf,
    /// Property has type.
    PropertyType,
}

impl EdgeKind {
    /// Returns the string label used in storage and MCP responses.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Calls => "calls",
            Self::ConformsTo => "conformsTo",
            Self::InheritsFrom => "inheritsFrom",
            Self::ExtendsType => "extendsType",
            Self::Overrides => "overrides",
            Self::ImplementsRequirement => "implementsRequirement",
            Self::References => "references",
            Self::Mutates => "mutates",
            Self::Imports => "imports",
            Self::DependsOn => "dependsOn",
            Self::Contains => "contains",
            Self::Returns => "returns",
            Self::ParameterOf => "parameterOf",
            Self::PropertyType => "propertyType",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_serialization_roundtrip() {
        let edge = GraphEdge {
            source: "s:5MyApp9AppRouterC".into(),
            target: "s:5MyApp11HTTPClientC".into(),
            kind: EdgeKind::Calls,
            location: Some(Location {
                file: "Sources/AppRouter.swift".into(),
                line: 25,
                column: 9,
                end_line: None,
                end_column: None,
            }),
            is_implicit: false,
        };

        let json = serde_json::to_string(&edge).unwrap();
        let deserialized: GraphEdge = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.kind, EdgeKind::Calls);
        assert!(!deserialized.is_implicit);
    }
}
