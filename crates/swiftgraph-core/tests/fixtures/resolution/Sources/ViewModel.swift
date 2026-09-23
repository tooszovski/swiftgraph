final class ViewModel {
    let profile: ProfileService
    private let wallet = WalletService()

    init(profile: ProfileService) {
        self.profile = profile
    }

    func refresh(items: [Int]) {
        profile.update(id: 1)
        self.wallet.update(model: "")
        let local: CartStore = CartStore()
        local.update()
        let doubled = items.map { $0 * 2 }
        _ = doubled.filter { $0 > 1 }
        Helper.make()
    }

    func reset(store: CartStore) {
        store.update()
    }
}
