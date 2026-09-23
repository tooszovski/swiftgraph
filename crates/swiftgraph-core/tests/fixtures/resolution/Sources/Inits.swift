final class Session {
    let token: String

    init(token: String) {
        self.token = token
        configure()
    }

    convenience init() {
        self.init(token: "")
    }

    func configure() {}
}

func openSession() {
    let session = Session(token: "abc")
    _ = session
}
