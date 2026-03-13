/// JSON output protocol for SwiftGraph parser.
/// Version 1: declaration extraction.

struct ParseResult: Codable {
    let version: Int
    let file: String
    let declarations: [Declaration]
    let imports: [String]
}

struct Declaration: Codable {
    let name: String
    let kind: String
    let line: Int
    let endLine: Int?
    let attributes: [String]
    let accessLevel: String?
    let signature: String?
    let docComment: String?
    let members: [Declaration]?
}
