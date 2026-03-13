import Foundation

@MainActor
class UserListViewModel: ObservableObject {
    @Published var users: [User] = []
    @Published var isLoading = false
    @Published var error: String?

    private let service: UserService

    init(service: UserService) {
        self.service = service
    }

    func loadUsers() {
        Task {
            isLoading = true
            defer { isLoading = false }
            do {
                let user = try await service.fetchUser(id: UUID())
                users.append(user)
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}
