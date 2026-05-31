import SwiftUI
import TokuKit

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

    private var iPhoneLayout: some View {
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
        }
        .listStyle(.sidebar)
        .navigationTitle("Toku")
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

    var id: String { rawValue }
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

            Section("About") {
                LabeledContent("Version", value: "0.2.1")
            }
        }
        .navigationTitle("More")
    }
}
