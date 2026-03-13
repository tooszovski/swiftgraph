import Foundation

actor DataStore {
    private var items: [String: Any] = [:]

    func get(_ key: String) -> Any? {
        items[key]
    }

    func set(_ key: String, value: Any) {
        items[key] = value
    }
}

@MainActor
class ScreenViewModel: ObservableObject {
    @Published var data: String = ""
    private let store = DataStore()

    func load() {
        Task {
            let value = await store.get("key")
            data = value as? String ?? ""
        }
    }

    func save() {
        // Potential issue: Task.detached accessing self
        Task.detached { [weak self] in
            guard let self else { return }
            await self.store.set("key", value: await self.data)
        }
    }
}

// Missing @MainActor — should be caught by CONC-001
class LegacyViewController: NSObject {
    var delegate: AnyObject? // Strong delegate — MEM-002

    func refresh() {
        Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            self?.updateUI()
        }
    }

    func updateUI() {
        // ...
    }
}
