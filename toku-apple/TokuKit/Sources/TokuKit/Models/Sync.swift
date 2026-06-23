import Foundation

/// Result of initializing sync (`toku sync init`).
public struct SyncInitOutcome: Codable, Hashable {
    public let deviceId: String
    public let libraryId: String
    public let deviceName: String
    public let server: String
    public let encryption: Bool
}

/// Result of a sync push.
public struct SyncPushOutcome: Codable, Hashable {
    /// Number of local ops that were pending before the push.
    public let pushed: Int
    public let accepted: Int
    public let duplicates: Int
    public let cursor: String?
    /// True when there was nothing to push.
    public let upToDate: Bool

    /// Human-readable summary.
    public var summary: String {
        upToDate
            ? "Already up to date"
            : "Pushed \(accepted) op(s)\(duplicates > 0 ? " (\(duplicates) duplicate)" : "")"
    }
}

/// Result of a sync pull.
public struct SyncPullOutcome: Codable, Hashable {
    public let pulled: Int
    public let cursor: String?

    /// Human-readable summary.
    public var summary: String {
        pulled == 0 ? "Already up to date" : "Pulled \(pulled) op(s)"
    }
}

/// Current sync status for the local library.
public struct SyncStatus: Codable, Hashable {
    public let enabled: Bool
    public let server: String
    public let deviceId: String
    public let deviceName: String
    public let libraryId: String
    public let encryption: Bool
    public let pendingOps: Int
    public let pushCursor: String?
    public let pullCursor: String?
    public let deviceCount: Int
}

/// A device registered to the library on the sync server.
public struct SyncDevice: Codable, Identifiable, Hashable {
    public let deviceId: String
    public let deviceName: String
    public let lastSeen: String?
    public let createdAt: String

    public var id: String { deviceId }
}
