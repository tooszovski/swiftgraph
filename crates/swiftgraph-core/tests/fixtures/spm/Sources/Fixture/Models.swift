/// A user of the app.
public struct User: Identifiable, Sendable {
    public let id: Int
    public var name: String
    public static let guest = User(id: 0, name: "Гость")
}

public protocol UserStore {
    func load(id: Int) -> User?
}
