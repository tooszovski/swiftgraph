/// SwiftGraph Parser — extracts declarations from Swift files using swift-syntax.
///
/// Usage:
///   swiftgraph-parser --version      Print {"name","version","protocol"} and exit
///   swiftgraph-parser --stdin        Read file paths (one per line), print one JSON line per file
///   swiftgraph-parser <file.swift>   Print one JSON line for the file

import Foundation
import os
import SwiftGraphParserCore

func encodeLine<T: Encodable>(_ value: T) -> String {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
    guard let data = try? encoder.encode(value), let text = String(data: data, encoding: .utf8) else {
        return "{\"error\":\"encoding failed\",\"file\":\"\"}"
    }
    return text
}

func parseLine(_ path: String) -> (line: String, ok: Bool) {
    do {
        return (encodeLine(try parseFile(atPath: path)), true)
    } catch {
        return (encodeLine(ParseFailure(file: path, error: "\(error)")), false)
    }
}

func writeLine(_ line: String) {
    FileHandle.standardOutput.write(Data((line + "\n").utf8))
}

let args = CommandLine.arguments
guard args.count >= 2 else {
    FileHandle.standardError.write(Data("Usage: swiftgraph-parser --version | --stdin | <file.swift>\n".utf8))
    exit(2)
}

switch args[1] {
case "--version":
    writeLine(encodeLine(ParserInfo.current))
case "--stdin":
    var paths: [String] = []
    while let line = readLine(strippingNewline: true) {
        let path = line.trimmingCharacters(in: .whitespaces)
        if !path.isEmpty { paths.append(path) }
    }
    // Parsing is synchronous CPU work, so a parallel-for is the right tool;
    // results go into a Sendable lock-protected buffer, kept in input order.
    let files = paths
    let lines = OSAllocatedUnfairLock(initialState: [String](repeating: "", count: files.count))
    DispatchQueue.concurrentPerform(iterations: files.count) { index in
        let line = parseLine(files[index]).line
        lines.withLock { $0[index] = line }
    }
    for line in lines.withLock({ $0 }) { writeLine(line) }
default:
    let result = parseLine(args[1])
    writeLine(result.line)
    if !result.ok { exit(1) }
}
