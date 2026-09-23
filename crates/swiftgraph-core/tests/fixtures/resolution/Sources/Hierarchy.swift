protocol Coordinating {}

extension Coordinating {
    func dismiss() {}
}

class BaseScreen {
    func track() {}
}

final class DetailScreen: BaseScreen, Coordinating {
    func close() {
        dismiss()
        track()
    }
}
