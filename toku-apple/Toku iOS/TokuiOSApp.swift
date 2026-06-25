import SwiftUI
import TokuKit

/// Main entry point for the Toku iOS app.
@main
struct TokuiOSApp: App {
    @StateObject private var appState = AppState()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(appState)
        }
    }
}

/// Shared app-level state holding the FFI connection and view models.
@MainActor
final class AppState: ObservableObject {
    let ffi: TokuFFI?
    @Published var libraryVM: LibraryViewModel?
    @Published var statsVM: StatsViewModel?
    @Published var importVM: ImportViewModel?
    @Published var syncVM: SyncViewModel?
    @Published var errorMessage: String?

    init() {
        let dbPath = AppState.defaultDatabasePath()

        do {
            let ffi = try TokuFFI(path: dbPath)
            self.ffi = ffi
            self.libraryVM = LibraryViewModel(ffi: ffi)
            self.statsVM = StatsViewModel(ffi: ffi)
            self.importVM = ImportViewModel(ffi: ffi)
            self.syncVM = SyncViewModel(ffi: ffi)
        } catch {
            self.ffi = nil
            self.errorMessage = "Failed to open database: \(error.localizedDescription)"
        }
    }

    /// Run a best-effort sync at launch and refresh the library and stats if
    /// remote changes were applied. Safe to call when sync is not configured.
    func performLaunchSync() {
        syncVM?.syncOnLaunch { [weak self] pulledChanges in
            guard pulledChanges else { return }
            self?.libraryVM?.loadBooks()
            self?.statsVM?.loadStats()
        }
    }

    /// Default database path in the app's Application Support directory.
    static func defaultDatabasePath() -> String {
        let appSupport = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first!.appendingPathComponent("dev.toku.ios")

        try? FileManager.default.createDirectory(
            at: appSupport,
            withIntermediateDirectories: true
        )

        return appSupport.appendingPathComponent("library.db").path
    }
}
