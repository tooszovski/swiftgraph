import Foundation

protocol Identifiable {
    var id: UUID { get }
}

struct User: Identifiable {
    let id: UUID
    let name: String
    let email: String
}

struct Post: Identifiable {
    let id: UUID
    let title: String
    let body: String
    let authorId: UUID
}

enum Role: String {
    case admin
    case editor
    case viewer
}
