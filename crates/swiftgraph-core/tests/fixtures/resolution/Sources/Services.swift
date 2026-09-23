final class ProfileService {
    func update(id: Int) {}
    func fetch() {}
}

final class WalletService {
    func update(model: String) {}
}

struct CartStore {
    func update() {}

    func reload() {
        self.update()
        update()
    }
}

enum Helper {
    static func make() {}
}

struct Mapper {
    func map(_ value: Int) -> Int { value }
}
