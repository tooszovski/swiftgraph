protocol WalletUpdating {
    func refresh(force: Bool)
    func reset()
}

extension WalletUpdating {
    func reset() {}
}

class BaseWallet: WalletUpdating {
    func refresh(force: Bool) {}
}

final class LedgerWallet: BaseWallet {}

protocol Syncing {
    func sync(now: Bool)
}

struct Portfolio {}

extension Portfolio: Syncing {
    func sync(now: Bool) {}
}

func refreshAll(wallet: WalletUpdating, syncing: Syncing) {
    wallet.refresh(force: true)
    syncing.sync(now: false)
}

protocol Pinging {
    func ping()
}

protocol Naming {}

typealias Endpoint = Pinging & Naming

final class Server: Endpoint {
    func ping() {}
}

func check(endpoint: Endpoint) {
    endpoint.ping()
}
