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
    let (declarations, imports) = extractor.statements(tree.statements)
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

    /// Declarations and imports of a statement list (file level or a `#if`
    /// clause), descending into every clause of nested `#if` blocks.
    func statements(_ items: CodeBlockItemListSyntax) -> ([Declaration], [ImportDecl]) {
        var declarations: [Declaration] = []
        var imports: [ImportDecl] = []
        for item in items {
            guard let decl = item.item.as(DeclSyntax.self) else { continue }
            if let imp = decl.as(ImportDeclSyntax.self) {
                imports.append(importDecl(imp))
            } else if let ifConfig = decl.as(IfConfigDeclSyntax.self) {
                for clause in ifConfig.clauses {
                    guard let nested = clause.elements?.as(CodeBlockItemListSyntax.self) else { continue }
                    let (d, i) = statements(nested)
                    declarations += d
                    imports += i
                }
            } else {
                declarations += self.declarations(for: decl, isMember: false)
            }
        }
        return (declarations, imports)
    }

    func members(_ block: MemberBlockSyntax) -> [Declaration] {
        members(block.members)
    }

    private func members(_ list: MemberBlockItemListSyntax) -> [Declaration] {
        list.flatMap { declarations(for: $0.decl, isMember: true) }
    }

    /// One syntax node can yield several declarations (`var a, b`, `case x, y`).
    func declarations(for decl: DeclSyntax, isMember: Bool) -> [Declaration] {
        switch decl.as(DeclSyntaxEnum.self) {
        case .classDecl(let n):
            [nominalType(n, keyword: "class")]
        case .structDecl(let n):
            [nominalType(n, keyword: "struct")]
        case .enumDecl(let n):
            [nominalType(n, keyword: "enum")]
        case .protocolDecl(let n):
            [nominalType(n, keyword: "protocol")]
        case .actorDecl(let n):
            [nominalType(n, keyword: "actor")]
        case .extensionDecl(let n):
            [make(n, name: n.extendedType.trimmedDescription, kind: "extension", attrs: n.attributes,
                  mods: n.modifiers,
                  signature: "extension \(n.extendedType.trimmedDescription)\(inheritance(n.inheritanceClause))",
                  members: members(n.memberBlock))]
        case .functionDecl(let n):
            [make(n, name: n.name.text, kind: isMember ? "method" : "function",
                  attrs: n.attributes, mods: n.modifiers,
                  signature: "func \(n.name.text)\(n.genericParameterClause?.trimmedDescription ?? "")\(n.signature.trimmedDescription)",
                  members: nil)]
        case .initializerDecl(let n):
            [make(n, name: "init", kind: "initializer", attrs: n.attributes, mods: n.modifiers,
                  signature: "init\(n.optionalMark?.text ?? "")\(n.signature.trimmedDescription)",
                  members: nil)]
        case .deinitializerDecl(let n):
            [make(n, name: "deinit", kind: "deinitializer", attrs: n.attributes, mods: n.modifiers,
                  signature: "deinit", members: nil)]
        case .subscriptDecl(let n):
            [make(n, name: "subscript", kind: "subscript", attrs: n.attributes, mods: n.modifiers,
                  signature: "subscript\(n.parameterClause.trimmedDescription) \(n.returnClause.trimmedDescription)",
                  members: nil)]
        case .variableDecl(let n):
            n.bindings.compactMap { binding in
                guard let id = binding.pattern.as(IdentifierPatternSyntax.self) else { return nil }
                let type = binding.typeAnnotation?.trimmedDescription ?? ""
                return make(n, name: id.identifier.text, kind: "property", attrs: n.attributes,
                            mods: n.modifiers,
                            signature: "\(n.bindingSpecifier.text) \(id.identifier.text)\(type)",
                            members: nil)
            }
        case .enumCaseDecl(let n):
            n.elements.map { element in
                make(n, name: element.name.text, kind: "enumCase", attrs: n.attributes, mods: n.modifiers,
                     signature: "case \(element.trimmedDescription)", members: nil)
            }
        case .associatedTypeDecl(let n):
            [make(n, name: n.name.text, kind: "associatedType", attrs: n.attributes,
                  mods: n.modifiers, signature: "associatedtype \(n.name.text)", members: nil)]
        case .typeAliasDecl(let n):
            [make(n, name: n.name.text, kind: "typeAlias", attrs: n.attributes, mods: n.modifiers,
                  signature: "typealias \(n.name.text) = \(n.initializer.value.trimmedDescription)",
                  members: nil)]
        case .macroDecl(let n):
            [make(n, name: n.name.text, kind: "macro", attrs: n.attributes, mods: n.modifiers,
                  signature: "macro \(n.name.text)\(n.signature.trimmedDescription)", members: nil)]
        case .ifConfigDecl(let n):
            // Members inside #if/#else: report all clauses.
            n.clauses.flatMap { clause -> [Declaration] in
                guard let list = clause.elements?.as(MemberBlockItemListSyntax.self) else { return [] }
                return members(list)
            }
        default:
            []
        }
    }

    /// `class`/`struct`/`enum`/`protocol`/`actor`: named declaration groups
    /// that differ only by keyword.
    private func nominalType<T: DeclGroupSyntax & NamedDeclSyntax>(_ n: T, keyword: String) -> Declaration {
        make(n, name: n.name.text, kind: keyword, attrs: n.attributes, mods: n.modifiers,
             signature: "\(keyword) \(n.name.text)\(inheritance(n.inheritanceClause))",
             members: members(n.memberBlock))
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
        let lines = trivia.compactMap { piece -> String? in
            switch piece {
            case .docLineComment(let text), .docBlockComment(let text): text
            default: nil
            }
        }
        return lines.isEmpty ? nil : lines.joined(separator: "\n")
    }
}
