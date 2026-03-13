import SwiftSyntax

/// Visits Swift AST and extracts declarations.
final class DeclarationVisitor: SyntaxVisitor {
    var declarations: [Declaration] = []
    var imports: [String] = []
    private let converter: SourceLocationConverter

    init(converter: SourceLocationConverter) {
        self.converter = converter
        super.init(viewMode: .sourceAccurate)
    }

    // MARK: - Imports

    override func visit(_ node: ImportDeclSyntax) -> SyntaxVisitorContinueKind {
        let moduleName = node.path.map { $0.name.text }.joined(separator: ".")
        imports.append(moduleName)
        return .skipChildren
    }

    // MARK: - Classes

    override func visit(_ node: ClassDeclSyntax) -> SyntaxVisitorContinueKind {
        let decl = extractDeclaration(
            name: node.name.text,
            kind: "class",
            node: Syntax(node),
            attributes: node.attributes,
            modifiers: node.modifiers,
            signature: buildClassSignature(node),
            docComment: extractDocComment(node.leadingTrivia),
            members: extractMembers(node.memberBlock)
        )
        declarations.append(decl)
        return .skipChildren
    }

    // MARK: - Structs

    override func visit(_ node: StructDeclSyntax) -> SyntaxVisitorContinueKind {
        let decl = extractDeclaration(
            name: node.name.text,
            kind: "struct",
            node: Syntax(node),
            attributes: node.attributes,
            modifiers: node.modifiers,
            signature: buildStructSignature(node),
            docComment: extractDocComment(node.leadingTrivia),
            members: extractMembers(node.memberBlock)
        )
        declarations.append(decl)
        return .skipChildren
    }

    // MARK: - Enums

    override func visit(_ node: EnumDeclSyntax) -> SyntaxVisitorContinueKind {
        let decl = extractDeclaration(
            name: node.name.text,
            kind: "enum",
            node: Syntax(node),
            attributes: node.attributes,
            modifiers: node.modifiers,
            signature: "enum \(node.name.text)",
            docComment: extractDocComment(node.leadingTrivia),
            members: extractMembers(node.memberBlock)
        )
        declarations.append(decl)
        return .skipChildren
    }

    // MARK: - Protocols

    override func visit(_ node: ProtocolDeclSyntax) -> SyntaxVisitorContinueKind {
        let decl = extractDeclaration(
            name: node.name.text,
            kind: "protocol",
            node: Syntax(node),
            attributes: node.attributes,
            modifiers: node.modifiers,
            signature: "protocol \(node.name.text)",
            docComment: extractDocComment(node.leadingTrivia),
            members: extractMembers(node.memberBlock)
        )
        declarations.append(decl)
        return .skipChildren
    }

    // MARK: - Actors

    override func visit(_ node: ActorDeclSyntax) -> SyntaxVisitorContinueKind {
        let decl = extractDeclaration(
            name: node.name.text,
            kind: "actor",
            node: Syntax(node),
            attributes: node.attributes,
            modifiers: node.modifiers,
            signature: "actor \(node.name.text)",
            docComment: extractDocComment(node.leadingTrivia),
            members: extractMembers(node.memberBlock)
        )
        declarations.append(decl)
        return .skipChildren
    }

    // MARK: - Functions

    override func visit(_ node: FunctionDeclSyntax) -> SyntaxVisitorContinueKind {
        let sig = "func \(node.name.text)\(node.signature.description.trimmingCharacters(in: .whitespacesAndNewlines))"
        let decl = extractDeclaration(
            name: node.name.text,
            kind: "function",
            node: Syntax(node),
            attributes: node.attributes,
            modifiers: node.modifiers,
            signature: sig,
            docComment: extractDocComment(node.leadingTrivia),
            members: nil
        )
        declarations.append(decl)
        return .skipChildren
    }

    // MARK: - Extensions

    override func visit(_ node: ExtensionDeclSyntax) -> SyntaxVisitorContinueKind {
        let name = node.extendedType.description.trimmingCharacters(in: .whitespacesAndNewlines)
        let decl = extractDeclaration(
            name: name,
            kind: "extension",
            node: Syntax(node),
            attributes: node.attributes,
            modifiers: node.modifiers,
            signature: "extension \(name)",
            docComment: extractDocComment(node.leadingTrivia),
            members: extractMembers(node.memberBlock)
        )
        declarations.append(decl)
        return .skipChildren
    }

    // MARK: - Type aliases

    override func visit(_ node: TypeAliasDeclSyntax) -> SyntaxVisitorContinueKind {
        let decl = extractDeclaration(
            name: node.name.text,
            kind: "typeAlias",
            node: Syntax(node),
            attributes: node.attributes,
            modifiers: node.modifiers,
            signature: "typealias \(node.name.text) = \(node.initializer.value.description.trimmingCharacters(in: .whitespacesAndNewlines))",
            docComment: extractDocComment(node.leadingTrivia),
            members: nil
        )
        declarations.append(decl)
        return .skipChildren
    }

    // MARK: - Helpers

    private func extractDeclaration(
        name: String,
        kind: String,
        node: Syntax,
        attributes: AttributeListSyntax,
        modifiers: DeclModifierListSyntax,
        signature: String?,
        docComment: String?,
        members: [Declaration]?
    ) -> Declaration {
        let startLoc = converter.location(for: node.positionAfterSkippingLeadingTrivia)
        let endLoc = converter.location(for: node.endPositionBeforeTrailingTrivia)

        let attrs = attributes.compactMap { attr -> String? in
            return "@\(attr.description.trimmingCharacters(in: .whitespacesAndNewlines).dropFirst())"
        }

        let accessLevel = extractAccessLevel(modifiers)

        return Declaration(
            name: name,
            kind: kind,
            line: startLoc.line,
            endLine: endLoc.line,
            attributes: attrs,
            accessLevel: accessLevel,
            signature: signature,
            docComment: docComment,
            members: members
        )
    }

    private func extractAccessLevel(_ modifiers: DeclModifierListSyntax) -> String? {
        for modifier in modifiers {
            let text = modifier.name.text
            if ["public", "private", "fileprivate", "internal", "open", "package"].contains(text) {
                return text
            }
        }
        return nil
    }

    private func extractDocComment(_ trivia: Trivia) -> String? {
        var lines: [String] = []
        for piece in trivia {
            switch piece {
            case .docLineComment(let text):
                lines.append(text)
            case .docBlockComment(let text):
                lines.append(text)
            default:
                break
            }
        }
        return lines.isEmpty ? nil : lines.joined(separator: "\n")
    }

    private func extractMembers(_ memberBlock: MemberBlockSyntax) -> [Declaration] {
        let visitor = MemberVisitor(converter: converter)
        visitor.walk(memberBlock)
        return visitor.members
    }

    private func buildClassSignature(_ node: ClassDeclSyntax) -> String {
        var sig = "class \(node.name.text)"
        if let inheritance = node.inheritanceClause {
            sig += inheritance.description.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return sig
    }

    private func buildStructSignature(_ node: StructDeclSyntax) -> String {
        var sig = "struct \(node.name.text)"
        if let inheritance = node.inheritanceClause {
            sig += inheritance.description.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return sig
    }
}

/// Extracts member declarations from a member block (methods, properties).
private final class MemberVisitor: SyntaxVisitor {
    var members: [Declaration] = []
    private let converter: SourceLocationConverter

    init(converter: SourceLocationConverter) {
        self.converter = converter
        super.init(viewMode: .sourceAccurate)
    }

    override func visit(_ node: FunctionDeclSyntax) -> SyntaxVisitorContinueKind {
        let sig = "func \(node.name.text)\(node.signature.description.trimmingCharacters(in: .whitespacesAndNewlines))"
        let startLoc = converter.location(for: node.positionAfterSkippingLeadingTrivia)
        let endLoc = converter.location(for: node.endPositionBeforeTrailingTrivia)

        let attrs = node.attributes.compactMap { attr -> String? in
            return "@\(attr.description.trimmingCharacters(in: .whitespacesAndNewlines).dropFirst())"
        }

        let accessLevel = extractAccessLevel(node.modifiers)

        members.append(Declaration(
            name: node.name.text,
            kind: "method",
            line: startLoc.line,
            endLine: endLoc.line,
            attributes: attrs,
            accessLevel: accessLevel,
            signature: sig,
            docComment: nil,
            members: nil
        ))
        return .skipChildren
    }

    override func visit(_ node: VariableDeclSyntax) -> SyntaxVisitorContinueKind {
        let startLoc = converter.location(for: node.positionAfterSkippingLeadingTrivia)

        let attrs = node.attributes.compactMap { attr -> String? in
            return "@\(attr.description.trimmingCharacters(in: .whitespacesAndNewlines).dropFirst())"
        }

        for binding in node.bindings {
            if let pattern = binding.pattern.as(IdentifierPatternSyntax.self) {
                let accessLevel = extractAccessLevel(node.modifiers)
                members.append(Declaration(
                    name: pattern.identifier.text,
                    kind: "property",
                    line: startLoc.line,
                    endLine: nil,
                    attributes: attrs,
                    accessLevel: accessLevel,
                    signature: node.description.trimmingCharacters(in: .whitespacesAndNewlines),
                    docComment: nil,
                    members: nil
                ))
            }
        }
        return .skipChildren
    }

    private func extractAccessLevel(_ modifiers: DeclModifierListSyntax) -> String? {
        for modifier in modifiers {
            let text = modifier.name.text
            if ["public", "private", "fileprivate", "internal", "open", "package"].contains(text) {
                return text
            }
        }
        return nil
    }
}
