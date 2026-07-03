import Foundation

/// A tiny, glanceable snapshot of reading state shared between the watch app and
/// its WidgetKit complication.
///
/// The watch app writes this whenever its data changes; the complication reads it
/// to render the current book and streak on the watch face. It is intentionally
/// small and dependency-free (no `toku-ffi`) so the complication extension stays
/// lightweight.
public struct WatchSnapshot: Codable, Equatable {
    /// Title of the book with the most recent active reading session, if any.
    public var currentBookTitle: String?
    /// Author display string for `currentBookTitle`.
    public var currentBookAuthor: String?
    /// Current reading streak in days.
    public var currentStreak: Int
    /// Books finished in the current calendar year.
    public var booksThisYear: Int
    /// When this snapshot was produced.
    public var updatedAt: Date

    public init(
        currentBookTitle: String? = nil,
        currentBookAuthor: String? = nil,
        currentStreak: Int = 0,
        booksThisYear: Int = 0,
        updatedAt: Date = Date()
    ) {
        self.currentBookTitle = currentBookTitle
        self.currentBookAuthor = currentBookAuthor
        self.currentStreak = currentStreak
        self.booksThisYear = booksThisYear
        self.updatedAt = updatedAt
    }

    /// An empty placeholder used before the app has written any real data.
    public static let placeholder = WatchSnapshot(
        currentBookTitle: nil,
        currentBookAuthor: nil,
        currentStreak: 0,
        booksThisYear: 0
    )
}

/// Shared storage for the `WatchSnapshot`, backed by an App Group so the watch app
/// and complication extension can exchange it.
///
/// Live complication data requires the App Group to be provisioned (a signing team
/// plus the `group.com.kafkade.toku` capability on both targets). When the group is
/// unavailable (e.g. an unsigned simulator build), reads gracefully fall back to a
/// placeholder and writes are best-effort, so the app and extension still build and
/// run.
public enum WatchSnapshotStore {
    /// App Group identifier shared by the watch app and the complication.
    public static let appGroupID = "group.com.kafkade.toku"

    private static let key = "watch.snapshot.v1"

    private static var defaults: UserDefaults {
        UserDefaults(suiteName: appGroupID) ?? .standard
    }

    /// Persist the latest snapshot. Best-effort: failures are ignored.
    public static func save(_ snapshot: WatchSnapshot) {
        guard let data = try? JSONEncoder().encode(snapshot) else { return }
        defaults.set(data, forKey: key)
    }

    /// Load the latest snapshot, or `nil` if none has been written yet.
    public static func load() -> WatchSnapshot? {
        guard let data = defaults.data(forKey: key) else { return nil }
        return try? JSONDecoder().decode(WatchSnapshot.self, from: data)
    }
}
