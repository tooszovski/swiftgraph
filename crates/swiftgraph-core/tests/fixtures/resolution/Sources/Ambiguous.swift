final class SyncA { func sync() {} }
final class SyncB { func sync() {} }

final class LoadA { func load() {} }
final class LoadB { func load() {} }
final class LoadC { func load() {} }
final class LoadD { func load() {} }

func useUnknown() {
    // Receivers of unknown type: results of calls.
    makeObject().sync()
    makeObject().load()
}
