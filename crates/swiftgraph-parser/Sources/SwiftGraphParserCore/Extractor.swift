import Foundation
import SwiftParser
import SwiftSyntax

/// Parse one file from disk.
public func parseFile(atPath path: String) throws -> ParseResult {
    let source = try String(contentsOfFile: path, encoding: .utf8)
    return parseSource(source, fileName: path)
}

/// Parse Swift source text.
public func parseSource(_ source: String, fileName: String) -> ParseResult {
    let tree = Parser.parse(source: source)
    let extractor = DeclarationExtractor(
        converter: SourceLocationConverter(fileName: fileName, tree: tree)
    )
    var imports: [ImportDecl] = []
    var declarations: [Declaration] = []
    for item in tree.statements {
        guard let decl = item.item.as(DeclSyntax.self) else { continue }
        if let imp = decl.as(ImportDeclSyntax.self) {
            imports.append(extractor.importDecl(imp))
        } else if let ifConfig = decl.as(IfConfigDeclSyntax.self) {
            let (decls, nestedImports) = extractor.topLevel(ifConfig)
            declarations += decls
            imports += nestedImports
        } else {
            declarations += extractor.declarations(for: decl, isMember: false)
        }
    }
    return ParseResult(
        version: ParserProtocol.version,
        file: fileName,
        declarations: declarations,
        imports: imports
    )
}

/// Recursively converts declaration syntax into `Declaration` values.
struct DeclarationExtractor {
    let converter: SourceLocationConverter

    func importDecl(_ node: ImportDeclSyntax) -> ImportDecl {
        ImportDecl(
            name: node.path.map { $0.name.text }.joined(separator: "."),
            line: line(of: Syntax(node)),
            attributes: attributes(node.attributes),
            kind: node.importKindSpecifier?.text
        )
    }

    /// Declarations and imports inside a top-level `#if` block (all clauses).
    func topLevel(_ node: IfConfigDeclSyntax) -> ([Declaration], [ImportDecl]) {
        var decls: [Declaration] = []
        var imports: [ImportDecl] = []
        for clause in node.clauses {
            if let items = clause.elements?.as(CodeBlockItemListSyntax.self) {
                for item in items {
                    guard let decl = item.item.as(DeclSyntax.self) else { continue }
                    if let imp = decl.as(ImportDeclSyntax.self) {
                        imports.append(importDecl(imp))
                    } else {
                        decls += declarations(for: decl, isMember: false)
                    }
                }
            }
        }
        return (decls, imports)
    }

    func members(_ block: MemberBlockSyntax) -> [Declaration] {
        members(block.members)
    }

    private func members(_ list: MemberBlockItemListSyntax) -> [Declaration] {
        list.flatMap { declarations(for: $0.decl, isMember: true) }
    }

    /// One syntax node can yield several declarations (`var a, b`, `case x, y`).
    func declarations(for decl: DeclSyntax, isMember: Bool) -> [Declaration] {
        if let n = decl.as(ClassDeclSyntax.self) {
            return [make(n, name: n.name.text, kind: "class", attrs: n.attributes, mods: n.modifiers,
                         signature: "class \(n.name.text)\(inheritance(n.inheritanceClause))",
                         members: members(n.memberBlock))]
        }
        if let n = decl.as(StructDeclSyntax.self) {
            return [make(n, name: n.name.text, kind: "struct", attrs: n.attributes, mods: n.modifiers,
                         signature: "struct \(n.name.text)\(inheritance(n.inheritanceClause))",
                         members: members(n.memberBlock))]
        }
        if let n = decl.as(EnumDeclSyntax.self) {
            return [make(n, name: n.name.text, kind: "enum", attrs: n.attributes, mods: n.modifiers,
                         signature: "enum \(n.name.text)\(inheritance(n.inheritanceClause))",
                         members: members(n.memberBlock))]
        }
        if let n = decl.as(ProtocolDeclSyntax.self) {
            return [make(n, name: n.name.text, kind: "protocol", attrs: n.attributes, mods: n.modifiers,
                         signature: "protocol \(n.name.text)\(inheritance(n.inheritanceClause))",
                         members: members(n.memberBlock))]
        }
        if let n = decl.as(ActorDeclSyntax.self) {
            return [make(n, name: n.name.text, kind: "actor", attrs: n.attributes, mods: n.modifiers,
                         signature: "actor \(n.name.text)\(inheritance(n.inheritanceClause))",
                         members: members(n.memberBlock))]
        }
        if let n = decl.as(ExtensionDeclSyntax.self) {
            let name = n.extendedType.trimmedDescription
            return [make(n, name: name, kind: "extension", attrs: n.attributes, mods: n.modifiers,
                         signature: "extension \(name)\(inheritance(n.inheritanceClause))",
                         members: members(n.memberBlock))]
        }
        if let n = decl.as(FunctionDeclSyntax.self) {
            return [make(n, name: n.name.text, kind: isMember ? "method" : "function",
                         attrs: n.attributes, mods: n.modifiers,
                         signature: "func \(n.name.text)\(n.genericParameterClause?.trimmedDescription ?? "")\(n.signature.trimmedDescription)",
                         members: nil)]
        }
        if let n = decl.as(InitializerDeclSyntax.self) {
            let optional = n.optionalMark?.text ?? ""
            return [make(n, name: "init", kind: "initializer", attrs: n.attributes, mods: n.modifiers,
                         signature: "init\(optional)\(n.signature.trimmedDescription)", members: nil)]
        }
        if let n = decl.as(DeinitializerDeclSyntax.self) {
            return [make(n, name: "deinit", kind: "deinitializer", attrs: n.attributes, mods: n.modifiers,
                         signature: "deinit", members: nil)]
        }
        if let n = decl.as(SubscriptDeclSyntax.self) {
            return [make(n, name: "subscript", kind: "subscript", attrs: n.attributes, mods: n.modifiers,
                         signature: "subscript\(n.parameterClause.trimmedDescription) \(n.returnClause.trimmedDescription)",
                         members: nil)]
        }
        if let n = decl.as(VariableDeclSyntax.self) {
            return n.bindings.compactMap { binding -> Declaration? in
                guard let id = binding.pattern.as(IdentifierPatternSyntax.self) else { return nil }
                let type = binding.typeAnnotation?.trimmedDescription ?? ""
                return make(n, name: id.identifier.text, kind: "property", attrs: n.attributes,
                            mods: n.modifiers,
                            signature: "\(n.bindingSpecifier.text) \(id.identifier.text)\(type)",
                            members: nil)
            }
        }
        if let n = decl.as(EnumCaseDeclSyntax.self) {
            return n.elements.map { element in
                make(n, name: element.name.text, kind: "enumCase", attrs: n.attributes, mods: n.modifiers,
                     signature: "case \(element.trimmedDescription)", members: nil)
            }
        }
        if let n = decl.as(AssociatedTypeDeclSyntax.self) {
            return [make(n, name: n.name.text, kind: "associatedType", attrs: n.attributes,
                         mods: n.modifiers, signature: "associatedtype \(n.name.text)", members: nil)]
        }
        if let n = decl.as(TypeAliasDeclSyntax.self) {
            return [make(n, name: n.name.text, kind: "typeAlias", attrs: n.attributes, mods: n.modifiers,
                         signature: "typealias \(n.name.text) = \(n.initializer.value.trimmedDescription)",
                         members: nil)]
        }
        if let n = decl.as(MacroDeclSyntax.self) {
            return [make(n, name: n.name.text, kind: "macro", attrs: n.attributes, mods: n.modifiers,
                         signature: "macro \(n.name.text)\(n.signature.trimmedDescription)", members: nil)]
        }
        if let n = decl.as(IfConfigDeclSyntax.self) {
            // Members inside #if/#else: report all clauses.
            return n.clauses.flatMap { clause -> [Declaration] in
                guard let list = clause.elements?.as(MemberBlockItemListSyntax.self) else { return [] }
                return members(list)
            }
        }
        return []
    }

    private func make(
        _ node: some SyntaxProtocol,
        name: String,
        kind: String,
        attrs: AttributeListSyntax,
        mods: DeclModifierListSyntax,
        signature: String?,
        members: [Declaration]?
    ) -> Declaration {
        let syntax = Syntax(node)
        return Declaration(
            name: name,
            kind: kind,
            line: line(of: syntax),
            endLine: converter.location(for: syntax.endPositionBeforeTrailingTrivia).line,
            attributes: attributes(attrs),
            accessLevel: accessLevel(mods),
            signature: signature,
            docComment: docComment(syntax.leadingTrivia),
            members: members
        )
    }

    private func line(of node: Syntax) -> Int {
        converter.location(for: node.positionAfterSkippingLeadingTrivia).line
    }

    private func inheritance(_ clause: InheritanceClauseSyntax?) -> String {
        clause?.trimmedDescription ?? ""
    }

    private func attributes(_ list: AttributeListSyntax) -> [String] {
        list.map { $0.trimmedDescription }
    }

    private func accessLevel(_ modifiers: DeclModifierListSyntax) -> String? {
        for modifier in modifiers {
            // `private(set)` restricts only the setter
            if modifier.detail != nil { continue }
            let text = modifier.name.text
            if ["open", "public", "package", "internal", "fileprivate", "private"].contains(text) {
                return text
            }
        }
        return nil
    }

    private func docComment(_ trivia: Trivia) -> String? {
        var lines: [String] = []
        for piece in trivia {
            switch piece {
            case .docLineComment(let text), .docBlockComment(let text):
                lines.append(text)
            default:
                break
            }
        }
        return lines.isEmpty ? nil : lines.joined(separator: "\n")
    }
}
