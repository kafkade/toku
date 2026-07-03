import Foundation

/// ViewModel for the library screen (table and grid views).
///
/// Manages the book list, search, sorting, and status filtering.
/// All FFI calls happen on a dedicated serial queue to respect
/// SQLite's single-thread constraint.
@MainActor
public final class LibraryViewModel: ObservableObject {
    @Published public var books: [Book] = []
    @Published public var searchText = ""
    @Published public var sortOrder: SortOrder = .title
    @Published public var isLoading = false
    @Published public var errorMessage: String?

    private let ffi: TokuFFI
    private let queue = DispatchQueue(label: "dev.toku.library", qos: .userInitiated)

    public enum SortOrder: String, CaseIterable, Identifiable {
        case title, author, rating, status, dateAdded

        public var id: String { rawValue }

        public var displayName: String {
            switch self {
            case .title: return "Title"
            case .author: return "Author"
            case .rating: return "Rating"
            case .status: return "Status"
            case .dateAdded: return "Date Added"
            }
        }
    }

    public init(ffi: TokuFFI) {
        self.ffi = ffi
    }

    /// Load all books from the database.
    public func loadBooks() {
        isLoading = true
        errorMessage = nil

        queue.async { [weak self] in
            guard let self else { return }
            do {
                let result: [Book]
                if self.searchText.isEmpty {
                    result = try self.ffi.listBooks()
                } else {
                    result = try self.ffi.searchBooks(query: self.searchText)
                }
                let sorted = self.sorted(result)
                Task { @MainActor in
                    self.books = sorted
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

    /// Delete a book by ID.
    public func deleteBook(id: String) {
        queue.async { [weak self] in
            guard let self else { return }
            do {
                try self.ffi.deleteBook(id: id)
                Task { @MainActor in
                    self.books.removeAll { $0.id == id }
                }
            } catch {
                Task { @MainActor in
                    self.errorMessage = error.localizedDescription
                }
            }
        }
    }

    /// Update a book's reading status.
    public func updateStatus(id: String, status: ReadingStatus) {
        queue.async { [weak self] in
            guard let self else { return }
            do {
                try self.ffi.updateBookStatus(id: id, status: status)
                Task { @MainActor in self.loadBooks() }
            } catch {
                Task { @MainActor in
                    self.errorMessage = error.localizedDescription
                }
            }
        }
    }

    /// Update a book's rating.
    public func updateRating(id: String, rating: Int?) {
        queue.async { [weak self] in
            guard let self else { return }
            do {
                try self.ffi.updateBookRating(id: id, rating: rating)
                Task { @MainActor in self.loadBooks() }
            } catch {
                Task { @MainActor in
                    self.errorMessage = error.localizedDescription
                }
            }
        }
    }

    /// Log a reading-progress entry for a book. `completion` runs on the main actor
    /// after the reload settles. Callers that also want to advance the book's status
    /// (e.g. to `.reading`) should call `updateStatus` alongside this.
    public func logProgress(
        id: String,
        type: ProgressType,
        value: Int,
        completion: (@MainActor () -> Void)? = nil
    ) {
        queue.async { [weak self] in
            guard let self else { return }
            do {
                try self.ffi.logProgress(id: id, type: type, value: value)
                Task { @MainActor in
                    self.loadBooks()
                    completion?()
                }
            } catch {
                Task { @MainActor in
                    self.errorMessage = error.localizedDescription
                    completion?()
                }
            }
        }
    }

    private func sorted(_ books: [Book]) -> [Book] {
        switch sortOrder {
        case .title:
            return books.sorted { $0.title.localizedCaseInsensitiveCompare($1.title) == .orderedAscending }
        case .author:
            return books.sorted { $0.authorDisplay.localizedCaseInsensitiveCompare($1.authorDisplay) == .orderedAscending }
        case .rating:
            return books.sorted { ($0.rating ?? 0) > ($1.rating ?? 0) }
        case .status:
            return books.sorted { $0.status.rawValue < $1.status.rawValue }
        case .dateAdded:
            return books // already in insertion order
        }
    }
}
