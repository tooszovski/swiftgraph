/// JSON output protocol for the SwiftGraph parser.
///
/// Protocol 2:
/// - `--version` prints one `ParserInfo` line;
/// - `--stdin` reads one file path per line and prints one line per file, in
///   input order: a `ParseResult`, or a `ParseFailure` for unreadable files;
/// - `<file>` prints a single `ParseResult`.
/// Output is compact JSON (one object per line).

/// Protocol constants shared with the Rust side (`swift_syntax::PROTOCOL_VERSION`).
public enum ParserProtocol {
    /// Bumped on any incompatible change of the JSON shapes below.
    public static let version = 2
    /// Informational parser version.
    public static let parserVersion = "0.5.1"
}

/// Reply to `--version`.
public struct ParserInfo: Codable, Sendable {
    public let name: String
    public let version: String
    public let `protocol`: Int

    public static let current = ParserInfo(
        name: "swiftgraph-parser",
        version: ParserProtocol.parserVersion,
        protocol: ParserProtocol.version
    )
}

/// Declarations and imports of one file.
public struct ParseResult: Codable, Sendable {
    public let version: Int
    public let file: String
    public let declarations: [Declaration]
    public let imports: [ImportDecl]
}

/// A file that could not be parsed (e.g. unreadable).
public struct ParseFailure: Codable, Sendable {
    public let file: String
    public let error: String

    public init(file: String, error: String) {
        self.file = file
        self.error = error
    }
}

/// An `import` statement.
public struct ImportDecl: Codable, Sendable, Equatable {
    /// Module path, e.g. `Foundation` or `UIKit.UIView`.
    public let name: String
    public let line: Int
    /// Attributes such as `@testable` or `@_exported`.
    public let attributes: [String]
    /// Import kind for `import struct Foo.Bar` style imports.
    public let kind: String?
}

/// A declaration, with nested members for type-like declarations.
public struct Declaration: Codable, Sendable {
    public let name: String
    /// class, struct, enum, protocol, actor, extension, function, method,
    /// initializer, deinitializer, subscript, property, enumCase,
    /// associatedType, typeAlias, macro
    public let kind: String
    public let line: Int
    public let endLine: Int?
    public let attributes: [String]
    public let accessLevel: String?
    public let signature: String?
    public let docComment: String?
    public let members: [Declaration]?
}
