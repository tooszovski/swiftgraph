enum Units {
    static let secondInMillis = 1000
}

enum ApiRequest {}

struct Client {
    var pending: ApiRequest?

    func timeout() -> Int {
        Units.secondInMillis * 30
    }
}

final class Orphan {}

func orphanHelper() {}
