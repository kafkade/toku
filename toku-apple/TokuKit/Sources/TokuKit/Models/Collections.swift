import Foundation

/// A tag with its type and count.
public struct Tag: Codable, Identifiable, Hashable {
    public let name: String
    public let tagType: String
    public let count: Int

    public var id: String { name }
}

/// A shelf (collection) of books.
public struct Shelf: Codable, Identifiable, Hashable {
    public let name: String
    public let isSmart: Bool
    public let bookCount: Int

    public var id: String { name }
}

/// Result of a Goodreads CSV import.
public struct ImportReport: Codable {
    public let totalRows: Int
    public let imported: Int
    public let skipped: Int
    public let updated: Int
    public let errors: Int

    /// Human-readable summary.
    public var summary: String {
        "\(imported) imported, \(skipped) skipped, \(updated) updated, \(errors) errors (of \(totalRows) total)"
    }
}
