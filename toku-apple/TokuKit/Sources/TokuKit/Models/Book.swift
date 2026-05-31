import Foundation

/// A book in the user's library.
public struct Book: Codable, Identifiable, Hashable {
    public let id: String
    public let title: String
    public let subtitle: String?
    public let status: ReadingStatus
    public let rating: Int?
    public let pageCount: Int?
    public let format: BookFormat
    public let pubDate: String?
    public let language: String?
    public let authors: [String]

    /// Display-friendly author string (comma-separated).
    public var authorDisplay: String {
        authors.isEmpty ? "Unknown Author" : authors.joined(separator: ", ")
    }

    /// Star rating for display (0–5 with half-star precision).
    public var starRating: Double? {
        rating.map { Double($0) / 2.0 }
    }
}

/// Reading status matching the Rust `ReadingStatus` enum's serialized form.
public enum ReadingStatus: String, Codable, CaseIterable, Identifiable {
    case wantToRead = "want-to-read"
    case reading = "reading"
    case read = "read"
    case onHold = "on-hold"
    case didNotFinish = "did-not-finish"

    public var id: String { rawValue }

    public var displayName: String {
        switch self {
        case .wantToRead: return "Want to Read"
        case .reading: return "Reading"
        case .read: return "Read"
        case .onHold: return "On Hold"
        case .didNotFinish: return "Did Not Finish"
        }
    }

    public var systemImage: String {
        switch self {
        case .wantToRead: return "bookmark"
        case .reading: return "book.fill"
        case .read: return "checkmark.circle.fill"
        case .onHold: return "pause.circle"
        case .didNotFinish: return "xmark.circle"
        }
    }
}

/// Book format matching the Rust `BookFormat` enum.
public enum BookFormat: String, Codable, CaseIterable {
    case physical = "physical"
    case ebook = "ebook"
    case audiobook = "audiobook"

    public var displayName: String {
        switch self {
        case .physical: return "Physical"
        case .ebook: return "E-Book"
        case .audiobook: return "Audiobook"
        }
    }
}
