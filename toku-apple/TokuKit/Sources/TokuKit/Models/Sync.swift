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
    public let unresolvedConflicts: Int

    /// Number of unresolved sync conflicts awaiting review.
    public var conflicts: Int { unresolvedConflicts }

    /// Whether any conflicts need the user's attention.
    public var hasConflicts: Bool { unresolvedConflicts > 0 }
}

/// A device registered to the library on the sync server.
public struct SyncDevice: Codable, Identifiable, Hashable {
    public let deviceId: String
    public let deviceName: String
    public let lastSeen: String?
    public let createdAt: String

    public var id: String { deviceId }
}

/// Which side of a sync conflict to keep when resolving.
public enum ConflictKeep: String, Codable, Hashable, CaseIterable {
    /// Keep this device's local value.
    case local
    /// Keep the incoming remote value.
    case remote
}

/// An unresolved sync conflict awaiting user review.
///
/// Conflicts are only produced for note and review edits that collide across
/// devices; all other entity types merge silently.
public struct SyncConflict: Codable, Identifiable, Hashable {
    public let id: String
    public let entityType: String
    public let entityId: String
    public let fieldName: String?
    public let localValue: String?
    public let remoteValue: String?
    public let localHlc: String
    public let remoteHlc: String
    public let createdAt: String

    /// The value that would remain if the given side is kept.
    public func keptValue(_ keep: ConflictKeep) -> String? {
        switch keep {
        case .local: return localValue
        case .remote: return remoteValue
        }
    }
}

/// Result of resolving one or more conflicts.
public struct ConflictResolveOutcome: Codable, Hashable {
    /// For a single resolve, `1` when applied or `0` when already resolved/missing.
    /// For a bulk resolve, the number of conflicts resolved.
    public let resolved: Int

    private enum CodingKeys: String, CodingKey { case resolved }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        // `resolve_conflict` returns a bool; `resolve_all` returns an int.
        if let count = try? container.decode(Int.self, forKey: .resolved) {
            resolved = count
        } else {
            resolved = (try container.decode(Bool.self, forKey: .resolved)) ? 1 : 0
        }
    }
}
