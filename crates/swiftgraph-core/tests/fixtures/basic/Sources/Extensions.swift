import Foundation

extension User {
    var displayName: String {
        name.isEmpty ? email : name
    }

    func toJSON() -> [String: Any] {
        [
            "id": id.uuidString,
            "name": name,
            "email": email
        ]
    }
}

extension Array where Element == User {
    func sortedByName() -> [User] {
        sorted { $0.name < $1.name }
    }
}

extension String {
    var isValidEmail: Bool {
        contains("@") && contains(".")
    }
}
