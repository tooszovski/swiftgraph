/// SwiftGraph Parser — extracts declarations from Swift files using swift-syntax.
///
/// Usage:
///   swiftgraph-parser --version      Print {"name","version","protocol"} and exit
///   swiftgraph-parser --stdin        Read file paths (one per line), print one JSON line per file
///   swiftgraph-parser <file.swift>   Print one JSON line for the file

import Foundation
import SwiftGraphParserCore

/// Collects encoded lines from concurrent workers.
final class Lines: @unchecked Sendable {
    private var storage: [String]
    private let lock = NSLock()

    init(count: Int) {
        storage = Array(repeating: "", count: count)
    }

    func set(_ index: Int, _ value: String) {
        lock.lock()
        storage[index] = value
        lock.unlock()
    }

    var all: [String] {
        lock.lock()
        defer { lock.unlock() }
        return storage
    }
}

func encodeLine<T: Encodable>(_ value: T) -> String {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
    guard let data = try? encoder.encode(value), let text = String(data: data, encoding: .utf8) else {
        return "{\"error\":\"encoding failed\",\"file\":\"\"}"
    }
    return text
}

func parseLine(_ path: String) -> String {
    do {
        return encodeLine(try parseFile(atPath: path))
    } catch {
        return encodeLine(ParseFailure(file: path, error: "\(error)"))
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
    let files = paths
    let lines = Lines(count: files.count)
    DispatchQueue.concurrentPerform(iterations: files.count) { index in
        lines.set(index, parseLine(files[index]))
    }
    for line in lines.all { writeLine(line) }
default:
    let line = parseLine(args[1])
    writeLine(line)
    if line.contains("\"error\"") && !line.contains("\"declarations\"") { exit(1) }
}
