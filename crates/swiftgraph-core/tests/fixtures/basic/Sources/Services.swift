import Foundation

protocol UserService {
    func fetchUser(id: UUID) async throws -> User
    func saveUser(_ user: User) async throws
}

class NetworkUserService: UserService {
    private let baseURL: URL

    init(baseURL: URL) {
        self.baseURL = baseURL
    }

    func fetchUser(id: UUID) async throws -> User {
        // Network implementation
        return User(id: id, name: "Test", email: "test@example.com")
    }

    func saveUser(_ user: User) async throws {
        // Network implementation
    }
}

class CachedUserService: UserService {
    private let inner: UserService
    private var cache: [UUID: User] = [:]

    init(inner: UserService) {
        self.inner = inner
    }

    func fetchUser(id: UUID) async throws -> User {
        if let cached = cache[id] {
            return cached
        }
        let user = try await inner.fetchUser(id: id)
        cache[id] = user
        return user
    }

    func saveUser(_ user: User) async throws {
        try await inner.saveUser(user)
        cache[user.id] = user
    }
}
