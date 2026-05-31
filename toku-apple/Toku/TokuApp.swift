import SwiftUI
import TokuKit

/// Main entry point for the Toku macOS app.
@main
struct TokuApp: App {
    @StateObject private var appState = AppState()

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

        #if os(macOS)
        Settings {
            SettingsView()
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
    @Published var errorMessage: String?

    init() {
        let dbPath = AppState.defaultDatabasePath()

        do {
            let ffi = try TokuFFI(path: dbPath)
            self.ffi = ffi
            self.libraryVM = LibraryViewModel(ffi: ffi)
            self.statsVM = StatsViewModel(ffi: ffi)
            self.importVM = ImportViewModel(ffi: ffi)
        } catch {
            self.ffi = nil
            self.errorMessage = "Failed to open database: \(error.localizedDescription)"
        }
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
    var body: some View {
        Form {
            Text("Toku settings will appear here.")
                .padding()
        }
        .frame(width: 400, height: 200)
    }
}
