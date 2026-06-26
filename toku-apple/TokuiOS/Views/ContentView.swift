import SwiftUI
import TokuKit
import TokuKitUI

/// Root view that adapts between iPhone (TabView) and iPad (NavigationSplitView).
struct ContentView: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.horizontalSizeClass) private var sizeClass

    var body: some View {
        if let errorMessage = appState.errorMessage {
            errorView(errorMessage)
        } else if sizeClass == .regular {
            iPadLayout
        } else {
            iPhoneLayout
        }
    }

    // MARK: - iPhone: Tab-based navigation

    @ViewBuilder
    private var iPhoneLayout: some View {
        if let syncVM = appState.syncVM {
            PhoneTabs(syncVM: syncVM)
                .task { appState.performLaunchSync() }
        } else {
            // Defensive: the DB normally opens (otherwise errorView is shown).
            PhoneTabs(syncVM: nil)
                .task { appState.performLaunchSync() }
        }
    }

    // MARK: - iPad: Sidebar navigation

    @State private var iPadSelection: SidebarItem? = .library
    @State private var selectedBookID: String?

    private var iPadLayout: some View {
        NavigationSplitView {
            iPadSidebar
        } content: {
            switch iPadSelection {
            case .library:
                if let vm = appState.libraryVM {
                    LibraryGridView(viewModel: vm, ffi: appState.ffi)
                }
            case .search:
                if let vm = appState.libraryVM {
                    SearchView(viewModel: vm, ffi: appState.ffi)
                }
            case .stats:
                if let vm = appState.statsVM {
                    StatsGlanceView(viewModel: vm)
                }
            case .importBooks:
                if let vm = appState.importVM {
                    ImportView(viewModel: vm, onImportComplete: {
                        appState.libraryVM?.loadBooks()
                    })
                }
            case .sync:
                if let vm = appState.syncVM {
                    SyncSettingsView(viewModel: vm)
                }
            case .none:
                Text("Select an item from the sidebar")
                    .foregroundStyle(.secondary)
            }
        } detail: {
            if let bookID = selectedBookID, let ffi = appState.ffi {
                BookDetailView(bookID: bookID, ffi: ffi)
            } else {
                Text("Select a book to see details")
                    .foregroundStyle(.secondary)
            }
        }
        .task {
            appState.performLaunchSync()
        }
    }

    private var iPadSidebar: some View {
        List(selection: $iPadSelection) {
            Section("Library") {
                Label("All Books", systemImage: "books.vertical")
                    .tag(SidebarItem.library)
                Label("Search", systemImage: "magnifyingglass")
                    .tag(SidebarItem.search)
            }

            Section("Insights") {
                Label("Statistics", systemImage: "chart.bar")
                    .tag(SidebarItem.stats)
            }

            Section("Manage") {
                Label("Import", systemImage: "square.and.arrow.down")
                    .tag(SidebarItem.importBooks)
            }

            Section("Settings") {
                Label("Sync", systemImage: "arrow.triangle.2.circlepath")
                    .tag(SidebarItem.sync)
            }
        }
        .listStyle(.sidebar)
        .navigationTitle("Toku")
        .safeAreaInset(edge: .bottom) {
            if let syncVM = appState.syncVM {
                HStack {
                    SyncStatusBadge(viewModel: syncVM)
                    Spacer()
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
            }
        }
    }

    // MARK: - Error state

    @ViewBuilder
    private func errorView(_ message: String) -> some View {
        VStack(spacing: 16) {
            Image(systemName: "exclamationmark.triangle")
                .font(.largeTitle)
                .foregroundStyle(.red)
            Text("Unable to start Toku")
                .font(.headline)
            Text(message)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding()
    }
}

/// Sidebar navigation items for iPad.
enum SidebarItem: String, Identifiable, CaseIterable {
    case library = "Library"
    case search = "Search"
    case stats = "Statistics"
    case importBooks = "Import"
    case sync = "Sync"

    var id: String { rawValue }
}

/// iPhone tab layout, dispatching to an observing wrapper when sync is available
/// so the More tab can show a live conflict badge.
private struct PhoneTabs: View {
    let syncVM: SyncViewModel?

    var body: some View {
        if let syncVM {
            ObservedPhoneTabs(syncVM: syncVM)
        } else {
            PhoneTabsContent(conflictCount: 0)
        }
    }
}

/// Observes the sync view model so the More tab badge updates live.
private struct ObservedPhoneTabs: View {
    @ObservedObject var syncVM: SyncViewModel

    var body: some View {
        PhoneTabsContent(conflictCount: syncVM.conflictCount)
            .onAppear { syncVM.refresh() }
    }
}

/// The actual iPhone `TabView`. `conflictCount` drives the badge on the More tab.
private struct PhoneTabsContent: View {
    @EnvironmentObject var appState: AppState
    let conflictCount: Int

    var body: some View {
        TabView {
            Tab("Library", systemImage: "books.vertical") {
                NavigationStack {
                    if let vm = appState.libraryVM {
                        LibraryGridView(viewModel: vm, ffi: appState.ffi)
                    }
                }
            }

            Tab("Search", systemImage: "magnifyingglass") {
                NavigationStack {
                    if let vm = appState.libraryVM {
                        SearchView(viewModel: vm, ffi: appState.ffi)
                    }
                }
            }

            Tab("Stats", systemImage: "chart.bar") {
                NavigationStack {
                    if let vm = appState.statsVM {
                        StatsGlanceView(viewModel: vm)
                    }
                }
            }

            Tab("More", systemImage: "ellipsis") {
                NavigationStack {
                    MoreView()
                }
            }
            .badge(conflictCount)
        }
    }
}

/// Placeholder "More" screen for iPhone (settings, import, about).
struct MoreView: View {
    @EnvironmentObject var appState: AppState

    var body: some View {
        List {
            if let vm = appState.importVM {
                NavigationLink {
                    ImportView(viewModel: vm, onImportComplete: {
                        appState.libraryVM?.loadBooks()
                    })
                } label: {
                    Label("Import Goodreads CSV", systemImage: "square.and.arrow.down")
                }
            }

            NavigationLink {
                BarcodeScannerView(ffi: appState.ffi)
            } label: {
                Label("Scan ISBN Barcode", systemImage: "barcode.viewfinder")
            }

            if let vm = appState.syncVM {
                Section("Settings") {
                    NavigationLink {
                        SyncSettingsView(viewModel: vm)
                    } label: {
                        SyncMoreRow(viewModel: vm)
                    }
                }
            }

            Section("About") {
                LabeledContent("Version", value: "0.2.1")
            }
        }
        .navigationTitle("More")
    }
}

/// "Sync" row in the More list that shows a live conflict count.
private struct SyncMoreRow: View {
    @ObservedObject var viewModel: SyncViewModel

    var body: some View {
        HStack {
            Label("Sync", systemImage: "arrow.triangle.2.circlepath")
            Spacer()
            if viewModel.hasConflicts {
                Text("\(viewModel.conflictCount)")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.white)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(.orange))
            }
        }
        .onAppear { viewModel.refresh() }
    }
}
