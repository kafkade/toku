import Foundation
import CTokuFFI

/// Swift errors wrapping TokuStatus codes.
public enum TokuError: LocalizedError {
    case nullPointer
    case invalidUtf8
    case notFound
    case database(String)
    case panic(String)
    case unknown(Int32)

    public var errorDescription: String? {
        switch self {
        case .nullPointer:
            return "Internal error: null pointer passed to FFI"
        case .invalidUtf8:
            return "Invalid text encoding"
        case .notFound:
            return "Not found"
        case .database(let msg):
            return "Database error: \(msg)"
        case .panic(let msg):
            return "Internal error: \(msg)"
        case .unknown(let code):
            return "Unknown error (code \(code))"
        }
    }
}

/// Type-safe Swift wrapper around the toku-ffi C API.
///
/// All methods are synchronous and must be called from the same thread
/// that opened the database (SQLite `!Send` constraint).
public final class TokuFFI: @unchecked Sendable {
    private let db: OpaquePointer

    /// Open (or create) a Toku database at the given path.
    public init(path: String) throws {
        var handle: OpaquePointer?
        let status = path.withCString { cPath in
            toku_open(cPath, &handle)
        }
        try TokuFFI.check(status)
        guard let h = handle else {
            throw TokuError.nullPointer
        }
        self.db = h
    }

    deinit {
        toku_close(db)
    }

    // MARK: - Books

    /// Add a book with a title and optional author. Returns the new UUID.
    public func addBook(title: String, author: String? = nil) throws -> String {
        var outId: UnsafeMutablePointer<CChar>?
        let status: TokuStatus
        if let author = author {
            status = title.withCString { t in
                author.withCString { a in
                    toku_add_book(db, t, a, &outId)
                }
            }
        } else {
            status = title.withCString { t in
                toku_add_book(db, t, nil, &outId)
            }
        }
        try TokuFFI.check(status)
        defer { toku_free_string(outId) }
        guard let ptr = outId else { throw TokuError.nullPointer }
        return String(cString: ptr)
    }

    /// List all books as decoded model objects.
    public func listBooks() throws -> [Book] {
        let json = try callJson { toku_list_books(db, &$0) }
        return try JSONDecoder.toku.decode([Book].self, from: json)
    }

    /// Get a single book by UUID.
    public func getBook(id: String) throws -> Book {
        let json = try callJsonWithId(id) { toku_get_book(db, $0, &$1) }
        return try JSONDecoder.toku.decode(Book.self, from: json)
    }

    /// Delete a book by UUID.
    public func deleteBook(id: String) throws {
        let status = id.withCString { toku_delete_book(db, $0) }
        try TokuFFI.check(status)
    }

    /// Update a book's reading status.
    public func updateBookStatus(id: String, status: ReadingStatus) throws {
        let result = id.withCString { idPtr in
            status.rawValue.withCString { sPtr in
                toku_update_book_status(db, idPtr, sPtr)
            }
        }
        try TokuFFI.check(result)
    }

    /// Update a book's rating (0–10). Pass nil to clear.
    public func updateBookRating(id: String, rating: Int?) throws {
        let r: Int32 = rating.map { Int32($0) } ?? -1
        let status = id.withCString { toku_update_book_rating(db, $0, r) }
        try TokuFFI.check(status)
    }

    // MARK: - Search

    /// Full-text search for books.
    public func searchBooks(query: String) throws -> [Book] {
        var outJson: UnsafeMutablePointer<CChar>?
        let status = query.withCString { toku_search_books(db, $0, &outJson) }
        try TokuFFI.check(status)
        defer { toku_free_string(outJson) }
        guard let ptr = outJson else { throw TokuError.nullPointer }
        let data = Data(String(cString: ptr).utf8)
        return try JSONDecoder.toku.decode([Book].self, from: data)
    }

    // MARK: - Statistics

    /// Get reading statistics. Pass nil for all-time, or a year for scoped stats.
    public func getStats(year: Int? = nil) throws -> ReadingStats {
        var outJson: UnsafeMutablePointer<CChar>?
        let y: Int32 = year.map { Int32($0) } ?? 0
        let status = toku_get_stats(db, y, &outJson)
        try TokuFFI.check(status)
        defer { toku_free_string(outJson) }
        guard let ptr = outJson else { throw TokuError.nullPointer }
        let data = Data(String(cString: ptr).utf8)
        return try JSONDecoder.toku.decode(ReadingStats.self, from: data)
    }

    // MARK: - Tags

    /// List all tags with counts.
    public func listTags() throws -> [Tag] {
        let json = try callJson { toku_list_tags(db, &$0) }
        return try JSONDecoder.toku.decode([Tag].self, from: json)
    }

    /// Get tags for a specific book.
    public func getBookTags(id: String) throws -> [Tag] {
        let json = try callJsonWithId(id) { toku_get_book_tags(db, $0, &$1) }
        return try JSONDecoder.toku.decode([Tag].self, from: json)
    }

    // MARK: - Shelves

    /// List all shelves with book counts.
    public func listShelves() throws -> [Shelf] {
        let json = try callJson { toku_list_shelves(db, &$0) }
        return try JSONDecoder.toku.decode([Shelf].self, from: json)
    }

    // MARK: - Import

    /// Import books from a Goodreads CSV file.
    public func importGoodreads(csvPath: String, dryRun: Bool = false) throws -> ImportReport {
        var outJson: UnsafeMutablePointer<CChar>?
        let status = csvPath.withCString { toku_import_goodreads(db, $0, dryRun, &outJson) }
        try TokuFFI.check(status)
        defer { toku_free_string(outJson) }
        guard let ptr = outJson else { throw TokuError.nullPointer }
        let data = Data(String(cString: ptr).utf8)
        return try JSONDecoder.toku.decode(ImportReport.self, from: data)
    }

    // MARK: - Internal helpers

    /// Call an FFI function that writes JSON to an out-pointer.
    private func callJson(
        _ fn: (inout UnsafeMutablePointer<CChar>?) -> TokuStatus
    ) throws -> Data {
        var outJson: UnsafeMutablePointer<CChar>?
        let status = fn(&outJson)
        try TokuFFI.check(status)
        defer { toku_free_string(outJson) }
        guard let ptr = outJson else { throw TokuError.nullPointer }
        return Data(String(cString: ptr).utf8)
    }

    /// Call an FFI function that takes an ID and writes JSON.
    private func callJsonWithId(
        _ id: String,
        _ fn: (UnsafePointer<CChar>, inout UnsafeMutablePointer<CChar>?) -> TokuStatus
    ) throws -> Data {
        var outJson: UnsafeMutablePointer<CChar>?
        let status = id.withCString { fn($0, &outJson) }
        try TokuFFI.check(status)
        defer { toku_free_string(outJson) }
        guard let ptr = outJson else { throw TokuError.nullPointer }
        return Data(String(cString: ptr).utf8)
    }

    /// Map a TokuStatus to a Swift error (no-op for OK).
    private static func check(_ status: TokuStatus) throws {
        switch status {
        case TOKU_STATUS_OK:
            return
        case TOKU_STATUS_ERROR_NULL_POINTER:
            throw TokuError.nullPointer
        case TOKU_STATUS_ERROR_INVALID_UTF8:
            throw TokuError.invalidUtf8
        case TOKU_STATUS_ERROR_NOT_FOUND:
            throw TokuError.notFound
        case TOKU_STATUS_ERROR_DB:
            throw TokuError.database(lastError())
        case TOKU_STATUS_ERROR_PANIC:
            throw TokuError.panic(lastError())
        default:
            throw TokuError.unknown(Int32(bitPattern: status.rawValue))
        }
    }

    private static func lastError() -> String {
        guard let ptr = toku_last_error() else { return "unknown" }
        return String(cString: ptr)
    }
}

extension JSONDecoder {
    static let toku: JSONDecoder = {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return d
    }()
}
