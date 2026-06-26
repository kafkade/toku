import Foundation

/// ViewModel backing the sync settings screen.
///
/// Wraps the FFI sync operations (init, push, pull, status, devices) and exposes
/// observable state for SwiftUI. All FFI calls run on a dedicated serial queue to
/// respect SQLite's single-thread constraint, matching the other view models.
@MainActor
public final class SyncViewModel: ObservableObject {
    @Published public var status: SyncStatus?
    @Published public var devices: [SyncDevice] = []
    @Published public var conflicts: [SyncConflict] = []
    @Published public var isBusy = false
    @Published public var lastResult: String?
    @Published public var errorMessage: String?

    /// Whether sync has been configured on this device.
    public var isConfigured: Bool { status != nil }

    /// Number of unresolved conflicts reported by the latest status refresh.
    public var conflictCount: Int { status?.conflicts ?? 0 }

    /// Whether the latest status reports unresolved conflicts needing attention.
    public var hasConflicts: Bool { conflictCount > 0 }

    private let ffi: TokuFFI
    private let queue = DispatchQueue(label: "dev.toku.sync", qos: .userInitiated)
    private var hasRunLaunchSync = false

    public init(ffi: TokuFFI) {
        self.ffi = ffi
    }

    /// Refresh the sync status and device list. Safe to call when sync is not configured.
    public func refresh() {
        isBusy = true
        errorMessage = nil

        queue.async { [weak self] in
            guard let self else { return }
            // status() throws when sync is not configured — treat that as "not configured".
            let status = try? self.ffi.syncStatus()
            let devices = (try? self.ffi.syncDevices()) ?? []
            // Conflicts live in the local DB; load them whenever sync is configured.
            let conflicts = status == nil ? [] : ((try? self.ffi.syncConflicts()) ?? [])
            Task { @MainActor in
                self.status = status
                self.devices = devices
                self.conflicts = conflicts
                self.isBusy = false
            }
        }
    }

    /// Reload just the unresolved conflict list (and status, so counts stay in sync).
    public func loadConflicts() {
        refresh()
    }

    /// Resolve a single conflict, keeping the local or remote value.
    public func resolve(_ conflict: SyncConflict, keep: ConflictKeep) {
        run(label: "Conflict resolved") { ffi in
            _ = try ffi.syncResolveConflict(id: conflict.id, keep: keep)
            return "Conflict resolved"
        }
    }

    /// Resolve every unresolved conflict with the same choice.
    public func resolveAll(keep: ConflictKeep) {
        run { ffi in
            let count = try ffi.syncResolveAllConflicts(keep: keep)
            return count == 0 ? "No conflicts to resolve" : "Resolved \(count) conflict(s)"
        }
    }

    /// Configure sync against `server`, optionally enabling encryption with `passphrase`.
    public func initialize(server: String, deviceName: String?, passphrase: String?) {
        run(label: "Sync configured") { try $0.syncInit(server: server, deviceName: deviceName, passphrase: passphrase) }
    }

    /// Push pending local changes to the server.
    public func push() {
        run { try $0.syncPush().summary }
    }

    /// Pull remote changes from the server.
    public func pull() {
        run { try $0.syncPull().summary }
    }

    /// Push then pull in one action — the common "sync now" gesture.
    public func syncNow() {
        run { ffi in
            let pushed = try ffi.syncPush()
            let pulled = try ffi.syncPull()
            return "\(pushed.summary); \(pulled.summary)"
        }
    }

    /// Best-effort automatic sync run once at app launch.
    ///
    /// Does nothing when sync is not configured. Unlike the interactive actions,
    /// failures (e.g. the server is unreachable) are recorded in `errorMessage`
    /// but never surfaced as blocking alerts — a launch sync is fire-and-forget.
    /// `completion` is invoked on the main actor with `true` when remote changes
    /// were applied, so callers can refresh dependent views.
    public func syncOnLaunch(completion: (@MainActor (Bool) -> Void)? = nil) {
        guard !hasRunLaunchSync else {
            completion.map { done in Task { @MainActor in done(false) } }
            return
        }
        hasRunLaunchSync = true
        isBusy = true
        errorMessage = nil

        queue.async { [weak self] in
            guard let self else { return }

            // Determine whether sync is configured without surfacing an error.
            guard (try? self.ffi.syncStatus()) != nil else {
                Task { @MainActor in
                    self.isBusy = false
                    completion?(false)
                }
                return
            }

            var pulledChanges = false
            var failure: String?
            do {
                _ = try self.ffi.syncPush()
                let pulled = try self.ffi.syncPull()
                pulledChanges = pulled.pulled > 0
            } catch {
                failure = error.localizedDescription
            }

            Task { @MainActor in
                self.isBusy = false
                if let failure {
                    self.errorMessage = failure
                } else {
                    self.lastResult = "Synced at launch"
                }
                self.refresh()
                completion?(pulledChanges)
            }
        }
    }

    // MARK: - Internal

    /// Best-effort push of pending local changes when the app moves to the background.
    ///
    /// Fire-and-forget: does nothing when sync is not configured, and failures
    /// (e.g. the server is unreachable) are recorded in `errorMessage` but never
    /// surfaced as blocking alerts. Intended to be called from `scenePhase`
    /// transitions to `.background` / `.inactive`.
    public func syncOnBackground() {
        isBusy = true
        errorMessage = nil

        queue.async { [weak self] in
            guard let self else { return }

            // Only push when sync is configured.
            guard (try? self.ffi.syncStatus()) != nil else {
                Task { @MainActor in self.isBusy = false }
                return
            }

            var failure: String?
            do {
                _ = try self.ffi.syncPush()
            } catch {
                failure = error.localizedDescription
            }

            Task { @MainActor in
                self.isBusy = false
                if let failure {
                    self.errorMessage = failure
                } else {
                    self.lastResult = "Pushed on background"
                }
            }
        }
    }

    /// Run an FFI sync action that produces a human-readable result string, then refresh.
    private func run(label: String? = nil, _ action: @escaping (TokuFFI) throws -> String) {
        isBusy = true
        errorMessage = nil
        lastResult = nil

        queue.async { [weak self] in
            guard let self else { return }
            do {
                let message = try action(self.ffi)
                Task { @MainActor in
                    self.lastResult = label ?? message
                    self.isBusy = false
                    self.refresh()
                }
            } catch {
                Task { @MainActor in
                    self.errorMessage = error.localizedDescription
                    self.isBusy = false
                }
            }
        }
    }

    /// Overload for actions returning a non-String (e.g. `syncInit`); records `label`.
    private func run<T>(label: String, _ action: @escaping (TokuFFI) throws -> T) {
        run(label: label) { ffi in
            _ = try action(ffi)
            return label
        }
    }
}
