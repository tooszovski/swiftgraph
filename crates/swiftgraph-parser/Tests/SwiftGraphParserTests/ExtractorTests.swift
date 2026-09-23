import XCTest
@testable import SwiftGraphParserCore

final class ExtractorTests: XCTestCase {
    private func parse(_ source: String) -> ParseResult {
        parseSource(source, fileName: "Test.swift")
    }

    func testNestedMembersWithDocCommentsAndAttributes() throws {
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

        XCTAssertEqual(result.version, ParserProtocol.version)
        XCTAssertEqual(result.imports.map(\.name), ["Foundation", "App"])
        XCTAssertEqual(result.imports[1].attributes, ["@testable"])
        XCTAssertEqual(result.imports[1].line, 2)

        let store = try XCTUnwrap(result.declarations.first)
        XCTAssertEqual(store.kind, "class")
        XCTAssertEqual(store.attributes, ["@MainActor"])
        XCTAssertEqual(store.accessLevel, "public")
        XCTAssertEqual(store.docComment, "/// A store.")

        let members = try XCTUnwrap(store.members)
        let byName = Dictionary(members.map { ($0.name, $0) }, uniquingKeysWith: { a, _ in a })
        XCTAssertEqual(byName["shared"]?.docComment, "/// Shared instance.")
        XCTAssertEqual(byName["count"]?.accessLevel, "public")
        XCTAssertEqual(byName["init"]?.kind, "initializer")
        XCTAssertEqual(byName["deinit"]?.kind, "deinitializer")
        XCTAssertEqual(byName["subscript"]?.kind, "subscript")
        let load = try XCTUnwrap(byName["load"])
        XCTAssertEqual(load.kind, "method")
        XCTAssertEqual(load.attributes, ["@discardableResult"])
        XCTAssertEqual(load.accessLevel, "public")
        XCTAssertEqual(load.docComment, "/// Loads.")

        let state = try XCTUnwrap(byName["State"])
        XCTAssertEqual(state.members?.map(\.name), ["idle", "loading"])
        XCTAssertEqual(state.members?.first?.kind, "enumCase")
    }

    func testProtocolAndConditionalMembers() {
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
        let repo = result.declarations[0]
        XCTAssertEqual(repo.members?.map(\.kind), ["associatedType", "method"])
        XCTAssertEqual(result.declarations[1].members?.map(\.name), ["debug", "release"])
    }

    func testNonASCIIIsPreserved() {
        let result = parse("func приветствие(имя: String) -> String { \"👋\" }")
        XCTAssertEqual(result.declarations.first?.name, "приветствие")
    }
}
