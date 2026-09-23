public final class MemoryStore: UserStore {
    fileprivate var users: [Int: User] = [:]
    package var hits = 0

    public init() {}

    public func load(id: Int) -> User? {
        hits += 1
        return users[id]
    }

    private func reset() {
        users.removeAll()
    }
}

@MainActor public final class Screen {
    let store: UserStore

    public init(store: UserStore) {
        self.store = store
    }

    public func show(id: Int) -> String {
        store.load(id: id)?.name ?? greeting(name: "мир")
    }
}

func greeting(name: String) -> String {
    "Привет, \(name)! 👋"
}
