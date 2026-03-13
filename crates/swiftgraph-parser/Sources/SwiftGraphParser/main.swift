/// SwiftGraph Parser — extracts declarations from Swift files using swift-syntax.
///
/// Usage: swiftgraph-parser <file.swift>
/// Output: JSON (ParseResult) to stdout

import Foundation
import SwiftParser
import SwiftSyntax

let args = CommandLine.arguments
guard args.count >= 2 else {
    FileHandle.standardError.write(Data("Usage: swiftgraph-parser <file.swift>\n".utf8))
    exit(1)
}

let filePath = args[1]
let source: String
do {
    source = try String(contentsOfFile: filePath, encoding: .utf8)
} catch {
    FileHandle.standardError.write(Data("Error reading file: \(error)\n".utf8))
    exit(1)
}

let tree = Parser.parse(source: source)
let converter = SourceLocationConverter(fileName: filePath, tree: tree)

let visitor = DeclarationVisitor(converter: converter)
visitor.walk(tree)

let result = ParseResult(
    version: 1,
    file: filePath,
    declarations: visitor.declarations,
    imports: visitor.imports
)

let encoder = JSONEncoder()
encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
let data = try encoder.encode(result)

FileHandle.standardOutput.write(data)
FileHandle.standardOutput.write(Data("\n".utf8))
