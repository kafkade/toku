import Foundation

/// ViewModel for the book detail inspector panel.
@MainActor
public final class BookDetailViewModel: ObservableObject {
    @Published public var book: Book?
    @Published public var tags: [Tag] = []
    @Published public var isLoading = false
    @Published public var errorMessage: String?

    private let ffi: TokuFFI
    private let queue = DispatchQueue(label: "dev.toku.bookdetail", qos: .userInitiated)

    public init(ffi: TokuFFI) {
        self.ffi = ffi
    }

    /// Load a book and its tags by ID.
    public func loadBook(id: String) {
        isLoading = true
        errorMessage = nil

        queue.async { [weak self] in
            guard let self else { return }
            do {
                let book = try self.ffi.getBook(id: id)
                let tags = try self.ffi.getBookTags(id: id)
                Task { @MainActor in
                    self.book = book
                    self.tags = tags
                    self.isLoading = false
                }
            } catch {
                Task { @MainActor in
                    self.errorMessage = error.localizedDescription
                    self.isLoading = false
                }
            }
        }
    }
}
