import Foundation

struct Settings: Codable {
    var theme: String

    enum CodingKeys: String, CodingKey {
        case theme = "theme_name"
    }
}

final class Counter {
    private var storage = 0

    /// Current value.
    var value: Int {
        get { storage }
        set { storage = newValue }
    }

    func unusedHelper() {}
}

struct Tag {}

func makeCounter(tag: Tag) -> Counter {
    let counter = Counter()
    counter.value = 1
    return counter
}

@MainActor final class CounterViewModel: ObservableObject {
    @Published var count = 0

    func refresh() async {
        count = makeCounter(tag: Tag()).value
    }
}
