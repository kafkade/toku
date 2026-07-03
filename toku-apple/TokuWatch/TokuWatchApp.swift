import SwiftUI
import WidgetKit
import TokuKit

/// Entry point for the Toku watchOS companion app.
///
/// A wrist-first surface for reading tracking: currently-reading books, quick
/// progress logging, quick status actions, and glance stats. It reuses the shared
/// `TokuKit` view models over `toku-ffi`, keeps its own local database, and syncs
/// through the existing op-log — the same model as the iPhone app.
@main
struct TokuWatchApp: App {
    @StateObject private var appState = WatchAppState()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            WatchRootView()
                .environmentObject(appState)
        }
        .onChange(of: scenePhase) { _, newPhase in
            switch newPhase {
            case .active:
                appState.refresh()
            case .background, .inactive:
                appState.performBackgroundSync()
            @unknown default:
                break
            }
        }
    }
}

/// Shared app-level state holding the FFI connection and companion view models.
@MainActor
final class WatchAppState: ObservableObject {
    let ffi: TokuFFI?
    @Published var libraryVM: LibraryViewModel?
    @Published var statsVM: StatsViewModel?
    @Published var syncVM: SyncViewModel?
    @Published var errorMessage: String?

    init() {
        let dbPath = WatchAppState.defaultDatabasePath()
        do {
            let ffi = try TokuFFI(path: dbPath)
            self.ffi = ffi
            self.libraryVM = LibraryViewModel(ffi: ffi)
            self.statsVM = StatsViewModel(ffi: ffi)
            self.syncVM = SyncViewModel(ffi: ffi)
        } catch {
            self.ffi = nil
            self.errorMessage = "Failed to open database: \(error.localizedDescription)"
        }
    }

    /// Currently-reading books (active reading session).
    var currentlyReading: [Book] {
        (libraryVM?.books ?? []).filter { $0.status == .reading }
    }

    /// Load library + stats, then refresh the shared complication snapshot.
    func refresh() {
        libraryVM?.loadBooks()
        statsVM?.loadStats()
        // Reload after the async FFI calls settle so the snapshot reflects fresh data.
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: 400_000_000)
            self.updateComplicationSnapshot()
        }
    }

    /// Run a best-effort launch sync, refreshing views when remote changes arrive.
    func performLaunchSync() {
        syncVM?.syncOnLaunch { [weak self] pulledChanges in
            guard let self else { return }
            if pulledChanges {
                self.libraryVM?.loadBooks()
                self.statsVM?.loadStats()
            }
            Task { @MainActor in
                try? await Task.sleep(nanoseconds: 400_000_000)
                self.updateComplicationSnapshot()
            }
        }
    }

    /// Push pending local changes when the app moves to the background.
    func performBackgroundSync() {
        syncVM?.syncOnBackground()
    }

    /// Write the latest reading state to the shared snapshot and refresh complications.
    func updateComplicationSnapshot() {
        let current = currentlyReading.first
        let stats = statsVM?.stats
        let snapshot = WatchSnapshot(
            currentBookTitle: current?.title,
            currentBookAuthor: current?.authorDisplay,
            currentStreak: stats?.currentStreak ?? 0,
            booksThisYear: stats?.booksRead ?? 0
        )
        WatchSnapshotStore.save(snapshot)
        WidgetCenter.shared.reloadAllTimelines()
    }

    /// Default database path in the app's Application Support directory.
    static func defaultDatabasePath() -> String {
        let appSupport = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first!.appendingPathComponent("dev.toku.watch")

        try? FileManager.default.createDirectory(
            at: appSupport,
            withIntermediateDirectories: true
        )

        return appSupport.appendingPathComponent("library.db").path
    }
}
