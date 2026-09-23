import Testing
@testable import SwiftGraphParserCore

private func parse(_ source: String) -> ParseResult {
    parseSource(source, fileName: "Test.swift")
}

@Test func nestedMembersCarryDocCommentsAttributesAndAccess() throws {
    let result = parse("""
    import Foundation
    @testable import App

    /// A store.
    @MainActor public final class Store {
        /// Shared instance.
        public static let shared = Store()
        public private(set) var count = 0
        init() {}
        deinit {}
        subscript(i: Int) -> Int { i }
        /// Loads.
        @discardableResult nonisolated public func load(id: Int) async throws -> Int { id }
        enum State { case idle, loading(Int) }
    }
    """)

    #expect(result.version == ParserProtocol.version)
    #expect(result.imports.map(\.name) == ["Foundation", "App"])
    #expect(result.imports[1].attributes == ["@testable"])
    #expect(result.imports[1].line == 2)

    let store = try #require(result.declarations.first)
    #expect(store.kind == "class")
    #expect(store.attributes == ["@MainActor"])
    #expect(store.accessLevel == "public")
    #expect(store.docComment == "/// A store.")

    let members = try #require(store.members)
    let byName = Dictionary(members.map { ($0.name, $0) }, uniquingKeysWith: { first, _ in first })
    #expect(byName["shared"]?.docComment == "/// Shared instance.")
    #expect(byName["count"]?.accessLevel == "public")
    #expect(byName["init"]?.kind == "initializer")
    #expect(byName["deinit"]?.kind == "deinitializer")
    #expect(byName["subscript"]?.kind == "subscript")

    let load = try #require(byName["load"])
    #expect(load.kind == "method")
    #expect(load.attributes == ["@discardableResult"])
    #expect(load.accessLevel == "public")
    #expect(load.docComment == "/// Loads.")

    let state = try #require(byName["State"])
    #expect(state.members?.map(\.name) == ["idle", "loading"])
    #expect(state.members?.first?.kind == "enumCase")
}

@Test func protocolRequirementsAndConditionalMembers() {
    let result = parse("""
    protocol Repo { associatedtype Item; func all() -> [Item] }
    struct S {
    #if DEBUG
        var debug = true
    #else
        var release = true
    #endif
    }
    """)
    #expect(result.declarations[0].members?.map(\.kind) == ["associatedType", "method"])
    #expect(result.declarations[1].members?.map(\.name) == ["debug", "release"])
}

@Test func topLevelConditionalCompilationIsFlattened() {
    let result = parse("""
    #if canImport(UIKit)
    import UIKit
    #if DEBUG
    func debugOnly() {}
    #endif
    #else
    import AppKit
    #endif
    """)
    #expect(result.imports.map(\.name) == ["UIKit", "AppKit"])
    #expect(result.declarations.map(\.name) == ["debugOnly"])
}

@Test(arguments: [
    ("func приветствие(имя: String) -> String { \"👋\" }", "приветствие", "function"),
    ("actor Cache {}", "Cache", "actor"),
    ("extension Array where Element == Int {}", "Array", "extension"),
    ("typealias ID = Int", "ID", "typeAlias"),
])
func topLevelDeclarationKinds(source: String, name: String, kind: String) throws {
    let decl = try #require(parse(source).declarations.first)
    #expect(decl.name == name)
    #expect(decl.kind == kind)
}
