import Foundation

/// Aggregated reading statistics from the stats engine.
public struct ReadingStats: Codable {
    public let totalBooks: Int
    public let booksRead: Int
    public let booksReading: Int
    public let booksWantToRead: Int
    public let totalPages: Int
    public let averageRating: Double?
    public let averagePages: Int?
    public let ratingDistribution: [Int]?
    public let formatBreakdown: FormatBreakdown?
    public let topAuthors: [AuthorCount]?
    public let topTags: [TagCount]?
    public let monthlyActivity: [MonthlyCount]?
    public let currentStreak: Int?
    public let longestStreak: Int?
}

public struct FormatBreakdown: Codable {
    public let physical: Int
    public let ebook: Int
    public let audiobook: Int
}

public struct AuthorCount: Codable {
    public let name: String
    public let count: Int
}

public struct TagCount: Codable {
    public let name: String
    public let count: Int
}

public struct MonthlyCount: Codable {
    public let month: String
    public let count: Int
}
