import SwiftUI
import TokuKit
import TokuKitUI

/// Main entry point for the Toku macOS app.
@main
struct TokuApp: App {
    @StateObject private var appState = AppState()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(appState)
                .frame(minWidth: 800, minHeight: 500)
        }
        .commands {
            TokuCommands()
        }
        .defaultSize(width: 1100, height: 700)
        .onChange(of: scenePhase) { _, newPhase in
            if newPhase == .background || newPhase == .inactive {
                appState.performBackgroundSync()
            }
        }

        #if os(macOS)
        Settings {
            SettingsView()
                .environmentObject(appState)
        }
        #endif
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

    /// Push pending local changes when the app moves to the background. Safe to
    /// call when sync is not configured (it becomes a no-op).
    func performBackgroundSync() {
        syncVM?.syncOnBackground()
    }

    /// Default database path in Application Support.
    static func defaultDatabasePath() -> String {
        let appSupport = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first!.appendingPathComponent("dev.toku.app")

        try? FileManager.default.createDirectory(
            at: appSupport,
            withIntermediateDirectories: true
        )

        return appSupport.appendingPathComponent("library.db").path
    }
}

struct SettingsView: View {
    @EnvironmentObject var appState: AppState

    var body: some View {
        Group {
            if let syncVM = appState.syncVM {
                SyncSettingsView(viewModel: syncVM)
            } else {
                Form {
                    Text("Sync is unavailable because the database failed to open.")
                        .foregroundStyle(.secondary)
                        .padding()
                }
            }
        }
        .frame(width: 460, height: 420)
    }
}
